//! The action codes of PS3.15 Table E.1-1a.

use pacs_deid::Action;

/// Table E.1-1a: compound actions, by default the choice that keeps an IOD valid; with
/// remove_when_allowed, X wherever it is a choice.
#[test]
fn compound_actions_resolve() {
    use Action::*;
    for (compound, valid, removing) in [
        (XZ, Z, X),
        (XD, D, X),
        (ZD, D, Z),
        (XZD, D, X),
        (XZU, U, X),
        (K, K, K),
    ] {
        assert_eq!(compound.resolve(false), valid, "{compound:?}");
        assert_eq!(compound.resolve(true), removing, "{compound:?}");
    }
}
