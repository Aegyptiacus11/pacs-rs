//! Limits on HTTP requests, so that one client cannot hold the server: how long each part
//! of a request may take, and how many may be served at once.
//!
//! | Limit | Setting | Default | Over it |
//! |---|---|---|---|
//! | The request's head must arrive within | `PACS_HEADER_TIMEOUT` | 30 s | the connection is closed |
//! | The request's body may pause for at most | `PACS_BODY_TIMEOUT` | 60 s | the request fails, the connection is closed |
//! | A request without a body must be answered within | `PACS_REQUEST_TIMEOUT` | 300 s | 503 |
//! | A write to the client may stall for at most | `PACS_WRITE_TIMEOUT` | 60 s | the connection is closed |
//! | Connections open at once | `PACS_MAX_CONNECTIONS` | 1024 | further ones wait to be accepted |
//! | Requests being served at once | `PACS_MAX_REQUESTS` | 256 | 503, `Retry-After: 1` |
//! | Requests being served at once for one address | `PACS_MAX_REQUESTS_PER_CLIENT` | 64 | 429, `Retry-After: 1` |
//!
//! A timeout of 0 means none; a maximum of 0, no bound.
//!
//! - The head's timeout runs from the connection's opening, and again from each answer:
//!   a connection kept open and left idle is closed after it.
//! - The timeouts are about pauses, not sizes: an upload or a download may take as long as
//!   it needs while bytes keep moving.
//! - `PACS_REQUEST_TIMEOUT` is for `GET`, `HEAD`, `DELETE` and `OPTIONS`. A request with a
//!   body is bounded by the pauses of that body, and a synchronous C-MOVE
//!   (`POST /modalities/{id}/move`) by `PACS_DIMSE_MOVE_TIMEOUT`.
//! - A request counts as being served until its answer's body has been sent, a download
//!   to its last byte.
//! - `/healthz` is outside the two bounds on requests: a busy server still says it is up.
//! - Behind a reverse proxy every request comes from the proxy's address: raise
//!   `PACS_MAX_REQUESTS_PER_CLIENT`, or set it to 0 and bound clients at the proxy.
//!
//! The first, fourth and fifth are the connection's and live in [`serve`]; the others are
//! a layer around the application ([`Limits::layer`]).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http_body::{Frame, SizeHint};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_io_timeout::TimeoutStream;
use tower::ServiceExt;
use tower_http::timeout::TimeoutBody;

use crate::Settings;

/// The limits of a running server. `None` is no limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub header_timeout: Option<Duration>,
    pub body_timeout: Option<Duration>,
    pub request_timeout: Option<Duration>,
    pub write_timeout: Option<Duration>,
    pub max_connections: Option<usize>,
    pub max_requests: Option<usize>,
    pub max_requests_per_client: Option<usize>,
}

impl Default for Limits {
    /// The defaults of the settings.
    fn default() -> Self {
        Limits {
            header_timeout: Some(Duration::from_secs(30)),
            body_timeout: Some(Duration::from_secs(60)),
            request_timeout: Some(Duration::from_secs(300)),
            write_timeout: Some(Duration::from_secs(60)),
            max_connections: Some(1024),
            max_requests: Some(256),
            max_requests_per_client: Some(64),
        }
    }
}

impl Limits {
    pub fn from_settings(settings: &Settings) -> Self {
        let seconds = |n: u64| (n > 0).then(|| Duration::from_secs(n));
        let bound = |n: usize| (n > 0).then_some(n);
        Limits {
            header_timeout: seconds(settings.header_timeout),
            body_timeout: seconds(settings.body_timeout),
            request_timeout: seconds(settings.request_timeout),
            write_timeout: seconds(settings.write_timeout),
            max_connections: bound(settings.max_connections),
            max_requests: bound(settings.max_requests),
            max_requests_per_client: bound(settings.max_requests_per_client),
        }
    }

    /// Put the bounds on requests, and the timeouts of a request, around `app`.
    pub fn layer(&self, app: Router) -> Router {
        let limiter = Arc::new(Limiter {
            limits: self.clone(),
            slots: self.max_requests.map(|n| Arc::new(Semaphore::new(n))),
            by_client: Mutex::new(HashMap::new()),
        });
        app.layer(axum::middleware::from_fn_with_state(limiter, limit))
    }
}

struct Limiter {
    limits: Limits,
    /// One permit per request being served, when there is a bound.
    slots: Option<Arc<Semaphore>>,
    /// How many requests each address has being served now.
    by_client: Mutex<HashMap<IpAddr, usize>>,
}

/// A request's place among those being served: given back when it is dropped, which is
/// when the answer's body has been sent, or given up.
struct Place {
    limiter: Arc<Limiter>,
    client: Option<IpAddr>,
    _slot: Option<OwnedSemaphorePermit>,
}

