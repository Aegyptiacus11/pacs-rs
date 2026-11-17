//! The documentation's examples, run against a server.
//!
//! A page of docs/ shows requests as `curl` commands in a `console` block, and an answer as
//! the `json` block right after it:
//!
//! ````text
//! ```console
//! $ curl http://127.0.0.1:8080/studies
//! ```
//! ```json
//! ["27f7126f-4f66fb14-03f4081b-f9341db2-53925988"]
//! ```
//! ````
//!
//! Each page in `PAGES` gets an empty server, and its commands are sent to it in order. A
//! command must be answered with a 2xx status, or with the `HttpStatus` its `json` block
//! shows; the answer must equal the block, where `"..."` stands for any value (a time, say).
//! `@name` reads docs/sample/name, the study the pages tell the reader to download. A page
//! with a `$ curl` line that is not in `PAGES` fails the test, so no example goes unchecked.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use clap::Parser;
use http_body_util::BodyExt;
use pacs_server::{Settings, app, open_archive};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

/// The pages whose examples are run.
const PAGES: &[&str] = &["dicom-files.md", "quickstart.md", "rest-guide.md"];
/// Where the pages say the server is: `compose.yaml`'s port.
const SERVER: &str = "http://127.0.0.1:8080";

fn docs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

/// The study in docs/sample: three slices of LiTS volume 0 (docs/sample-study.md), each
/// file with its SHA-256.
const SAMPLE: &[(&str, &str)] = &[
    (
        "00001.dcm",
        "e921d6a590d3e6108d91e298fde5556946b93e14b55824240cb0b607104d9a3f",
    ),
    (
        "00002.dcm",
        "bc1d93aff976c029e214c1d3ea41a2b345e0d6d54bdf5ce0d40974cc68194705",
    ),
    (
        "00003.dcm",
        "34acf9eb124147ed7fa6ae9624c2b1e943a4a1f17fd1b2bba1977f015593899f",
    ),
];
const SAMPLE_PATIENT: &str = "LITS-0";
const SAMPLE_STUDY: &str = "2.25.3100000";

#[derive(Debug, PartialEq)]
enum Sent {
    Nothing,
    Text(String),
    /// A file of docs/sample.
    File(String),
}

#[derive(Debug, PartialEq)]
struct Call {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Sent,
    /// `-o`: the answer goes to a file, so the page shows none.
    saved: bool,
}

#[derive(Debug, PartialEq)]
struct Example {
    line: usize,
    call: Call,
    expected: Option<Value>,
}

