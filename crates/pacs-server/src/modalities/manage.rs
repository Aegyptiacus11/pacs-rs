//! Remote AEs added, changed and removed while the server runs (`PUT` and `DELETE
//! /modalities/{id}`), for an administrator: kept in the index, so the server finds them again
//! when it starts, and put in the directory every part of the server shares, so the next
//! association, search or move sees them. Those of `PACS_DIMSE_REMOTES` are the settings'
//! and are changed there only.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pacs_dimse::{AeTitle, RemoteAe};
use pacs_index::StoredRemoteAe;
use pacs_rest::RestError;

use super::schema::{Error, NewRemoteAe, RemoteAe as Described};
use super::{Remotes, describe};

fn conflict(message: &str, details: String) -> Response {
    RestError::Status {
        status: StatusCode::CONFLICT,
        message: message.into(),
        details: Some(details),
    }
    .into_response()
}

/// Why `id` cannot name a remote AE, if it cannot: it goes in URLs and in settings lists.
fn bad_id(id: &str) -> Option<String> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    (!ok).then(|| format!("{id:?} is not an id: 1 to 64 ASCII letters, digits, _ and -"))
}

/// Why `host` cannot be one, if it cannot: a host name (RFC 1123) or an IP address.
fn bad_host(host: &str) -> Option<String> {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    // a name's last label is not all digits (RFC 1123 2.1): 999.1.1.1 is a bad address
    let last_numeric = host
        .rsplit('.')
        .next()
        .is_some_and(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()));
    let name = !host.is_empty()
        && !last_numeric
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        });
    (!name).then(|| format!("{host:?} is neither a host name nor an IP address"))
}

/// A remote AE kept in the index, as the directory holds it; why not, if it cannot be one.
pub(crate) fn stored_remote(stored: &StoredRemoteAe) -> Result<RemoteAe, String> {
    if let Some(why) = bad_id(&stored.id).or_else(|| bad_host(&stored.host)) {
        return Err(why);
    }
    if stored.port == 0 {
        return Err("port 0 is no port".into());
    }
    Ok(RemoteAe {
        id: stored.id.clone(),
        ae_title: AeTitle::new(&stored.ae_title).map_err(|e| e.to_string())?,
        host: stored.host.clone(),
        port: stored.port,
        secure: stored.secure,
        tls: None,
    })
}