impl Drop for Place {
    fn drop(&mut self) {
        let Some(client) = self.client else { return };
        let mut by_client = match self.limiter.by_client.lock() {
            Ok(by_client) => by_client,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(count) = by_client.get_mut(&client) {
            *count -= 1;
            if *count == 0 {
                by_client.remove(&client);
            }
        }
    }
}

/// Why a request is not served now.
#[derive(Debug, PartialEq, Eq)]
enum Busy {
    Server,
    Client,
}

impl Limiter {
    /// A place for a request from `client`, or why there is none.
    fn place(self: &Arc<Self>, client: Option<IpAddr>) -> Result<Place, Busy> {
        // the client's own bound first: one over it must not take a slot from the others
        let counted = match (client, self.limits.max_requests_per_client) {
            (Some(client), Some(most)) => {
                let mut by_client = match self.by_client.lock() {
                    Ok(by_client) => by_client,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let count = by_client.entry(client).or_insert(0);
                if *count >= most {
                    return Err(Busy::Client);
                }
                *count += 1;
                Some(client)
            }
            _ => None,
        };
        // from here the count is given back by the place's drop
        let mut place = Place {
            limiter: self.clone(),
            client: counted,
            _slot: None,
        };
        if let Some(slots) = &self.slots {
            match slots.clone().try_acquire_owned() {
                Ok(slot) => place._slot = Some(slot),
                Err(_) => return Err(Busy::Server),
            }
        }
        Ok(place)
    }
}

fn busy(why: Busy) -> Response {
    let (status, said) = match why {
        Busy::Server => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The server is serving as many requests as it may: try again\n",
        ),
        Busy::Client => (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests at once from this address: try again\n",
        ),
    };
    let mut response = (status, said).into_response();
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

/// A body that keeps its request's place until it has been sent or dropped.
struct Held {
    body: Body,
    _place: Place,
}

impl http_body::Body for Held {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        // axum's body is boxed, so it can be polled wherever this one sits
        Pin::new(&mut self.get_mut().body).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

/// Around every request: a place among those being served, a body that may not stall,
/// and for a request without one, an answer in time.
async fn limit(State(limiter): State<Arc<Limiter>>, request: Request, next: Next) -> Response {
    // a busy server still says it is up, and whether it can serve
    if matches!(request.uri().path(), "/healthz" | "/readyz") {
        return next.run(request).await;
    }
    let client = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip());
    let place = match limiter.place(client) {
        Ok(place) => place,
        Err(why) => {
            tracing::warn!(?client, ?why, "request refused: too many at once");
            return busy(why);
        }
    };
    let bodiless = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::DELETE | Method::OPTIONS
    );
    let request = match limiter.limits.body_timeout {
        Some(pause) => request.map(|body| Body::new(TimeoutBody::new(pause, body))),
        None => request,
    };
    let response = match limiter.limits.request_timeout.filter(|_| bodiless) {
        Some(most) => match tokio::time::timeout(most, next.run(request)).await {
            Ok(response) => response,
            Err(_) => {
                tracing::warn!(
                    ?client,
                    seconds = most.as_secs(),
                    "request abandoned: no answer in time"
                );
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The server took too long to answer\n",
                )
                    .into_response();
            }
        },
        None => next.run(request).await,
    };
    response.map(|body| {
        Body::new(Held {
            body,
            _place: place,
        })
    })
}

