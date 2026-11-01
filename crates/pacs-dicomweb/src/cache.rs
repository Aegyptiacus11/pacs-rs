//! Browser caching of what WADO-RS sends. A response is a function of the stored files
//! (their keys, sizes and when they were received), the Accept header and this server's
//! version, so its validator is a hash of those: a client holding it gets 304 Not
//! Modified before any file is read. The tag is weak because a multipart body's boundary
//! differs from one response to the next.
//!
//! One instance and its frames cannot change while instances are never overwritten, so
//! the browser keeps them a day without asking; study and series retrieves gain
//! instances as they arrive, so those (and everything when overwriting is on) are asked
//! about each time, which costs a round trip and not the bytes. `private` keeps them out
//! of shared caches: the deployment in front decides who may read a study.

use std::hash::{DefaultHasher, Hash, Hasher};

use axum::http::header::{ACCEPT, CACHE_CONTROL, ETAG, IF_NONE_MATCH, VARY};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use pacs_index::InstanceEntry;

/// How long a browser keeps what cannot change without asking again.
const IMMUTABLE: &str = "private, max-age=86400";
/// Kept, but asked about each time.
const REVALIDATE: &str = "private, no-cache";

/// A response's validator and how long it may be kept.
#[derive(Debug, Clone)]
pub(crate) struct Validator {
    etag: String,
    cache_control: &'static str,
}

impl Validator {
    /// For `entries` sent as the request's Accept header asks; `extra` tells apart what
    /// one URL can send otherwise (the frame numbers). `fixed` when the response cannot
    /// change: one instance, never overwritten.
    pub(crate) fn for_entries(
        headers: &HeaderMap,
        entries: &[InstanceEntry],
        extra: &str,
        fixed: bool,
    ) -> Validator {
        let mut hasher = DefaultHasher::new();
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        headers
            .get(ACCEPT)
            .map(HeaderValue::as_bytes)
            .hash(&mut hasher);
        extra.hash(&mut hasher);
        for e in entries {
            (
                &e.sop_instance_uid,
                &e.object_key,
                e.size_bytes,
                e.received_ms,
            )
                .hash(&mut hasher);
        }
        Validator::new(hasher.finish(), fixed)
    }

    /// For a body already in hand.
    pub(crate) fn for_body(body: &str) -> Validator {
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        Validator::new(hasher.finish(), false)
    }

    fn new(hash: u64, fixed: bool) -> Validator {
        Validator {
            etag: format!(r#"W/"{hash:016x}""#),
            cache_control: if fixed { IMMUTABLE } else { REVALIDATE },
        }
    }

    /// Whether the client already holds this response (If-None-Match, compared weakly).
    pub(crate) fn matches(&self, headers: &HeaderMap) -> bool {
        let opaque = |tag: &str| tag.trim().trim_start_matches("W/").to_owned();
        let ours = opaque(&self.etag);
        headers
            .get_all(IF_NONE_MATCH)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .flat_map(|h| h.split(','))
            .any(|tag| tag.trim() == "*" || opaque(tag) == ours)
    }

    pub(crate) fn not_modified(&self) -> Response {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        self.apply(&mut response);
        response
    }

    /// The validator's headers on `response`.
    pub(crate) fn apply(&self, response: &mut Response) {
        let headers = response.headers_mut();
        headers.insert(ETAG, HeaderValue::from_str(&self.etag).expect("hex"));
        headers.insert(CACHE_CONTROL, HeaderValue::from_static(self.cache_control));
        headers.insert(VARY, HeaderValue::from_static("Accept"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(received_ms: i64) -> InstanceEntry {
        InstanceEntry {
            study_instance_uid: "1.2".into(),
            series_instance_uid: "1.2.3".into(),
            sop_instance_uid: "1.2.3.4".into(),
            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".into(),
            transfer_syntax_uid: "1.2.840.10008.1.2.4.80".into(),
            object_key: "1.2/1.2.3/1.2.3.4.dcm".into(),
            size_bytes: 1000,
            received_ms,
            attributes: serde_json::Value::Null,
        }
    }

    fn headers(pairs: &[(axum::http::HeaderName, &str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(n, v)| (n.clone(), HeaderValue::from_str(v).unwrap()))
            .collect()
    }

    #[test]
    fn the_tag_follows_the_file_and_the_accept_header() {
        let jls = headers(&[(ACCEPT, r#"multipart/related; type="image/jls""#)]);
        let native = headers(&[(ACCEPT, "multipart/related; type=application/octet-stream")]);
        let tag = |h: &HeaderMap, e: &[InstanceEntry], extra: &str| {
            Validator::for_entries(h, e, extra, true).etag
        };
        let a = tag(&jls, &[entry(1)], "1");
        assert_eq!(a, tag(&jls, &[entry(1)], "1"));
        assert_ne!(a, tag(&native, &[entry(1)], "1"));
        assert_ne!(a, tag(&jls, &[entry(2)], "1"));
        assert_ne!(a, tag(&jls, &[entry(1)], "2"));
        assert!(a.starts_with(r#"W/""#));
    }

    #[test]
    fn if_none_match() {
        let v = Validator::for_body("[]");
        let strong = v.etag.trim_start_matches("W/").to_owned();
        assert!(v.matches(&headers(&[(IF_NONE_MATCH, &v.etag)])));
        assert!(v.matches(&headers(&[(IF_NONE_MATCH, &strong)])));
        assert!(v.matches(&headers(&[(IF_NONE_MATCH, &format!(r#""x", {}"#, v.etag))])));
        assert!(v.matches(&headers(&[(IF_NONE_MATCH, "*")])));
        assert!(!v.matches(&headers(&[(IF_NONE_MATCH, r#"W/"0""#)])));
        assert!(!v.matches(&HeaderMap::new()));
    }

    #[test]
    fn what_may_be_kept() {
        let mut response = ().into_response();
        Validator::for_entries(&HeaderMap::new(), &[entry(1)], "", true).apply(&mut response);
        assert_eq!(response.headers()[CACHE_CONTROL], IMMUTABLE);
        assert_eq!(response.headers()[VARY], "Accept");
        let mut response = ().into_response();
        Validator::for_entries(&HeaderMap::new(), &[entry(1)], "", false).apply(&mut response);
        assert_eq!(response.headers()[CACHE_CONTROL], REVALIDATE);
        assert_eq!(Validator::for_body("[]").not_modified().status(), 304);
    }
}