/// Adds a remote AE, or changes one added before.
///
/// Kept by the server, and used at once: by the next C-ECHO, search, move or send, and by the
/// DIMSE server, which answers and sends to the remote AEs it knows. One of
/// `PACS_DIMSE_REMOTES` is changed there, not here. Two remote AEs that may receive studies
/// cannot share an AE title: a C-MOVE names its destination by it. One that forwarding or a
/// project's write-back sends to may be given a new address, but studies may still be sent to
/// it.
#[utoipa::path(
    put,
    path = "/modalities/{id}",
    tag = "Remote archives",
    params(("id" = String, Path, description = "The remote AE's id: 1 to 64 ASCII letters, digits, _ and -.")),
    request_body(content = NewRemoteAe, description = "Its AE title, host, port, TLS, and whether studies may be sent to it."),
    responses(
        (status = 200, description = "Added or changed: the remote AE as it is now.", body = Described),
        (status = 400, description = "Not a remote AE: a bad id, AE title, host or port, or TLS this server has no DIMSE TLS for.", body = Error),
        (status = 409, description = "One of `PACS_DIMSE_REMOTES`; another that may receive studies has its AE title; or the settings send to it and studies would no longer be sent to it.", body = Error)
    )
)]
pub(crate) async fn put(
    State(remotes): State<Arc<Remotes>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let asked: NewRemoteAe = match serde_json::from_slice(&body) {
        Ok(asked) => asked,
        Err(e) => {
            return RestError::bad_request("Bad request", format!("not a remote AE: {e}"))
                .into_response();
        }
    };
    let _changing = remotes.changes.lock().await;
    let directory = &remotes.config.directory;
    if directory.is_fixed(&id) {
        return conflict(
            "Set in the settings",
            format!("{id} is one of PACS_DIMSE_REMOTES: change it there"),
        );
    }
    // what the settings send to may move (a new address), not stop taking studies
    if !asked.may_receive
        && let Some((_, why)) = remotes.in_use.iter().find(|(used, _)| *used == id)
    {
        return conflict(
            "In use",
            format!("{id} is what {why} sends to: studies may still be sent to it"),
        );
    }
    let stored = StoredRemoteAe {
        id: id.clone(),
        ae_title: asked.aet.trim().to_owned(),
        host: asked.host.trim().to_owned(),
        port: asked.port,
        secure: asked.tls,
        may_receive: asked.may_receive,
    };
    let remote = match stored_remote(&stored) {
        Ok(remote) => remote,
        Err(why) => return RestError::bad_request("Bad request", why).into_response(),
    };
    if remote.secure && !directory.speaks_tls() {
        return RestError::bad_request(
            "Bad request",
            "this server has no DIMSE TLS to call it with: set PACS_DIMSE_TLS_CA_FILE",
        )
        .into_response();
    }
    if asked.may_receive
        && let Some(other) = directory
            .move_destination(remote.ae_title.as_str())
            .filter(|other| other.id != id)
    {
        return conflict(
            "AE title taken",
            format!(
                "{} may already receive studies as {}: a C-MOVE could not tell them apart",
                other.id, remote.ae_title
            ),
        );
    }
    // kept first: what the directory has, the next start finds again
    if let Err(e) = remotes.index.put_remote_ae(&stored).await {
        return RestError::from(e).into_response();
    }
    directory.set(remote.clone(), asked.may_receive);
    tracing::info!(remote = %id, ae_title = %remote.ae_title, host = %remote.host, port = remote.port, "remote AE set");
    Json(describe(&remote, directory)).into_response()
}

/// Removes a remote AE added through the API.
///
/// The searches, moves and sends asked of it from then on are refused as for an unknown one.
/// One of `PACS_DIMSE_REMOTES` is removed there; one that forwarding or a project's write-back
/// sends to is not removed while they do.
#[utoipa::path(
    delete,
    path = "/modalities/{id}",
    tag = "Remote archives",
    params(("id" = String, Path, description = "The remote AE's id.")),
    responses(
        (status = 200, description = "Removed; the body is empty."),
        (status = 404, description = "No remote AE has this id; the body is empty."),
        (status = 409, description = "One of `PACS_DIMSE_REMOTES`, or one the settings send to.", body = Error)
    )
)]
pub(crate) async fn delete(
    State(remotes): State<Arc<Remotes>>,
    Path(id): Path<String>,
) -> Response {
    let _changing = remotes.changes.lock().await;
    let directory = &remotes.config.directory;
    if directory.remote(&id).is_none() {
        return RestError::NotFound.into_response();
    }
    if directory.is_fixed(&id) {
        return conflict(
            "Set in the settings",
            format!("{id} is one of PACS_DIMSE_REMOTES: remove it there"),
        );
    }
    if let Some((_, why)) = remotes.in_use.iter().find(|(used, _)| *used == id) {
        return conflict("In use", format!("{id} is what {why} sends to"));
    }
    if let Err(e) = remotes.index.delete_remote_ae(&id).await {
        return RestError::from(e).into_response();
    }
    directory.remove(&id);
    tracing::info!(remote = %id, "remote AE removed");
    StatusCode::OK.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_hosts_are_checked() {
        for good in ["pacs", "ws-2", "A_1"] {
            assert_eq!(bad_id(good), None);
        }
        for bad in ["", "a b", "a/b", &"x".repeat(65)] {
            assert!(bad_id(bad).is_some(), "{bad:?}");
        }
        for good in ["10.0.0.5", "::1", "pacs.hospital.local", "ws2"] {
            assert_eq!(bad_host(good), None, "{good}");
        }
        for bad in [
            "",
            "a b",
            "-x.local",
            "x..local",
            "pacs.hospital.local:104",
            "http://pacs",
            "999.1.1.1",
        ] {
            assert!(bad_host(bad).is_some(), "{bad:?}");
        }
    }
}
