//! The limits on requests (`pacs_server::limits`), against clients that misbehave over
//! real connections: one that never finishes its request, one that stops reading, one
//! that asks for too much at once. A small application stands in for the archive, so
//! that a request can be made as slow as a test needs; the last test is the real one.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::Query;
use axum::routing::{get, post};
use pacs_server::limits::{Limits, serve};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn ms(n: u64) -> Option<Duration> {
    Some(Duration::from_millis(n))
}

/// Limits a test does not care about: none of them.
fn unlimited() -> Limits {
    Limits {
        header_timeout: None,
        body_timeout: None,
        request_timeout: None,
        write_timeout: None,
        max_connections: None,
        max_requests: None,
        max_requests_per_client: None,
    }
}

#[derive(serde::Deserialize)]
struct Slow {
    ms: u64,
}

async fn slow(Query(slow): Query<Slow>) -> &'static str {
    tokio::time::sleep(Duration::from_millis(slow.ms)).await;
    "done\n"
}

/// 256 MiB in 64 KiB pieces: more than any socket buffer holds.
async fn huge() -> Body {
    let piece = Bytes::from(vec![b'x'; 64 * 1024]);
    let pieces = (0..4096).map(move |_| Ok::<_, std::io::Error>(piece.clone()));
    Body::from_stream(futures::stream::iter(pieces))
}

/// The stand-in application behind `limits`, on a free port.
async fn start(limits: Limits) -> SocketAddr {
    let app = Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .route("/slow", get(slow).post(slow))
        .route(
            "/echo",
            post(|body: Bytes| async move { format!("{}\n", body.len()) }),
        )
        .route("/huge", get(huge));
    let app = limits.layer(app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, app, limits, std::future::pending()));
    address
}

/// What the server sent until it closed the connection, or until `within` ran out
/// (`None`).
async fn read_all(stream: &mut TcpStream, within: Duration) -> Option<String> {
    let mut all = Vec::new();
    match tokio::time::timeout(within, stream.read_to_end(&mut all)).await {
        // a reset is a closed connection too
        Ok(_) => Some(String::from_utf8_lossy(&all).into_owned()),
        Err(_) => None,
    }
}

