//! Media types: parsing Content-Type and Accept, and choosing what to answer with.

/// A media type with its parameters, e.g. `multipart/related; type="application/dicom"`.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaType {
    /// Lower case, e.g. `multipart/related`.
    pub essence: String,
    /// Names in lower case, values unquoted.
    pub params: Vec<(String, String)>,
}

impl MediaType {
    pub fn parse(text: &str) -> Option<MediaType> {
        let mut pieces = split_unquoted(text, ';').into_iter();
        let essence = pieces.next()?.trim().to_ascii_lowercase();
        if essence.is_empty() {
            return None;
        }
        let params = pieces
            .filter_map(|p| {
                let (name, value) = p.split_once('=')?;
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(value);
                Some((name.trim().to_ascii_lowercase(), value.to_owned()))
            })
            .collect();
        Some(MediaType { essence, params })
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn quality(&self) -> f32 {
        self.param("q").and_then(|q| q.parse().ok()).unwrap_or(1.0)
    }
}

/// An Accept header: its ranges, without those of quality 0, or `None` when there is
/// no header, which accepts anything.
#[derive(Debug, Clone, PartialEq)]
pub struct Accept(Option<Vec<MediaType>>);

impl Accept {
    pub fn parse(header: Option<&str>) -> Accept {
        Accept(header.map(|a| {
            split_unquoted(a, ',')
                .into_iter()
                .filter_map(MediaType::parse)
                .filter(|m| m.quality() > 0.0)
                .collect()
        }))
    }
}

/// How to send content whose parts are `part_type` in transfer syntax `ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packaging {
    /// `multipart/related`, as WADO-RS specifies.
    Multipart,
    /// The part on its own, for a client that asked for exactly that media type.
    Single,
}

/// Whether some range of `ranges` accepts `part_type` content in transfer syntax `ts`,
/// and how to package it. `None` means 406 Not Acceptable, unless
/// [`negotiate_transcoded`] finds something else the server can make.
pub fn negotiate(accept: &Accept, part_type: &str, ts: &str) -> Option<Packaging> {
    let Some(ranges) = &accept.0 else {
        return Some(Packaging::Multipart);
    };
    ranges
        .iter()
        .find_map(|range| range_accepts(range, part_type, ts))
}

/// For content that is not acceptable as stored: the first of `candidates` (part type,
/// transfer syntax) that a range accepts, ranges taken in the client's order, and how to
/// package it. Returns the candidate's index.
pub fn negotiate_transcoded(
    accept: &Accept,
    candidates: &[(&str, &str)],
) -> Option<(usize, Packaging)> {
    let ranges = accept.0.as_ref()?;
    ranges.iter().find_map(|range| {
        candidates
            .iter()
            .enumerate()
            .find_map(|(i, (part_type, ts))| range_accepts(range, part_type, ts).map(|p| (i, p)))
    })
}

fn range_accepts(range: &MediaType, part_type: &str, ts: &str) -> Option<Packaging> {
    let ts_ok = |requested: Option<&str>| requested.is_none_or(|r| r == "*" || r == ts);
    match range.essence.as_str() {
        "*/*" | "multipart/*" => Some(Packaging::Multipart),
        "multipart/related" => {
            let requested_ts = range.param("transfer-syntax");
            if requested_ts == Some("*") {
                return Some(Packaging::Multipart);
            }
            let type_ok = range
                .param("type")
                .is_none_or(|t| t.eq_ignore_ascii_case(part_type) || t == "*/*");
            (type_ok && ts_ok(requested_ts)).then_some(Packaging::Multipart)
        }
        essence if essence == part_type && ts_ok(range.param("transfer-syntax")) => {
            Some(Packaging::Single)
        }
        _ => None,
    }
}

/// Whether the client takes DICOM JSON.
pub fn accepts_json(accept: &Accept) -> bool {
    accept.0.as_ref().is_none_or(|ranges| {
        ranges.iter().any(|r| {
            matches!(
                r.essence.as_str(),
                "application/dicom+json" | "application/json" | "application/*" | "*/*"
            )
        })
    })
}

