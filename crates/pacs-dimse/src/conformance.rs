//! The DICOM Conformance Statement (PS3.2) of pacs-rs's DIMSE side, written from what the
//! code negotiates: the services of the SCP's registry and their SOP classes, the transfer
//! syntaxes the build reads, what each SCU proposes, the keys of C-FIND. The prose around them
//! is the behaviour the code has and its tests hold. `cargo run -p pacs-dimse --example
//! conformance` writes `docs/conformance.md`; a test fails when the file is not what this
//! writes.

use std::fmt::Write;

use dicom_core::dictionary::DataDictionary;
use dicom_dictionary_std::StandardDataDictionary;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;

use crate::query::{
    PATIENT_ROOT_FIND, PATIENT_ROOT_GET, PATIENT_ROOT_MOVE, STUDY_ROOT_FIND, STUDY_ROOT_GET,
};
use crate::scp::Service;
use crate::scu::retrieve::STUDY_ROOT_MOVE;
use crate::{
    EXPLICIT_VR_LE, IMPLEMENTATION_CLASS_UID, IMPLEMENTATION_VERSION_NAME, IMPLICIT_VR_LE,
    VERIFICATION,
};

/// The name of a SOP class pacs-rs serves.
fn sop_name(uid: &str) -> &'static str {
    match uid {
        VERIFICATION => "Verification",
        STUDY_ROOT_FIND => "Study Root Query/Retrieve Information Model - FIND",
        STUDY_ROOT_MOVE => "Study Root Query/Retrieve Information Model - MOVE",
        STUDY_ROOT_GET => "Study Root Query/Retrieve Information Model - GET",
        PATIENT_ROOT_FIND => "Patient Root Query/Retrieve Information Model - FIND",
        PATIENT_ROOT_MOVE => "Patient Root Query/Retrieve Information Model - MOVE",
        PATIENT_ROOT_GET => "Patient Root Query/Retrieve Information Model - GET",
        _ => crate::sop_classes::STORAGE
            .iter()
            .find(|(u, _)| *u == uid)
            .map_or("?", |(_, name)| *name),
    }
}

/// The transfer syntaxes this build reads and writes: those the SCP accepts.
fn transfer_syntaxes() -> Vec<(String, String)> {
    let mut syntaxes: Vec<(String, String)> = TransferSyntaxRegistry
        .iter()
        .filter(|ts| !ts.is_unsupported())
        .map(|ts| (ts.uid().to_owned(), ts.name().to_owned()))
        .collect();
    syntaxes.sort();
    syntaxes.dedup();
    syntaxes
}

fn ts_name(uid: &str) -> String {
    TransferSyntaxRegistry
        .get(uid)
        .map_or_else(|| uid.to_owned(), |ts| ts.name().to_owned())
}

fn keyword(tag: dicom_core::Tag) -> String {
    StandardDataDictionary
        .by_tag(tag)
        .map_or_else(|| format!("{tag}"), |entry| entry.alias.to_owned())
}