/// One request on a connection of its own, closed after the answer: the status line's
/// code, and the whole answer.
async fn request(address: SocketAddr, head: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!("{head} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let answer = read_all(&mut stream, Duration::from_secs(10))
        .await
        .expect("an answer within 10 s");
    let status = answer
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status in {answer:?}"));
    (status, answer)
}

async fn status(address: SocketAddr, head: &str) -> u16 {
    request(address, head).await.0
}

/// Ask until the answer is `wanted`, for at most `within`: for a place that comes free
/// when the server notices a client is gone.
async fn eventually(address: SocketAddr, head: &str, wanted: u16, within: Duration) -> Duration {
    let started = Instant::now();
    loop {
        if status(address, head).await == wanted {
            return started.elapsed();
        }
        assert!(
            started.elapsed() < within,
            "{head}: never {wanted} within {within:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_request_head_that_does_not_arrive_closes_the_connection() {
    let address = start(Limits {
        header_timeout: ms(300),
        ..unlimited()
    })
    .await;
    // half a head, and nothing at all: both are closed once the time is up, not before
    for sent in [&b"GET /healthz HTTP/1.1\r\nHost: te"[..], b""] {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(sent).await.unwrap();
        let started = Instant::now();
        let answer = read_all(&mut stream, Duration::from_secs(5)).await;
        let took = started.elapsed();
        assert!(answer.is_some(), "still open after 5 s: {sent:?}");
        assert!(
            took >= Duration::from_millis(250),
            "closed after {took:?}: {sent:?}"
        );
        assert!(!answer.unwrap().contains("200 OK"));
    }
    // a whole request in time is served
    assert_eq!(status(address, "GET /healthz").await, 200);

    // without the limit, the half head is waited for
    let patient = start(unlimited()).await;
    let mut stream = TcpStream::connect(patient).await.unwrap();
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: te")
        .await
        .unwrap();
    assert_eq!(
        read_all(&mut stream, Duration::from_millis(600)).await,
        None
    );
    stream
        .write_all(b"st\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    assert!(
        read_all(&mut stream, Duration::from_secs(5))
            .await
            .unwrap()
            .contains("200 OK")
    );
}

#[tokio::test]
async fn a_body_that_pauses_too_long_fails_the_request_and_frees_its_place() {
    let address = start(Limits {
        body_timeout: ms(300),
        max_requests: Some(1),
        ..unlimited()
    })
    .await;
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 100\r\n\r\n0123456789")
        .await
        .unwrap();
    // while the upload is open it holds the only place
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(status(address, "GET /slow?ms=0").await, 503);
    // the pause runs out: the request fails, and is not answered as if it had worked
    let answer = read_all(&mut stream, Duration::from_secs(5))
        .await
        .expect("closed");
    assert!(!answer.contains("200 OK"), "{answer}");
    assert_eq!(status(address, "GET /slow?ms=0").await, 200);

    // a body that keeps coming, slowly, is not a pause: 5 pieces 100 ms apart
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            b"POST /echo HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 50\r\n\r\n",
        )
        .await
        .unwrap();
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stream.write_all(b"0123456789").await.unwrap();
    }
    let answer = read_all(&mut stream, Duration::from_secs(5)).await.unwrap();
    assert!(
        answer.starts_with("HTTP/1.1 200 OK") && answer.ends_with("50\n"),
        "{answer}"
    );
}

#[tokio::test]
async fn a_request_without_a_body_is_answered_in_time_or_gets_503() {
    let address = start(Limits {
        request_timeout: ms(300),
        max_requests: Some(1),
        ..unlimited()
    })
    .await;
    let started = Instant::now();
    let (status_code, answer) = request(address, "GET /slow?ms=5000").await;
    assert_eq!(status_code, 503, "{answer}");
    assert!(answer.contains("took too long"), "{answer}");
    assert!(started.elapsed() < Duration::from_secs(3));
    // the abandoned request gave its place back
    assert_eq!(status(address, "GET /slow?ms=0").await, 200);
    // in time is in time
    assert_eq!(status(address, "GET /slow?ms=100").await, 200);
    // a request with a body is bounded by its body's pauses, not by this
    assert_eq!(status(address, "POST /slow?ms=600").await, 200);
}

#[tokio::test]
async fn requests_at_once_are_bounded_and_health_is_not() {
    let address = start(Limits {
        max_requests: Some(2),
        ..unlimited()
    })
    .await;
    let held: Vec<_> = (0..2)
        .map(|_| tokio::spawn(status(address, "GET /slow?ms=600")))
        .collect();
    tokio::time::sleep(Duration::from_millis(150)).await;
    // full: one more is told to come back, and the health check still answers
    let (status_code, answer) = request(address, "GET /slow?ms=0").await;
    assert_eq!(status_code, 503, "{answer}");
    assert!(answer.to_lowercase().contains("retry-after: 1"), "{answer}");
    assert_eq!(status(address, "GET /healthz").await, 200);
    // the two that were being served are served
    for one in held {
        assert_eq!(one.await.unwrap(), 200);
    }
    assert_eq!(status(address, "GET /slow?ms=0").await, 200);
}

#[tokio::test]
async fn one_address_cannot_take_every_place() {
    let address = start(Limits {
        max_requests: Some(10),
        max_requests_per_client: Some(2),
        ..unlimited()
    })
    .await;
    let held: Vec<_> = (0..2)
        .map(|_| tokio::spawn(status(address, "GET /slow?ms=600")))
        .collect();
    tokio::time::sleep(Duration::from_millis(150)).await;
    // this address has its two: 429, though the server has eight places left
    let (status_code, answer) = request(address, "GET /slow?ms=0").await;
    assert_eq!(status_code, 429, "{answer}");
    assert!(answer.to_lowercase().contains("retry-after: 1"), "{answer}");
    assert_eq!(status(address, "GET /healthz").await, 200);
    for one in held {
        assert_eq!(one.await.unwrap(), 200);
    }
    assert_eq!(status(address, "GET /slow?ms=0").await, 200);
}

#[tokio::test]
async fn a_download_holds_its_place_until_it_is_sent_and_a_reader_that_stops_loses_it() {
    let address = start(Limits {
        write_timeout: ms(400),
        max_requests: Some(1),
        ..unlimited()
    })
    .await;
    // the answer has begun (its head is read) and its body has not been taken: the
    // download still holds the only place
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"GET /huge HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    let mut head = [0u8; 15];
    stream.read_exact(&mut head).await.unwrap();
    assert_eq!(&head, b"HTTP/1.1 200 OK");
    assert_eq!(status(address, "GET /slow?ms=0").await, 503);
    // the reader stops reading: once a write has stalled for the limit, the connection
    // is closed and the place is free again
    let freed = eventually(address, "GET /slow?ms=0", 200, Duration::from_secs(8)).await;
    assert!(freed >= Duration::from_millis(200), "freed after {freed:?}");
    drop(stream);

    // a reader that reads gets all of it, however long that takes
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"GET /huge HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut total = 0usize;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        if read == 0 {
            break;
        }
        total += read;
    }
    assert!(total > 256 * 1024 * 1024, "{total} bytes");
}