/// A command line as a shell splits it: spaces, single and double quotes, no expansion.
fn words(command: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    for c in command.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            None => {
                word.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(format!("a quote is not closed: {command}"));
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

/// The request a `curl` command makes. Only the options the pages use are known; another
/// one is an error, so that a command is never run as something else than what it says.
fn parse_curl(command: &str) -> Result<Call, String> {
    let words = words(command)?;
    let mut words = words.iter().map(String::as_str);
    if words.next() != Some("curl") {
        return Err(format!("not a curl command: {command}"));
    }
    let mut method = None;
    let mut url = None;
    let mut headers = Vec::new();
    let mut body = Sent::Nothing;
    let mut saved = false;
    while let Some(word) = words.next() {
        let mut value = || {
            words
                .next()
                .ok_or_else(|| format!("{word} needs a value: {command}"))
        };
        match word {
            "-s" => {}
            "-X" => method = Some(value()?.to_owned()),
            "-o" => {
                value()?;
                saved = true;
            }
            "-H" => {
                let header = value()?;
                let (name, text) = header
                    .split_once(':')
                    .ok_or_else(|| format!("not a header: {header}"))?;
                headers.push((name.trim().to_owned(), text.trim().to_owned()));
            }
            "-d" | "--data-binary" => {
                let data = value()?;
                body = match data.strip_prefix('@') {
                    Some(file) => Sent::File(file.to_owned()),
                    None => Sent::Text(data.to_owned()),
                };
            }
            option if option.starts_with('-') => {
                return Err(format!("unknown curl option {option}: {command}"));
            }
            _ if url.is_some() => return Err(format!("two URLs: {command}")),
            _ => url = Some(word),
        }
    }
    let url = url.ok_or_else(|| format!("no URL: {command}"))?;
    let path = url
        .strip_prefix(SERVER)
        .filter(|path| path.starts_with('/'))
        .ok_or_else(|| format!("the examples are for {SERVER}/..., not {url}"))?;
    let method = method.unwrap_or_else(|| match body {
        Sent::Nothing => "GET".to_owned(),
        _ => "POST".to_owned(),
    });
    Ok(Call {
        method,
        path: path.to_owned(),
        headers,
        body,
        saved,
    })
}

/// The `curl` commands of a page, each with the `json` block that follows its `console`
/// block, if one does. A `console` block without `curl` (docker, say) is not an example.
fn examples(markdown: &str) -> Result<Vec<Example>, String> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut examples: Vec<Example> = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        if lines[at].trim() != "```console" {
            at += 1;
            continue;
        }
        let opened = at + 1;
        let close = (opened..lines.len())
            .find(|&i| lines[i].trim() == "```")
            .ok_or_else(|| format!("line {opened}: the console block is not closed"))?;
        let mut commands: Vec<(usize, String)> = Vec::new();
        let mut continued = false;
        for (i, line) in lines.iter().enumerate().take(close).skip(opened) {
            if continued {
                let (_, command) = commands.last_mut().expect("a continued line has a command");
                continued = line.trim_end().ends_with('\\');
                command.push(' ');
                command.push_str(line.trim().trim_end_matches('\\').trim_end());
            } else if let Some(command) = line.strip_prefix("$ ") {
                continued = command.trim_end().ends_with('\\');
                let command = command.trim_end().trim_end_matches('\\').trim_end();
                commands.push((i + 1, command.to_owned()));
            } else {
                return Err(format!(
                    "line {}: a console block holds commands only, each after `$ `; an answer goes in the json block after it",
                    i + 1
                ));
            }
        }
        at = close + 1;
        if !commands.iter().any(|(_, c)| c.starts_with("curl ")) {
            continue;
        }
        let first = examples.len();
        for (line, command) in commands {
            let call = parse_curl(&command).map_err(|e| format!("line {line}: {e}"))?;
            examples.push(Example {
                line,
                call,
                expected: None,
            });
        }
        let mut next = at;
        while next < lines.len() && lines[next].trim().is_empty() {
            next += 1;
        }
        if next < lines.len() && lines[next].trim() == "```json" {
            let close = (next + 1..lines.len())
                .find(|&i| lines[i].trim() == "```")
                .ok_or_else(|| format!("line {}: the json block is not closed", next + 1))?;
            let text = lines[next + 1..close].join("\n");
            let expected = serde_json::from_str(&text)
                .map_err(|e| format!("line {}: the answer is not JSON: {e}", next + 1))?;
            let last = examples.last_mut().expect("the block had a command");
            if last.call.saved {
                return Err(format!(
                    "line {}: the command saves its answer with -o, so the page cannot show it",
                    last.line
                ));
            }
            last.expected = Some(expected);
            at = close + 1;
        }
        debug_assert!(examples.len() > first);
    }
    Ok(examples)
}

