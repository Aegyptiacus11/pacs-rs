//! `multipart/related` (RFC 2387) bodies: written for WADO-RS, read for STOW-RS.

use std::hash::{BuildHasher, Hasher};

use bytes::{Bytes, BytesMut};
use memchr::memmem;

/// A fresh boundary: 128 random bits in hex, from the standard library's hasher seeds.
pub fn new_boundary() -> String {
    let random = || {
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    };
    format!("pacs-rs-{:016x}{:016x}", random(), random())
}

/// The header of one part, with the delimiter before it.
pub fn part_header(boundary: &str, content_type: &str) -> Bytes {
    Bytes::from(format!(
        "--{boundary}\r\nContent-Type: {content_type}\r\n\r\n"
    ))
}

/// The delimiter after the last part.
pub fn closing(boundary: &str) -> Bytes {
    Bytes::from(format!("--{boundary}--\r\n"))
}

/// A whole body from parts already in memory.
pub fn body(boundary: &str, parts: impl IntoIterator<Item = (String, Bytes)>) -> Bytes {
    let mut out = BytesMut::new();
    for (content_type, content) in parts {
        out.extend_from_slice(&part_header(boundary, &content_type));
        out.extend_from_slice(&content);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(&closing(boundary));
    out.freeze()
}

/// One part of a received body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// The part's Content-Type, if it has one.
    pub content_type: Option<String>,
    pub content: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

/// Split a body on `boundary` into its parts. The preamble and epilogue are ignored.
pub fn parse(boundary: &str, body: &Bytes) -> Result<Vec<Part>, ParseError> {
    let delimiter = format!("--{boundary}");
    // Every delimiter but the first follows a line break that belongs to it.
    let next_delimiter = format!("\r\n--{boundary}");
    let next_finder = memmem::Finder::new(next_delimiter.as_bytes());
    let mut at = if body.starts_with(delimiter.as_bytes()) {
        delimiter.len()
    } else {
        next_finder
            .find(body)
            .map(|i| i + next_delimiter.len())
            .ok_or_else(|| ParseError("no opening boundary".into()))?
    };
    let mut parts = Vec::new();
    loop {
        let rest = &body[at..];
        if rest.starts_with(b"--") {
            return Ok(parts);
        }
        // the rest of the delimiter line: optional transport padding, then CRLF
        let part_start = at
            + memmem::find(rest, b"\r\n")
                .ok_or_else(|| ParseError("unterminated boundary line".into()))?
            + 2;
        let (headers, content_start) = if body[part_start..].starts_with(b"\r\n") {
            ("", part_start + 2)
        } else {
            let end = memmem::find(&body[part_start..], b"\r\n\r\n")
                .map(|i| part_start + i)
                .ok_or_else(|| ParseError("part headers never end".into()))?;
            let headers = std::str::from_utf8(&body[part_start..end])
                .map_err(|_| ParseError("part headers are not text".into()))?;
            (headers, end + 4)
        };
        let content_end = next_finder
            .find(&body[content_start..])
            .map(|i| content_start + i)
            .ok_or_else(|| ParseError("no closing boundary".into()))?;
        parts.push(Part {
            content_type: headers.split("\r\n").find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("content-type")
                    .then(|| value.trim().to_owned())
            }),
            content: body.slice(content_start..content_end),
        });
        at = content_end + next_delimiter.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back() {
        let boundary = new_boundary();
        let parts = vec![
            (
                "application/dicom".to_owned(),
                Bytes::from_static(b"first\r\n--almost"),
            ),
            ("application/dicom".to_owned(), Bytes::from_static(b"")),
            ("image/jls".to_owned(), Bytes::from_static(&[0, 1, 2, 255])),
        ];
        let body = body(&boundary, parts.clone());
        let read = parse(&boundary, &body).unwrap();
        assert_eq!(read.len(), 3);
        for (part, (content_type, content)) in read.iter().zip(&parts) {
            assert_eq!(part.content_type.as_deref(), Some(content_type.as_str()));
            assert_eq!(&part.content, content);
        }
    }

    #[test]
    fn preamble_and_headerless_parts() {
        let body = Bytes::from_static(
            b"preamble\r\n--b\r\n\r\nno headers\r\n--b\r\ncontent-type: x/y\r\nX-Other: 1\r\n\r\ntwo\r\n--b--\r\nepilogue",
        );
        let read = parse("b", &body).unwrap();
        assert_eq!(read[0].content_type, None);
        assert_eq!(read[0].content, "no headers");
        assert_eq!(read[1].content_type.as_deref(), Some("x/y"));
        assert_eq!(read[1].content, "two");
    }

    #[test]
    fn a_truncated_body_is_an_error() {
        let body = Bytes::from_static(b"--b\r\nContent-Type: a/b\r\n\r\ncut off");
        assert!(parse("b", &body).is_err());
        assert!(parse("b", &Bytes::from_static(b"nothing here")).is_err());
    }

    #[test]
    fn boundaries_differ() {
        assert_ne!(new_boundary(), new_boundary());
    }
}
