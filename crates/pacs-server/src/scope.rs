//! The index as the request's account sees it, for the routes of this crate that read the
//! hierarchy (the study list, the measurements).
//!
//! As in the REST and DICOMweb crates: the routers' state is [`Whole`], which no handler
//! can open, and a handler takes [`Seen`], the index bounded to the account the guard
//! named in the request ([`SeenBy`]), or whole when it named none.

use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use pacs_index::{Index, SeenBy};

/// A router's state. Only [`Seen`] reads it.
#[derive(Debug, Clone)]
pub(crate) struct Whole(Index);

impl Whole {
    pub(crate) fn new(index: Index) -> Self {
        Whole(index)
    }
}

/// The index for one request, within what its account may see.
pub(crate) struct Seen(pub(crate) Index);

impl FromRequestParts<Whole> for Seen {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, whole: &Whole) -> Result<Self, Infallible> {
        Ok(Seen(match parts.extensions.get::<SeenBy>() {
            Some(SeenBy(account)) => whole.0.limited_to(account),
            None => whole.0.clone(),
        }))
    }
}