/// Where `actual` differs from `expected`, if it does. `"..."` in `expected` is any value.
fn difference(expected: &Value, actual: &Value, at: &str) -> Option<String> {
    match (expected, actual) {
        (Value::String(any), _) if any == "..." => None,
        (Value::Object(e), Value::Object(a)) => {
            for (key, value) in e {
                let Some(found) = a.get(key) else {
                    return Some(format!("{at}.{key}: missing from the answer"));
                };
                if let Some(d) = difference(value, found, &format!("{at}.{key}")) {
                    return Some(d);
                }
            }
            a.keys()
                .find(|key| !e.contains_key(*key))
                .map(|key| format!("{at}.{key}: in the answer, not on the page"))
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                return Some(format!(
                    "{at}: {} items on the page, {} in the answer",
                    e.len(),
                    a.len()
                ));
            }
            e.iter()
                .zip(a)
                .enumerate()
                .find_map(|(i, (e, a))| difference(e, a, &format!("{at}[{i}]")))
        }
        _ if expected == actual => None,
        _ => Some(format!(
            "{at}: {expected} on the page, {actual} in the answer"
        )),
    }
}

async fn server(dir: &Path) -> Router {
    let settings = Settings::try_parse_from([
        "pacs-server".to_owned(),
        "--index".into(),
        dir.join("index.sqlite").display().to_string(),
        "--storage".into(),
        format!("file:{}", dir.join("files").display()),
    ])
    .unwrap();
    app(open_archive(&settings).await.unwrap(), &settings).unwrap()
}

/// Sends a page's examples to an empty server, in order; the first one that does not hold.
async fn run(markdown: &str) -> Result<usize, String> {
    let examples = examples(markdown)?;
    let dir = tempfile::tempdir().unwrap();
    let app = server(dir.path()).await;
    for example in &examples {
        let Call {
            method,
            path,
            headers,
            body,
            saved,
        } = &example.call;
        let said = format!("line {}: {method} {path}", example.line);
        let body = match body {
            Sent::Nothing => Body::empty(),
            Sent::Text(text) => Body::from(text.clone()),
            Sent::File(name) => {
                if name.contains(['/', '\\']) {
                    return Err(format!("{said}: @{name} is not a file of docs/sample"));
                }
                let bytes = std::fs::read(docs().join("sample").join(name))
                    .map_err(|e| format!("{said}: docs/sample/{name}: {e}"))?;
                Body::from(bytes)
            }
        };
        let mut request = Request::builder().method(method.as_str()).uri(path);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let request = request.body(body).map_err(|e| format!("{said}: {e}"))?;
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let shown = example
            .expected
            .as_ref()
            .and_then(|e| e.get("HttpStatus"))
            .and_then(Value::as_u64);
        if !status.is_success() && shown != Some(u64::from(status.as_u16())) {
            return Err(format!(
                "{said}: answered {status}: {}",
                String::from_utf8_lossy(&bytes)
            ));
        }
        if *saved && bytes.is_empty() {
            return Err(format!("{said}: nothing to save"));
        }
        if let Some(expected) = &example.expected {
            let actual: Value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("{said}: the answer is not JSON: {e}"))?;
            if let Some(d) = difference(expected, &actual, "answer") {
                return Err(format!(
                    "{said}: {d}\nthe answer:\n{}",
                    serde_json::to_string_pretty(&actual).unwrap()
                ));
            }
        }
    }
    Ok(examples.len())
}

#[tokio::test]
async fn the_pages_examples_hold() {
    for page in PAGES {
        let markdown = std::fs::read_to_string(docs().join(page))
            .unwrap_or_else(|e| panic!("docs/{page}: {e}"));
        match run(&markdown).await {
            Ok(0) => panic!("docs/{page}: in PAGES, but it has no example"),
            Ok(_) => {}
            Err(e) => panic!("docs/{page}: {e}"),
        }
    }
}

#[test]
fn no_page_with_examples_is_left_out() {
    let mut seen = 0;
    for entry in std::fs::read_dir(docs()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        seen += 1;
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let markdown = std::fs::read_to_string(&path).unwrap();
        let found = examples(&markdown).unwrap_or_else(|e| panic!("docs/{name}: {e}"));
        assert!(
            found.is_empty() || PAGES.contains(&name.as_str()),
            "docs/{name} has curl examples: add it to PAGES so they are run"
        );
    }
    assert!(seen > PAGES.len(), "docs/ was not read");
}

