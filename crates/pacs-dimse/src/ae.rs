//! AE titles (PS3.5 Table 6.2-1, VR AE) and the remote AEs pacs-rs talks to.

use std::fmt;
use std::str::FromStr;

/// An AE title: 1 to 16 characters of the default repertoire, no backslash or control
/// characters, not all spaces; leading and trailing spaces are not significant, so they
/// are dropped here and two titles compare by what is left.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AeTitle(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AeError {
    #[error("an AE title cannot be empty or only spaces")]
    Empty,
    #[error("AE title {0:?} is longer than 16 characters")]
    TooLong(String),
    #[error("AE title {0:?} has a backslash, a control character or a non-ASCII character")]
    BadCharacter(String),
    #[error("a remote AE is written id=AE_TITLE@host:port, not {0:?}")]
    RemoteSyntax(String),
    #[error("remote AE {0:?}: the port must be a number from 1 to 65535")]
    Port(String),
}

impl AeTitle {
    pub fn new(text: &str) -> Result<Self, AeError> {
        let trimmed = text.trim_matches(' ');
        if trimmed.is_empty() {
            return Err(AeError::Empty);
        }
        if text.len() > 16 {
            return Err(AeError::TooLong(text.to_owned()));
        }
        if trimmed
            .chars()
            .any(|c| c == '\\' || !c.is_ascii() || c.is_ascii_control())
        {
            return Err(AeError::BadCharacter(text.to_owned()));
        }
        Ok(AeTitle(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `received`, as it came in a PDU, names this AE: trailing and leading spaces
    /// (and the NUL padding some peers send) aside.
    pub fn matches(&self, received: &str) -> bool {
        received.trim_matches([' ', '\0']) == self.0
    }
}

impl fmt::Display for AeTitle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AeTitle {
    type Err = AeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AeTitle::new(s)
    }
}

/// A remote AE pacs-rs can call: its AE title and network address, under an `id` the REST
/// API names it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAe {
    pub id: String,
    pub ae_title: AeTitle,
    pub host: String,
    pub port: u16,
}

impl FromStr for RemoteAe {
    type Err = AeError;

    /// `id=AE_TITLE@host:port`; an IPv6 host goes in brackets, `[::1]:104`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let syntax = || AeError::RemoteSyntax(s.to_owned());
        let (id, rest) = s.trim().split_once('=').ok_or_else(syntax)?;
        let (ae_title, address) = rest.rsplit_once('@').ok_or_else(syntax)?;
        let (host, port) = address.rsplit_once(':').ok_or_else(syntax)?;
        let host = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        let id = id.trim();
        if id.is_empty()
            || host.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(syntax());
        }
        let port = port
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or_else(|| AeError::Port(s.to_owned()))?;
        Ok(RemoteAe {
            id: id.to_owned(),
            ae_title: AeTitle::new(ae_title)?,
            host: host.to_owned(),
            port,
        })
    }
}
