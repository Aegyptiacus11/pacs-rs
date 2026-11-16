//! The archive as the request's account sees it.
//!
//! The router's state is [`Whole`], which no handler can open: a handler takes [`Seen`]
//! instead, and gets the state with its archive bounded to the account the server named
//! in the request ([`SeenBy`]), or whole when it named none. So no handler reads the
//! archive without the bound.

use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use pacs_index::SeenBy;

use crate::AppState;

/// The router's state. Only [`Seen`] reads it.
#[derive(Debug, Clone)]
pub(crate) struct Whole(AppState);

impl Whole {
    pub(crate) fn new(state: AppState) -> Self {
        Whole(state)
    }
}

/// The state for one request, within what its account may see.
pub(crate) struct Seen(pub(crate) AppState);

impl FromRequestParts<Whole> for Seen {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, whole: &Whole) -> Result<Self, Infallible> {
        let mut state = whole.0.clone();
        if let Some(SeenBy(account)) = parts.extensions.get::<SeenBy>() {
            state.archive = state.archive.limited_to(account);
        }
        Ok(Seen(state))
    }
}
