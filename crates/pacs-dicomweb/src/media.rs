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
        self.param("q")
            .and_then(|q| q.parse::<f32>().ok())
            .filter(|q| q.is_finite())
            .map_or(1.0, |q| q.clamp(0.0, 1.0))
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

/// What to send: the content as stored, or one of the transcoding candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Stored,
    /// The index of the candidate.
    Candidate(usize),
}

/// What to answer with, and how to package it, for content stored as `stored` (part type,
/// transfer syntax) that the server can also make as any of `candidates`. The client's
/// preference decides (PS3.18 8.7.3.5): ranges by quality, those of equal quality in the
/// order sent. Within the first range that takes anything, the content as stored wins,
/// then the candidates in their order. `None` means 406 Not Acceptable.
pub fn choose(
    accept: &Accept,
    stored: (&str, &str),
    candidates: &[(&str, &str)],
) -> Option<(Choice, Packaging)> {
    let Some(ranges) = &accept.0 else {
        return Some((Choice::Stored, Packaging::Multipart));
    };
    let mut ranges: Vec<&MediaType> = ranges.iter().collect();
    // Stable, so ties keep the client's order.
    ranges.sort_by(|a, b| b.quality().total_cmp(&a.quality()));
    ranges.into_iter().find_map(|range| {
        if let Some(packaging) = range_accepts(range, stored.0, stored.1) {
            return Some((Choice::Stored, packaging));
        }
        candidates
            .iter()
            .enumerate()
            .find_map(|(i, (part_type, ts))| {
                range_accepts(range, part_type, ts).map(|p| (Choice::Candidate(i), p))
            })
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
    const JXL: &str = "1.2.840.10008.1.2.4.110";
    /// What a native frame can become, as `wado.rs` offers it.
    const FROM_NATIVE: [(&str, &str); 2] = [("image/jls", JLS), ("image/jxl", JXL)];

    /// How content is sent as stored, when nothing else is on offer.
    fn negotiate(accept: &Accept, part_type: &str, ts: &str) -> Option<Packaging> {
        choose(accept, (part_type, ts), &[]).map(|(_, packaging)| packaging)
    }

    fn negotiate_transcoded(
        accept: &Accept,
        stored: (&str, &str),
        candidates: &[(&str, &str)],
    ) -> Option<(usize, Packaging)> {
        match choose(accept, stored, candidates)? {
            (Choice::Candidate(i), packaging) => Some((i, packaging)),
            (Choice::Stored, _) => None,
        }
    }

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
        let stored = ("image/jls", JLS);
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
            negotiate_transcoded(&ranges, stored, &candidates),
            Some((2, Packaging::Multipart))
        );
        let ranges = Accept::parse(Some(
            r#"multipart/related; type="application/octet-stream"; transfer-syntax=1.2.840.10008.1.2.1"#,
        ));
        assert_eq!(
            negotiate_transcoded(&ranges, stored, &candidates),
            Some((0, Packaging::Multipart))
        );
        let ranges = Accept::parse(Some("image/png"));
        assert_eq!(negotiate_transcoded(&ranges, stored, &candidates), None);
        assert_eq!(
            negotiate_transcoded(&Accept::parse(None), stored, &candidates),
            None
        );
    }

    /// The bug this replaced: a later range that took the stored content beat an earlier
    /// one the server could also meet.
    #[test]
    fn the_first_range_wins_over_the_stored_content() {
        let native = ("application/octet-stream", ELE);
        let ranges = Accept::parse(Some(
            r#"multipart/related; type="image/jls"; transfer-syntax=1.2.840.10008.1.2.4.80, multipart/related; type="application/octet-stream"; transfer-syntax=*"#,
        ));
        assert_eq!(
            choose(&ranges, native, &FROM_NATIVE),
            Some((Choice::Candidate(0), Packaging::Multipart))
        );
        // Stored as asked first: no transcoding.
        assert_eq!(
            choose(&ranges, ("image/jls", JLS), &[native]),
            Some((Choice::Stored, Packaging::Multipart))
        );
        // Nothing to make it from (say, a codec the server cannot decode): the fallback.
        assert_eq!(
            choose(&ranges, native, &[]),
            Some((Choice::Stored, Packaging::Multipart))
        );
    }

    #[test]
    fn quality_before_order() {
        let native = ("application/octet-stream", ELE);
        let ranges = Accept::parse(Some(
            r#"multipart/related; type="application/octet-stream"; transfer-syntax=*; q=0.5, multipart/related; type="image/jxl", multipart/related; type="image/jls""#,
        ));
        assert_eq!(
            choose(&ranges, native, &FROM_NATIVE),
            Some((Choice::Candidate(1), Packaging::Multipart))
        );
        let ranges = Accept::parse(Some(
            r#"multipart/related; type="image/jls"; q=0.2, multipart/related; type="application/octet-stream"; q=0.9"#,
        ));
        assert_eq!(
            choose(&ranges, native, &FROM_NATIVE),
            Some((Choice::Stored, Packaging::Multipart))
        );
        // A quality out of range counts as its bound, not as a reason to fail.
        let ranges = Accept::parse(Some(
            r#"multipart/related; type="application/octet-stream"; q=7, multipart/related; type="image/jls""#,
        ));
        assert_eq!(
            choose(&ranges, native, &FROM_NATIVE),
            Some((Choice::Stored, Packaging::Multipart))
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