/// The statement, as Markdown.
pub fn statement() -> String {
    let mut out = String::new();
    let w = &mut out;
    let line = |w: &mut String, text: &str| {
        w.push_str(text);
        w.push('\n');
    };
    line(w, "# Conformance Statement");
    line(w, "");
    line(
        w,
        "The DICOM Conformance Statement of pacs-rs's network services (PS3.2), written by the code",
    );
    line(
        w,
        "from what it negotiates (`cargo run -p pacs-dimse --example conformance`): a test fails when",
    );
    line(
        w,
        "this page and the code part. DICOMweb is described in [DICOMweb](dicomweb.md) and the",
    );
    line(
        w,
        "[reference](api-reference.md#dicomweb); what each service does, in [DIMSE](dimse.md).",
    );
    line(w, "");
    line(w, "## Implementation");
    line(w, "");
    line(w, "| | |");
    line(w, "|---|---|");
    let _ = writeln!(
        w,
        "| Implementation Class UID | `{IMPLEMENTATION_CLASS_UID}`, in the files pacs-rs writes |"
    );
    let _ = writeln!(
        w,
        "| Implementation Version Name | `{IMPLEMENTATION_VERSION_NAME}` |"
    );
    line(
        w,
        "| On associations | the Implementation Class UID and Version Name of dicom-ul, the library pacs-rs negotiates with, which does not let them be set |",
    );
    let _ = writeln!(
        w,
        "| Maximum PDU length | {} bytes received; what the peer says sent |",
        dicom_ul::pdu::DEFAULT_MAX_PDU
    );
    line(
        w,
        "| Associations served at once | `PACS_DIMSE_MAX_ASSOCIATIONS`, 16 unless set; one more is rejected as transient, local limit exceeded |",
    );
    line(
        w,
        "| Asynchronous operations | none: one operation at a time on an association |",
    );
    line(w, "");
    line(w, "## Network services");
    line(w, "");
    line(w, "| SOP Class | UID | SCU | SCP |");
    line(w, "|---|---|---|---|");
    for (service, scu) in [
        (Service::Verification, true),
        (Service::Find, true),
        (Service::Move, true),
        (Service::Get, true),
    ] {
        for uid in service.sop_classes() {
            let _ = writeln!(
                w,
                "| {} | `{uid}` | {} | yes |",
                sop_name(uid),
                if scu { "yes" } else { "no" }
            );
        }
    }
    let storage = Service::Storage.sop_classes();
    let _ = writeln!(
        w,
        "| The {} Storage SOP Classes of PS3.4 Table B.5-1, [listed below](#storage-sop-classes) | | yes | yes |",
        storage.len()
    );
    line(w, "");
    line(w, "## Application entities");
    line(w, "");
    line(
        w,
        "One listener (`PACS_DIMSE_LISTEN`) answers pacs-rs's AE title (`PACS_DIMSE_AE_TITLE`, `PACSRS`",
    );
    line(
        w,
        "unless set), any further ones (`PACS_DIMSE_EXTRA_AE_TITLES`) and each project's; another called",
    );
    line(
        w,
        "AE title is rejected (PS3.8, reason 7). With `PACS_DIMSE_KNOWN_CALLERS_ONLY`, so is a calling AE",
    );
    line(
        w,
        "title not among the remote AEs (reason 3). Each AE title offers every service below unless",
    );
    line(
        w,
        "`PACS_DIMSE_CAPABILITIES` gives it its own services and transfer syntaxes; over TCP these are",
    );
    line(
        w,
        "what it negotiates, over TLS they hold per operation (status `0122`).",
    );
    line(w, "");
    line(w, "### Accepted presentation contexts");
    line(w, "");
    line(
        w,
        "Every abstract syntax of the services, in the first transfer syntax the requester proposes",
    );
    line(w, "among these, which this build reads:");
    line(w, "");
    line(w, "| Transfer syntax | UID |");
    line(w, "|---|---|");
    for (uid, name) in transfer_syntaxes() {
        let _ = writeln!(w, "| {name} | `{uid}` |");
    }
    line(w, "");
    line(w, "| Service | Role | Extended negotiation |");
    line(w, "|---|---|---|");
    line(w, "| Verification | SCP | none |");
    line(
        w,
        "| Storage | SCP; SCU too for a C-GET's requester, whose SCP/SCU Role Selection (PS3.7 D.3.3.4) is accepted as proposed | none |",
    );
    line(
        w,
        "| FIND, MOVE, GET of both models | SCP | none: relational queries, combined date and time, fuzzy, empty value and multiple value matching are not negotiated |",
    );
    line(w, "");
    line(w, "### Proposed presentation contexts");
    line(w, "");
    let plain = format!("{}, {}", ts_name(EXPLICIT_VR_LE), ts_name(IMPLICIT_VR_LE));
    let _ = writeln!(
        w,
        "| Operation | Abstract syntaxes | Transfer syntaxes | Role |"
    );
    line(w, "|---|---|---|---|");
    let _ = writeln!(
        w,
        "| C-ECHO | Verification | {} | SCU |",
        ts_name(IMPLICIT_VR_LE)
    );
    let _ = writeln!(
        w,
        "| C-FIND | Study Root FIND, Patient Root FIND: Study Root used when accepted | {plain} | SCU |"
    );
    let _ = writeln!(
        w,
        "| C-MOVE | Study Root MOVE, Patient Root MOVE: Study Root used when accepted | {plain} | SCU |"
    );
    let images: Vec<String> = crate::scu::get::IMAGES_AS
        .iter()
        .filter(|ts| TransferSyntaxRegistry.get(ts).is_some())
        .map(|ts| ts_name(ts))
        .collect();
    let _ = writeln!(
        w,
        "| C-GET | Study Root GET, Patient Root GET; and {} Storage SOP Classes, the images most held first ({}) | {plain} for the GET models and most storage classes; the images also {} | SCU of GET, SCP of storage by role selection |",
        crate::scu::get::proposed_storage().len(),
        crate::scu::get::IMAGES.len(),
        images[2..].join(", ")
    );
    let _ = writeln!(
        w,
        "| C-STORE (sending, a retrieval's sub-operations) | the SOP class of each instance | the one it is stored in, and {plain}; decoded to an uncompressed one when the peer takes only that | SCU |"
    );
    line(w, "");
    line(
        w,
        "At most 128 presentation contexts on an association (PS3.8 9.3.2.2).",
    );
    line(w, "");
    line(w, "## SOP specific conformance");
    line(w, "");
    line(w, "### Storage (SCP)");
    line(w, "");
    line(
        w,
        "Each data set is kept as received, with the file meta information added, then stored, compressed",
    );
    line(
        w,
        "losslessly when it came uncompressed (JPEG-LS, or deflated frames beyond 16 bits; `PACS_COMPRESSION`),",
    );
    line(
        w,
        "and indexed. Level 2 (Full) storage: every element is kept. An instance sent again is kept as",
    );
    line(
        w,
        "first stored unless `PACS_OVERWRITE_INSTANCES` is set, and answered Success either way.",
    );
    line(w, "");
    line(w, "| Status | Meaning |");
    line(w, "|---|---|");
    for (code, meaning) in [
        ("0000", "stored, or already stored"),
        (
            "A700, A701",
            "Refused: Out of resources: the store, or the index, did not take it",
        ),
        (
            "A702",
            "Refused: Out of resources: larger than `PACS_MAX_UPLOAD_BYTES`",
        ),
        ("A900", "Data Set does not match SOP Class"),
        (
            "C000",
            "Cannot understand: the data set cannot be read, or lacks a UID",
        ),
        (
            "C001",
            "SOP Instance UID is not the Affected SOP Instance UID",
        ),
        ("C002", "the UIDs contradict the archive"),
        (
            "C003, C004",
            "a project refused or could not de-identify it",
        ),
        (
            "0122",
            "SOP Class not supported, or not by the AE title called",
        ),
    ] {
        let _ = writeln!(w, "| `{code}` | {meaning} |");
    }
    line(w, "");
    line(w, "### Query (C-FIND SCP)");
    line(w, "");
    line(
        w,
        "Hierarchical queries at every level of both models (PS3.4 C.6.1, C.6.2), matched by PS3.4",
    );
    line(
        w,
        "C.2.2.2: single value, wild card, list of UIDs, range, sequence; person names without regard to",
    );
    line(
        w,
        "case. Answered to the calling AE titles of `PACS_DIMSE_REMOTES`; another is refused `0124`.",
    );
    line(w, "The keys matched and returned:");
    line(w, "");
    line(w, "| Model | Level | Keys |");
    line(w, "|---|---|---|");
    for (model, level, keys) in crate::scp::find::keys() {
        let keys: Vec<String> = keys
            .iter()
            .map(|(tag, vr, items)| {
                let mut text = format!("{} ({vr})", keyword(*tag));
                if !items.is_empty() {
                    let inner: Vec<String> = items.iter().map(|t| keyword(*t)).collect();
                    let _ = write!(text, " > {}", inner.join(", "));
                }
                text
            })
            .collect();
        let _ = writeln!(w, "| {model} | {level} | {} |", keys.join(", "));
    }
    line(w, "");
    line(w, "### Retrieve (C-MOVE and C-GET SCP)");
    line(w, "");
    line(
        w,
        "At every level of both models. A C-MOVE sends to the remote AEs of",
    );
    line(
        w,
        "`PACS_DIMSE_MOVE_DESTINATIONS` only, refusing any other with `A801`; a C-GET sends on the",
    );
    line(
        w,
        "association that asks. Sub-operations are counted in Pending responses; a C-CANCEL stops",
    );
    line(
        w,
        "before the next instance. Final statuses: `0000`, `B000` (some failed or warned), `FE00`",
    );
    line(
        w,
        "(cancelled), `A702` (none could be sent), `A801`, `A900`, `0124` (a caller not answered).",
    );
    line(w, "");
    line(w, "## Communication profiles");
    line(w, "");
    line(
        w,
        "TCP/IP (PS3.8). With `PACS_DIMSE_TLS_CERT_FILE` and `PACS_DIMSE_TLS_KEY_FILE` the listener",
    );
    line(
        w,
        "speaks TLS only, by the BCP 195 profile of PS3.15 B.12, and asks callers for a certificate",
    );
    line(
        w,
        "unless told not to; remote AEs marked `/tls` are called over TLS.",
    );
    line(w, "");
    line(w, "## Security");
    line(w, "");
    line(
        w,
        "Every association, search, retrieval and storage is written to the audit trail (PS3.15 A.5)",
    );
    line(
        w,
        "when `PACS_AUDIT_FILE` is set. Which calling AE titles are answered is configured (above).",
    );
    line(w, "");
    line(w, "## Character sets");
    line(w, "");
    line(
        w,
        "Data sets are kept in the Specific Character Set they came in. Identifiers pacs-rs writes are",
    );
    line(w, "in ISO_IR 192 (UTF-8) when a value is not ASCII.");
    line(w, "");
    line(w, "## Storage SOP Classes");
    line(w, "");
    line(w, "| SOP Class | UID |");
    line(w, "|---|---|");
    for uid in storage {
        let _ = writeln!(w, "| {} | `{uid}` |", sop_name(uid));
    }
    out
}
