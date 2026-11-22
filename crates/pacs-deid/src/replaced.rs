//! What a de-identification replaced, for a server that keeps the way back (write-back of
//! results to the original study): each replacement UID with the UID it replaced.

use std::collections::{BTreeMap, BTreeSet};

use dicom_core::VR;
use dicom_object::InMemDicomObject;

use crate::keys::Keys;

fn uids(object: &InMemDicomObject, out: &mut BTreeSet<String>) {
    for element in object.iter() {
        if let Some(items) = element.items() {
            for item in items {
                uids(item, out);
            }
        } else if element.vr() == VR::UI
            && let Ok(values) = element.to_multi_str()
        {
            out.extend(
                values
                    .iter()
                    .map(|v| v.trim_matches([' ', '\0']).to_owned())
                    .filter(|v| !v.is_empty()),
            );
        }
    }
}

/// Each UID of `deidentified` that replaces one of `original` (by `keys`), with the one it
/// replaced, ordered by the replacement. UIDs kept as they were are left out.
pub fn replaced_uids(
    original: &InMemDicomObject,
    deidentified: &InMemDicomObject,
    keys: &Keys,
) -> Vec<(String, String)> {
    let (mut before, mut after) = (BTreeSet::new(), BTreeSet::new());
    uids(original, &mut before);
    uids(deidentified, &mut after);
    let mut pairs = BTreeMap::new();
    for uid in before {
        let replacement = keys.uid(&uid);
        if replacement != uid && after.contains(&replacement) {
            pairs.insert(replacement, uid);
        }
    }
    pairs.into_iter().collect()
}
