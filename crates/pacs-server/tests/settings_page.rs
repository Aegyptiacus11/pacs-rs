//! docs/settings.md against the program's settings: every setting on the page, every
//! setting of the page in the program, and the defaults the same.

use std::collections::BTreeMap;
use std::path::Path;

use clap::CommandFactory;
use pacs_server::Settings;

/// Read from the environment only, never as a flag: secrets, and the log filter. Each must
/// be named in the program's source.
const ENVIRONMENT_ONLY: &[&str] = &[
    "PACS_INDEX_PASSWORD",
    "PACS_S3_ACCESS_KEY_ID",
    "PACS_S3_SECRET_ACCESS_KEY",
    "PACS_AUTH_SECRET",
    "PACS_ADMIN_PASSWORD",
    "PACS_PROJECT_SECRET_<AE title>",
    "RUST_LOG",
];

/// Defaults the page writes for a reader, and what the program holds for them.
const WRITTEN_AS: &[(&str, &str)] = &[("1 GiB", "1073741824"), ("512 MiB", "536870912")];

/// The program's settings: each variable with its default, if it has one.
fn program() -> BTreeMap<String, Option<String>> {
    Settings::command()
        .get_arguments()
        .filter_map(|argument| {
            let variable = argument.get_env()?.to_str()?.to_owned();
            let default: Vec<String> = argument
                .get_default_values()
                .iter()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            Some((variable, (!default.is_empty()).then(|| default.join(","))))
        })
        .collect()
}

/// The settings of a page: each variable of a table row with the row's default cell.
fn page(markdown: &str) -> BTreeMap<String, String> {
    let mut settings = BTreeMap::new();
    for line in markdown.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // | `VARIABLE`[, `OTHER`] | default | what |
        if cells.len() < 5 || !cells[1].starts_with('`') {
            continue;
        }
        for variable in cells[1].split(',') {
            let variable = variable.trim().trim_matches('`');
            if variable.starts_with("PACS_") || variable == "RUST_LOG" {
                settings.insert(variable.to_owned(), cells[2].to_owned());
            }
        }
    }
    settings
}

/// What differs between a page and the program, one line each.
fn differences(markdown: &str, program: &BTreeMap<String, Option<String>>) -> Vec<String> {
    let page = page(markdown);
    let mut problems = Vec::new();
    for (variable, default) in program {
        let Some(written) = page.get(variable) else {
            problems.push(format!(
                "{variable}: a setting of the program, not on the page"
            ));
            continue;
        };
        let shown = written.strip_prefix('`').and_then(|w| w.strip_suffix('`'));
        let shown = shown.or_else(|| {
            WRITTEN_AS
                .iter()
                .find(|(as_written, _)| as_written == written)
                .map(|(_, held)| *held)
        });
        match (default, shown) {
            (Some(default), Some(shown)) if default == shown => {}
            (Some(default), _) => problems.push(format!(
                "{variable}: the default is {default}, and the page says {written:?}"
            )),
            (None, Some(shown)) => problems.push(format!(
                "{variable}: no default in the program, and the page says {shown}"
            )),
            (None, None) => {}
        }
    }
    for variable in page.keys() {
        if !program.contains_key(variable) && !ENVIRONMENT_ONLY.contains(&variable.as_str()) {
            problems.push(format!(
                "{variable}: on the page, not a setting of the program"
            ));
        }
    }
    problems
}

fn read(path: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()))
}

#[test]
fn the_page_has_every_setting_with_its_default() {
    let problems = differences(&read("../../docs/settings.md"), &program());
    assert!(
        problems.is_empty(),
        "docs/settings.md:\n{}",
        problems.join("\n")
    );
    assert!(
        program().len() > 40,
        "{} settings read from the program",
        program().len()
    );
}

#[test]
fn what_is_read_from_the_environment_only_is_read_somewhere() {
    let source: String = [
        "src/lib.rs",
        "src/auth.rs",
        "src/projects.rs",
        "src/main.rs",
    ]
    .iter()
    .map(|file| read(file))
    .collect();
    let page = page(&read("../../docs/settings.md"));
    for variable in ENVIRONMENT_ONLY {
        // a project's secret is named by its AE title; the log filter is read by the
        // logging library, under the name it gives it
        let named = match *variable {
            "RUST_LOG" => "try_from_default_env",
            variable => variable.split('<').next().unwrap(),
        };
        assert!(
            source.contains(named),
            "{variable} is not read by the program"
        );
        assert!(
            page.contains_key(*variable),
            "{variable} is not on the page"
        );
        assert!(
            !program().contains_key(*variable),
            "{variable} is a flag too"
        );
    }
}

#[test]
fn a_page_that_differs_is_told_how() {
    let program: BTreeMap<String, Option<String>> = [
        ("PACS_LISTEN".to_owned(), Some("127.0.0.1:8080".to_owned())),
        (
            "PACS_MAX_UPLOAD_BYTES".to_owned(),
            Some("1073741824".to_owned()),
        ),
        ("PACS_AUDIT_FILE".to_owned(), None),
    ]
    .into();
    let good = "\
| Variable | Default | |
|---|---|---|
| `PACS_LISTEN` | `127.0.0.1:8080` | address |
| `PACS_MAX_UPLOAD_BYTES` | 1 GiB | largest body |
| `PACS_AUDIT_FILE` | none | a file |
| `PACS_AUTH_SECRET` | | a secret, from the environment only |
";
    assert_eq!(differences(good, &program), Vec::<String>::new());

    for (from, to, why) in [
        (
            "`127.0.0.1:8080`",
            "`0.0.0.0:8080`",
            "PACS_LISTEN: the default is 127.0.0.1:8080",
        ),
        (
            "`127.0.0.1:8080`",
            "the usual one",
            "PACS_LISTEN: the default is 127.0.0.1:8080",
        ),
        (
            "1 GiB",
            "512 MiB",
            "PACS_MAX_UPLOAD_BYTES: the default is 1073741824",
        ),
        (
            "| none |",
            "| `audit.log` |",
            "PACS_AUDIT_FILE: no default in the program",
        ),
        (
            "| `PACS_AUDIT_FILE` | none | a file |\n",
            "",
            "PACS_AUDIT_FILE: a setting of the program, not on the page",
        ),
        (
            "`PACS_AUTH_SECRET`",
            "`PACS_AUTH_SECRETS`",
            "PACS_AUTH_SECRETS: on the page, not a setting of the program",
        ),
    ] {
        assert!(good.contains(from), "{from}");
        let problems = differences(&good.replace(from, to), &program);
        assert!(
            problems.iter().any(|p| p.starts_with(why)),
            "{why}: {problems:?}"
        );
    }
}
