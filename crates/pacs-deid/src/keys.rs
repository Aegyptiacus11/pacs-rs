//! Replacement values derived from a project's secret with HMAC-SHA-256, so that the same
//! original always gets the same replacement, on every instance and every pull, with no
//! table kept (PS3.15 E.1.1: replacement UIDs and dates consistent across related instances).
//! Without the secret a replacement cannot be computed from a guessed original, which a plain
//! hash would allow ("unencrypted hashes ... are vulnerable to trivial dictionary based
//! attacks", E.1.1).

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// A project's secret.
#[derive(Clone)]
pub struct Keys {
    secret: Vec<u8>,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys(..)")
    }
}

#[derive(Debug, thiserror::Error)]
#[error("a project secret needs at least 32 bytes, this one has {0}")]
pub struct ShortSecret(usize);

impl Keys {
    /// At least 32 bytes, random, kept as secret as the data it protects.
    pub fn new(secret: impl Into<Vec<u8>>) -> Result<Self, ShortSecret> {
        let secret = secret.into();
        if secret.len() < 32 {
            return Err(ShortSecret(secret.len()));
        }
        Ok(Keys { secret })
    }

    fn mac(&self, purpose: &str, value: &str) -> [u8; 32] {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.secret)
            .expect("HMAC takes keys of any length");
        mac.update(purpose.as_bytes());
        mac.update(&[0]);
        mac.update(value.as_bytes());
        mac.finalize().into_bytes().into()
    }

    /// The replacement of a UID: a UUID-derived UID (PS3.5 B.2), "2.25." and the decimal value
    /// of a version 8 (custom) UUID (RFC 9562) whose 122 free bits come from the HMAC.
    pub fn uid(&self, original: &str) -> String {
        let mac = self.mac("uid", original);
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&mac[..16]);
        bytes[6] = (bytes[6] & 0x0F) | 0x80; // version 8
        bytes[8] = (bytes[8] & 0x3F) | 0x80; // variant 10
        format!("2.25.{}", u128::from_be_bytes(bytes))
    }

    /// A patient's pseudonym: `prefix` and ten characters of Crockford base 32 (no I, L, O or
    /// U), 50 bits of the HMAC.
    pub fn pseudonym(&self, patient: &str, prefix: &str) -> String {
        const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
        let mac = self.mac("patient", patient);
        let mut bits = u64::from_be_bytes(mac[..8].try_into().expect("8 bytes"));
        let mut code = String::with_capacity(10);
        for _ in 0..10 {
            code.push(ALPHABET[(bits >> 59) as usize] as char);
            bits <<= 5;
        }
        format!("{prefix}{code}")
    }

    /// A patient's date shift, a whole number of days in `range` (both ends included).
    pub fn date_shift(&self, patient: &str, range: (i32, i32)) -> i32 {
        let (low, high) = (range.0.min(range.1), range.0.max(range.1));
        let span = (high as i64 - low as i64 + 1) as u64;
        let mac = self.mac("date", patient);
        let draw = u64::from_be_bytes(mac[..8].try_into().expect("8 bytes"));
        (low as i64 + (draw % span) as i64) as i32
    }
}
