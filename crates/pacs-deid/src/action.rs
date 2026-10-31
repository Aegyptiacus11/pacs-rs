//! The action codes of PS3.15 Table E.1-1a.

use serde::Deserialize;

/// What to do with an attribute (PS3.15 Table E.1-1a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Action {
    /// Replace with a non-zero length value, which may be a dummy, consistent with the VR.
    D,
    /// Replace with a zero length value (or a dummy).
    Z,
    /// Remove; for a sequence, with all its items.
    X,
    /// Keep; a sequence's items still go through the profile.
    K,
    /// Clean: replace with values of similar meaning known not to identify.
    C,
    /// Replace with a non-zero length UID, consistent within a set of instances.
    U,
    /// Z unless D is required by the IOD (Type 2 versus Type 1).
    #[serde(rename = "Z/D")]
    ZD,
    /// X unless Z is required (Type 3 versus Type 2).
    #[serde(rename = "X/Z")]
    XZ,
    /// X unless D is required (Type 3 versus Type 1).
    #[serde(rename = "X/D")]
    XD,
    /// X unless Z or D is required.
    #[serde(rename = "X/Z/D")]
    XZD,
    /// X unless Z or replacing the contained instance UIDs is required (sequences).
    #[serde(rename = "X/Z/U*")]
    XZU,
}

impl Action {
    /// A single action for a compound one. pacs-rs does not know the Type an attribute has in
    /// the instance's IOD, so by default it takes the choice that keeps every IOD valid (Z for
    /// X/Z, D where D is a choice, U for X/Z/U*); `remove_when_allowed` takes X wherever X is
    /// a choice, which leaves less behind but may make a Type 1 or 2 attribute missing.
    pub fn resolve(self, remove_when_allowed: bool) -> Action {
        use Action::*;
        match (self, remove_when_allowed) {
            (XZ | XD | XZD | XZU, true) => X,
            (ZD, true) => Z,
            (XZ, false) => Z,
            (ZD | XD | XZD, false) => D,
            (XZU, false) => U,
            (single, _) => single,
        }
    }
}