#[tokio::test]
async fn connections_at_once_are_bounded_and_the_rest_wait() {
    let address = start(Limits {
        max_connections: Some(1),
        ..unlimited()
    })
    .await;
    // the first connection stays open after its answer
    let mut first = TcpStream::connect(address).await.unwrap();
    first
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    let mut head = [0u8; 15];
    first.read_exact(&mut head).await.unwrap();
    assert_eq!(&head, b"HTTP/1.1 200 OK");
    // the second connects (the system takes it) but is not served while the first is open
    let mut second = TcpStream::connect(address).await.unwrap();
    second
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    assert_eq!(
        read_all(&mut second, Duration::from_millis(500)).await,
        None
    );
    drop(first);
    let answer = read_all(&mut second, Duration::from_secs(5))
        .await
        .expect("served");
    assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
}

/// The archive itself, started as the binary starts it, has the limits of its settings.
#[tokio::test]
async fn the_server_applies_the_limits_of_its_settings() {
    use clap::Parser;
    let dir = tempfile::tempdir().unwrap();
    let settings = pacs_server::Settings::try_parse_from([
        "pacs-server",
        "--index",
        dir.path().join("index.sqlite").to_str().unwrap(),
        "--storage",
        &format!("file:{}", dir.path().join("files").display()),
        "--max-requests-per-client",
        "1",
        "--body-timeout",
        "1",
    ])
    .unwrap();
    let archive = pacs_server::open_archive(&settings).await.unwrap();
    let app = pacs_server::app(archive, &settings).unwrap();
    let limits = Limits::from_settings(&settings);
    assert_eq!(limits.max_requests_per_client, Some(1));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, app, limits, std::future::pending()));

    assert_eq!(status(address, "GET /studies").await, 200);
    // an upload that has stopped half way holds this address's one place
    let mut upload = TcpStream::connect(address).await.unwrap();
    upload
        .write_all(b"POST /instances HTTP/1.1\r\nHost: test\r\nContent-Length: 1000\r\n\r\nDICM")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(status(address, "GET /studies").await, 429);
    assert_eq!(status(address, "GET /healthz").await, 200);
    // ...until its body has paused for the second the settings allow
    eventually(address, "GET /studies", 200, Duration::from_secs(8)).await;
}

#[tokio::test]
async fn a_connection_left_idle_after_an_answer_is_closed_too() {
    let address = start(Limits {
        header_timeout: ms(300),
        ..unlimited()
    })
    .await;
    let mut stream = TcpStream::connect(address).await.unwrap();
    for _ in 0..2 {
        // a second request on the same connection, sent in time, is served
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: test\r\n\r\n")
            .await
            .unwrap();
        let mut answer = vec![0u8; 512];
        let read = stream.read(&mut answer).await.unwrap();
        assert!(answer[..read].starts_with(b"HTTP/1.1 200 OK"));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // then nothing more is asked: the next head is waited for as long as the first was
    let started = Instant::now();
    let closed = read_all(&mut stream, Duration::from_secs(5)).await;
    assert_eq!(
        closed.as_deref(),
        Some(""),
        "not closed, or something was sent"
    );
    assert!(started.elapsed() >= Duration::from_millis(150));
}
