//! Public identifiers of patients, studies, series and instances in the REST API.
//!
//! Each is the SHA-1 of the resource's DICOM identifiers joined by `|` - Patient ID, then
//! Study, Series and SOP Instance UID down to the resource's level - written as five
//! groups of eight hex digits. The same scheme as the reference server (checked against
//! its answers), so an identifier means the same resource in both.

use sha1::{Digest, Sha1};

fn hash(parts: &[&str]) -> String {
    let digest = Sha1::digest(parts.join("|").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    [
        &hex[0..8],
        &hex[8..16],
        &hex[16..24],
        &hex[24..32],
        &hex[32..40],
    ]
    .join("-")
}

pub fn patient(patient_id: &str) -> String {
    hash(&[patient_id])
}

pub fn study(patient_id: &str, study: &str) -> String {
    hash(&[patient_id, study])
}

pub fn series(patient_id: &str, study: &str, series: &str) -> String {
    hash(&[patient_id, study, series])
}

pub fn instance(patient_id: &str, study: &str, series: &str, instance: &str) -> String {
    hash(&[patient_id, study, series, instance])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Identifiers the reference server gave these UIDs.
    #[test]
    fn same_as_the_reference_server() {
        let (p, st, se, i) = (
            "SYN901",
            "2.25.901100000",
            "2.25.901200000",
            "2.25.901300001",
        );
        assert_eq!(
            series(p, st, se),
            "0727c1d7-c44bae92-1cf3e5dc-570fe392-acf56079"
        );
        assert_eq!(
            instance(p, st, se, i),
            "04cad042-b8cfdc54-2dc4e492-96e877f8-ae5a1541"
        );
        let (p, st) = ("SYN95", "2.25.95100000");
        assert_eq!(patient(p), "0976b31f-23df918c-e121e39f-1bc2d8ad-f5cac107");
        assert_eq!(study(p, st), "b1086121-db524880-674c51be-8a6d2a24-9fba2b61");
    }
}