#[test]
fn the_sample_study_is_the_one_the_pages_describe() {
    for (name, digest) in SAMPLE {
        let file = std::fs::read(docs().join("sample").join(name))
            .unwrap_or_else(|e| panic!("docs/sample/{name}: {e}"));
        let found: String = Sha256::digest(&file)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            &found, digest,
            "docs/sample/{name} is not the file docs/sample-study.md describes"
        );
    }
    let files = std::fs::read_dir(docs().join("sample"))
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            name.to_string_lossy().ends_with(".dcm")
        })
        .count();
    assert_eq!(files, SAMPLE.len(), "docs/sample has other DICOM files");
}

#[test]
fn curl_commands_as_requests() {
    assert_eq!(
        parse_curl("curl http://127.0.0.1:8080/studies?expand").unwrap(),
        Call {
            method: "GET".into(),
            path: "/studies?expand".into(),
            headers: vec![],
            body: Sent::Nothing,
            saved: false,
        }
    );
    let upload = parse_curl("curl --data-binary @00001.dcm http://127.0.0.1:8080/instances");
    let upload = upload.unwrap();
    assert_eq!(upload.method, "POST");
    assert_eq!(upload.body, Sent::File("00001.dcm".into()));
    let find = parse_curl(
        r#"curl -s -H 'Accept: application/json' -d '{"Level": "Study"}' http://127.0.0.1:8080/tools/find"#,
    )
    .unwrap();
    assert_eq!(find.headers, [("Accept".into(), "application/json".into())]);
    assert_eq!(find.body, Sent::Text(r#"{"Level": "Study"}"#.into()));
    let delete = parse_curl("curl -X DELETE -o gone.json http://127.0.0.1:8080/studies/x").unwrap();
    assert_eq!((delete.method.as_str(), delete.saved), ("DELETE", true));
}

#[test]
fn a_command_that_cannot_be_run_as_written_is_refused() {
    for (command, why) in [
        (
            "curl -k http://127.0.0.1:8080/studies",
            "unknown curl option -k",
        ),
        ("curl http://localhost:8042/studies", "the examples are for"),
        ("curl http://127.0.0.1:8080", "the examples are for"),
        ("curl -X", "-X needs a value"),
        ("curl -s", "no URL"),
        (
            "curl http://127.0.0.1:8080/a http://127.0.0.1:8080/b",
            "two URLs",
        ),
        (
            "curl -d '{ http://127.0.0.1:8080/a",
            "a quote is not closed",
        ),
        ("curl -H Accept http://127.0.0.1:8080/a", "not a header"),
        ("wget http://127.0.0.1:8080/a", "not a curl command"),
    ] {
        let error = parse_curl(command).unwrap_err();
        assert!(error.contains(why), "{command}: {error}");
    }
}

#[test]
fn examples_of_a_page() {
    let page = "\
text
```console
$ docker compose up -d
```
```console
$ curl -d '{\"Level\": \"Study\"}' \\
    http://127.0.0.1:8080/tools/find
```

```json
[\"a\"]
```
```console
$ curl http://127.0.0.1:8080/studies
```
text
```json
{\"not\": \"an answer: text came between\"}
```
";
    let found = examples(page).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(
        (found[0].line, found[0].call.path.as_str()),
        (6, "/tools/find")
    );
    assert_eq!(
        found[0].call.body,
        Sent::Text(r#"{"Level": "Study"}"#.into())
    );
    assert_eq!(found[0].expected, Some(serde_json::json!(["a"])));
    assert_eq!(found[1].expected, None);

    for (page, why) in [
        ("```console\n$ curl http://127.0.0.1:8080/a\n", "not closed"),
        (
            "```console\n$ curl http://127.0.0.1:8080/a\n[]\n```\n",
            "commands only",
        ),
        (
            "```console\n$ curl -k http://127.0.0.1:8080/a\n```\n",
            "line 2: unknown curl option",
        ),
        (
            "```console\n$ curl http://127.0.0.1:8080/a\n```\n```json\n[\n```\n",
            "not JSON",
        ),
        (
            "```console\n$ curl -o a http://127.0.0.1:8080/a\n```\n```json\n[]\n```\n",
            "saves its answer",
        ),
    ] {
        let error = examples(page).unwrap_err();
        assert!(error.contains(why), "{page}: {error}");
    }
}

#[test]
fn answers_are_compared_whole_but_for_the_dots() {
    use serde_json::json;
    let same = |e: Value, a: Value| difference(&e, &a, "answer");
    assert_eq!(
        same(
            json!({"a": [1, "..."], "t": "..."}),
            json!({"t": 7, "a": [1, {}]})
        ),
        None
    );
    for (expected, actual, why) in [
        (
            json!({"a": 1}),
            json!({"a": 2}),
            "answer.a: 1 on the page, 2 in the answer",
        ),
        (
            json!({"a": 1}),
            json!({}),
            "answer.a: missing from the answer",
        ),
        (
            json!({}),
            json!({"b": 1}),
            "answer.b: in the answer, not on the page",
        ),
        (
            json!([1]),
            json!([1, 2]),
            "answer: 1 items on the page, 2 in the answer",
        ),
        (json!([{"a": "x"}]), json!([{"a": "y"}]), "answer[0].a"),
        (json!("..."), json!(null), ""),
    ] {
        match same(expected.clone(), actual) {
            Some(found) => assert!(!why.is_empty() && found.contains(why), "{found}"),
            None => assert!(why.is_empty(), "{expected}"),
        }
    }
}

#[tokio::test]
async fn an_example_that_does_not_hold_fails() {
    let upload =
        "```console\n$ curl --data-binary @00001.dcm http://127.0.0.1:8080/instances\n```\n";
    assert_eq!(run(upload).await, Ok(1));

    // the page shows another answer than the server gives
    let wrong = format!(
        "{upload}```console\n$ curl http://127.0.0.1:8080/studies\n```\n```json\n[]\n```\n"
    );
    let error = run(&wrong).await.unwrap_err();
    assert!(
        error.contains("line 5: GET /studies: answer: 0 items on the page, 1 in the answer"),
        "{error}"
    );
    assert!(
        error.contains(&pacs_core::public_ids::study(SAMPLE_PATIENT, SAMPLE_STUDY)),
        "{error}"
    );

    // a failure the page does not show
    let missing = "```console\n$ curl http://127.0.0.1:8080/studies/none\n```\n";
    let error = run(missing).await.unwrap_err();
    assert!(error.contains("answered 404"), "{error}");
    // ... and one it shows
    let shown = "```console\n$ curl -d junk http://127.0.0.1:8080/instances\n```\n```json\n\
        {\"HttpError\": \"Bad Request\", \"HttpStatus\": 400, \"Message\": \"Bad file format\", \"Details\": \"...\"}\n```\n";
    assert_eq!(run(shown).await, Ok(1));
    // ... but not another status than the one shown
    let other = "```console\n$ curl http://127.0.0.1:8080/studies/none\n```\n```json\n{\"HttpStatus\": 400}\n```\n";
    assert!(run(other).await.unwrap_err().contains("answered 404"));

    // a file that is not the sample's
    for (name, why) in [
        ("../book.toml", "not a file of docs/sample"),
        ("none.dcm", "docs/sample/none.dcm"),
    ] {
        let page = format!(
            "```console\n$ curl --data-binary @{name} http://127.0.0.1:8080/instances\n```\n"
        );
        let error = run(&page).await.unwrap_err();
        assert!(error.contains(why), "{error}");
    }
    // an answer saved with -o must hold something
    let saved = format!(
        "{upload}```console\n$ curl -o study.zip http://127.0.0.1:8080/studies/none/archive\n```\n"
    );
    assert!(run(&saved).await.unwrap_err().contains("answered 404"));
}