/// Serve `app` on `listener` until `shutdown` completes, then finish open requests: the
/// connection side of the limits, with each caller's address on its requests.
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    limits: Limits,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let mut builder = Builder::new(TokioExecutor::new());
    // hyper's timeouts do nothing without a timer
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_timeout);
    // HTTP/2 has no head to time: a peer that is gone is found by a ping it does not answer
    builder
        .http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(Some(Duration::from_secs(20)))
        .keep_alive_timeout(Duration::from_secs(20));
    let connections = limits.max_connections.map(|n| Arc::new(Semaphore::new(n)));
    let graceful = GracefulShutdown::new();
    tokio::pin!(shutdown);
    loop {
        // a connection is accepted only when there is room for it: the others wait in
        // the listener's queue
        let room = async {
            match &connections {
                Some(connections) => connections.clone().acquire_owned().await.ok(),
                None => None,
            }
        };
        let (room, accepted) = tokio::select! {
            () = &mut shutdown => break,
            accepted = async { (room.await, listener.accept().await) } => accepted,
        };
        let (socket, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                // out of file descriptors, or a connection reset before it was accepted:
                // neither is a reason to stop serving
                tracing::warn!(%error, "accept failed");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let service = app.clone().map_request(move |mut request: Request<_>| {
            request.extensions_mut().insert(ConnectInfo(peer));
            request
        });
        let (builder, watcher) = (builder.clone(), graceful.watcher());
        let (header_timeout, write_timeout) = (limits.header_timeout, limits.write_timeout);
        tokio::spawn(async move {
            // The head's timeout is HTTP/1's and starts with the head's first byte, after
            // the version has been told from it: a connection that says nothing at all
            // is waited for here.
            if let Some(most) = header_timeout {
                let mut first = [0u8; 1];
                let first_byte = socket.peek(&mut first);
                if !matches!(tokio::time::timeout(most, first_byte).await, Ok(Ok(1..))) {
                    tracing::debug!(%peer, "connection closed: it sent nothing");
                    return;
                }
            }
            let mut socket = TimeoutStream::new(socket);
            socket.set_write_timeout(write_timeout);
            let connection = builder.serve_connection_with_upgrades(
                TokioIo::new(Box::pin(socket)),
                TowerToHyperService::new(service),
            );
            // a client that goes away mid-request is not this server's error
            if let Err(error) = watcher.watch(connection.into_owned()).await {
                tracing::debug!(%peer, %error, "connection ended");
            }
            drop(room);
        });
    }
    graceful.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(max_requests: Option<usize>, per_client: Option<usize>) -> Arc<Limiter> {
        Arc::new(Limiter {
            limits: Limits {
                max_requests,
                max_requests_per_client: per_client,
                ..Limits::default()
            },
            slots: max_requests.map(|n| Arc::new(Semaphore::new(n))),
            by_client: Mutex::new(HashMap::new()),
        })
    }

    fn ip(last: u8) -> Option<IpAddr> {
        Some(IpAddr::from([192, 0, 2, last]))
    }

    #[test]
    fn places_are_bounded_for_the_server_and_for_each_client() {
        let limiter = limiter(Some(3), Some(2));
        let first = limiter.place(ip(1)).unwrap();
        let _second = limiter.place(ip(1)).unwrap();
        // the client's bound: its third is refused, and takes no slot from the others
        assert_eq!(limiter.place(ip(1)).err(), Some(Busy::Client));
        let _other = limiter.place(ip(2)).unwrap();
        // the server's bound: full, whoever asks
        assert_eq!(limiter.place(ip(3)).err(), Some(Busy::Server));
        assert_eq!(limiter.place(None).err(), Some(Busy::Server));
        // a refusal for the server leaves the client's count as it was
        assert!(
            !limiter
                .by_client
                .lock()
                .unwrap()
                .contains_key(&ip(3).unwrap())
        );
        assert_eq!(limiter.by_client.lock().unwrap()[&ip(2).unwrap()], 1);

        // a place given back is a place again, for the server and for that client
        drop(first);
        assert_eq!(limiter.by_client.lock().unwrap()[&ip(1).unwrap()], 1);
        let _again = limiter.place(ip(1)).unwrap();
        assert_eq!(limiter.place(ip(1)).err(), Some(Busy::Client));
    }

    #[test]
    fn a_client_that_is_done_is_forgotten() {
        let limiter = limiter(None, Some(2));
        let (a, b) = (limiter.place(ip(1)).unwrap(), limiter.place(ip(1)).unwrap());
        drop(a);
        drop(b);
        assert!(limiter.by_client.lock().unwrap().is_empty());
    }

    #[test]
    fn no_bound_is_no_bound() {
        let limiter = limiter(None, None);
        let places: Vec<_> = (0..1000).map(|_| limiter.place(ip(1)).unwrap()).collect();
        assert_eq!(places.len(), 1000);
        // nothing is counted when nothing is bounded
        assert!(limiter.by_client.lock().unwrap().is_empty());
        // a request whose address is not known is bounded by the server's bound alone
        let limiter = self::limiter(Some(1), Some(1));
        let _one = limiter.place(None).unwrap();
        assert_eq!(limiter.place(None).err(), Some(Busy::Server));
    }

    #[test]
    fn zero_in_the_settings_is_no_limit() {
        use clap::Parser;
        let none = Settings::try_parse_from([
            "pacs-server",
            "--header-timeout",
            "0",
            "--body-timeout",
            "0",
            "--request-timeout",
            "0",
            "--write-timeout",
            "0",
            "--max-connections",
            "0",
            "--max-requests",
            "0",
            "--max-requests-per-client",
            "0",
        ])
        .unwrap();
        let limits = Limits::from_settings(&none);
        assert_eq!(
            limits,
            Limits {
                header_timeout: None,
                body_timeout: None,
                request_timeout: None,
                write_timeout: None,
                max_connections: None,
                max_requests: None,
                max_requests_per_client: None,
            }
        );
        let defaults = Settings::try_parse_from(["pacs-server"]).unwrap();
        assert_eq!(Limits::from_settings(&defaults), Limits::default());
    }
}
