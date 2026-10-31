//! De-identification of DICOM data sets by the Application Level Confidentiality Profiles of
//! DICOM PS3.15 Annex E (2026d): the Basic Profile of Table E.1-1, the Options a project
//! chooses, and the overrides it adds, so that each deployment decides what it keeps.
//!
//! Replacement UIDs, pseudonyms and date shifts come from a project secret ([`Keys`]), the
//! same on every instance and every run. Instances that may carry identification this crate
//! cannot remove (burned-in text, structured content, graphics) go to quarantine instead
//! ([`Quarantine`]). Nothing here touches the network or storage: pacs-rs's C-STORE path
//! uses it, and so can any other tool.

pub mod action;
pub mod table;

pub use action::Action;