fn split_unquoted(text: &str, separator: char) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            c if c == separator && !quoted => {
                pieces.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    pieces.push(&text[start..]);
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    const ELE: &str = "1.2.840.10008.1.2.1";
    const JLS: &str = "1.2.840.10008.1.2.4.80";

    #[test]
    fn content_type_with_quoted_params() {
        let m = MediaType::parse(r#"Multipart/Related; type="application/dicom"; boundary="a;b""#)
            .unwrap();
        assert_eq!(m.essence, "multipart/related");
        assert_eq!(m.param("type"), Some("application/dicom"));
        assert_eq!(m.param("boundary"), Some("a;b"));
    }

    #[test]
    fn what_cornerstone_asks_for_frames() {
        let ranges = Accept::parse(Some(
            "multipart/related; type=application/octet-stream; transfer-syntax=*",
        ));
        assert_eq!(
            negotiate(&ranges, "application/octet-stream", ELE),
            Some(Packaging::Multipart)
        );
        assert_eq!(
            negotiate(&ranges, "image/jls", JLS),
            Some(Packaging::Multipart)
        );
    }

    #[test]
    fn no_transcoding() {
        let ranges = Accept::parse(Some(
            r#"multipart/related; type="application/octet-stream"; transfer-syntax=1.2.840.10008.1.2.1"#,
        ));
        assert_eq!(negotiate(&ranges, "image/jls", JLS), None);
        let ranges = Accept::parse(Some(
            r#"multipart/related; type="application/octet-stream""#,
        ));
        assert_eq!(negotiate(&ranges, "image/jls", JLS), None);
        assert_eq!(
            negotiate(&ranges, "application/octet-stream", ELE),
            Some(Packaging::Multipart)
        );
    }

    #[test]
    fn transcoding_follows_the_clients_order() {
        const JXL: &str = "1.2.840.10008.1.2.4.110";
        let candidates = [
            ("application/octet-stream", ELE),
            ("image/jls", JLS),
            ("image/jxl", JXL),
        ];
        let ranges = Accept::parse(Some(
            r#"multipart/related; type="image/jxl", multipart/related; type="application/octet-stream""#,
        ));
        assert_eq!(negotiate(&ranges, "image/jls", JLS), None);
        assert_eq!(
            negotiate_transcoded(&ranges, &candidates),
            Some((2, Packaging::Multipart))
        );
        let ranges = Accept::parse(Some(
            r#"multipart/related; type="application/octet-stream"; transfer-syntax=1.2.840.10008.1.2.1"#,
        ));
        assert_eq!(
            negotiate_transcoded(&ranges, &candidates),
            Some((0, Packaging::Multipart))
        );
        let ranges = Accept::parse(Some("image/png"));
        assert_eq!(negotiate_transcoded(&ranges, &candidates), None);
        assert_eq!(
            negotiate_transcoded(&Accept::parse(None), &candidates),
            None
        );
    }

    #[test]
    fn single_part_and_wildcards() {
        let ranges = Accept::parse(Some("application/dicom"));
        assert_eq!(
            negotiate(&ranges, "application/dicom", JLS),
            Some(Packaging::Single)
        );
        assert_eq!(
            negotiate(&Accept::parse(Some("*/*")), "image/jls", JLS),
            Some(Packaging::Multipart)
        );
        assert_eq!(
            negotiate(&Accept::parse(None), "image/jls", JLS),
            Some(Packaging::Multipart)
        );
        assert_eq!(
            negotiate(&Accept::parse(Some("image/png")), "image/jls", JLS),
            None
        );
        assert_eq!(
            negotiate(&Accept::parse(Some("*/*;q=0")), "image/jls", JLS),
            None
        );
    }

    #[test]
    fn json() {
        assert!(accepts_json(&Accept::parse(Some("application/dicom+json"))));
        assert!(accepts_json(&Accept::parse(None)));
        assert!(!accepts_json(&Accept::parse(Some("application/dicom+xml"))));
    }
}
