//! Pairing a phone: the one-time code the desktop shows (the phone types it, with an address),
//! and the SPAKE2 exchange both sides run on it.
//!
//! On the wire (WebSocket binary frames, see [`crate::wire`]):
//! 1. phone → `PAIR` + SPAKE2 message; desktop → `PAIR` + its SPAKE2 message. Both now hold a
//!    key K that only matches if both used the same code.
//! 2. phone → `PAIR` + AEAD_K(`PhoneHello`). A frame that doesn't decrypt means a wrong code:
//!    the desktop counts the attempt and closes the connection.
//! 3. desktop → `PAIR` + AEAD_K(`DesktopHello`), with the phone's new device id. The code is
//!    used up; the phone connects again with a Noise session.

use std::time::{Duration, Instant};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use spake2::{Ed25519Group, Identity as SpakeId, Password, Spake2};

/// How long a code works.
pub const CODE_TTL: Duration = Duration::from_secs(5 * 60);
/// Wrong codes before the code closes.
pub const MAX_ATTEMPTS: u32 = 5;
/// Code length in characters (Crockford base32: 5 bits each, 40 bits).
pub const CODE_LEN: usize = 8;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const SPAKE_ID: &[u8] = b"oah-pair-v1";

/// A fresh code: 8 characters of Crockford base32.
pub fn new_code() -> String {
    let bytes: [u8; CODE_LEN] = crate::random();
    bytes.iter().map(|b| ALPHABET[(b & 31) as usize] as char).collect()
}

/// `K7QM9XTD` → `K7QM-9XTD`.
pub fn format_code(code: &str) -> String {
    let c = normalize_code(code);
    if c.len() == CODE_LEN {
        format!("{}-{}", &c[..4], &c[4..])
    } else {
        c
    }
}

/// What the user typed, as the code: upper case, dashes and spaces gone, and the letters people
/// confuse with digits read as those digits (O → 0, I and L → 1), the Crockford way.
pub fn normalize_code(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect()
}

/// The code the desktop is showing, while it lasts.
#[derive(Debug)]
pub struct Session {
    pub code: String,
    pub expires: Instant,
    /// Wall-clock expiry (ms), for the page's countdown.
    pub expires_at: u64,
    pub attempts: u32,
}

impl Session {
    pub fn new() -> Session {
        Session {
            code: new_code(),
            expires: Instant::now() + CODE_TTL,
            expires_at: crate::now_ms() + CODE_TTL.as_millis() as u64,
            attempts: 0,
        }
    }

    pub fn open(&self) -> bool {
        Instant::now() < self.expires && self.attempts < MAX_ATTEMPTS
    }
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

// ---- The exchange ----------------------------------------------------------------------------

/// The phone's first sealed message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneHello {
    pub name: String,
    pub public_key: String,
}

/// The desktop's answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopHello {
    pub device_id: String,
    pub desktop_name: String,
    pub public_key: String,
    pub protocol: u32,
}

/// One side of the SPAKE2 exchange on `code`: the message to send, and the state that turns the
/// other side's message into the shared key.
pub fn start(code: &str) -> (Spake2<Ed25519Group>, Vec<u8>) {
    Spake2::<Ed25519Group>::start_symmetric(&Password::new(normalize_code(code).as_bytes()), &SpakeId::new(SPAKE_ID))
}

pub fn finish(state: Spake2<Ed25519Group>, theirs: &[u8]) -> Result<[u8; 32], String> {
    let k = state.finish(theirs).map_err(|_| "the pairing message didn't read".to_string())?;
    k.try_into().map_err(|_| "unexpected pairing key length".to_string())
}

/// Who sealed a message: each direction has its own nonce, and each seals exactly one message.
#[derive(Clone, Copy)]
pub enum Dir {
    Phone = 1,
    Desktop = 2,
}

fn nonce(dir: Dir) -> Nonce {
    let mut n = [0u8; 12];
    n[0] = dir as u8;
    Nonce::from(n)
}

pub fn seal(key: &[u8; 32], dir: Dir, plaintext: &[u8]) -> Vec<u8> {
    ChaCha20Poly1305::new(Key::from_slice(key)).encrypt(&nonce(dir), Payload { msg: plaintext, aad: SPAKE_ID }).expect("sealing")
}

/// `None` when the key doesn't match (the codes differed) or the message was altered.
pub fn open(key: &[u8; 32], dir: Dir, sealed: &[u8]) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(Key::from_slice(key)).decrypt(&nonce(dir), Payload { msg: sealed, aad: SPAKE_ID }).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        let c = new_code();
        assert_eq!(c.len(), CODE_LEN);
        assert!(c.bytes().all(|b| ALPHABET.contains(&b)));
        assert_eq!(normalize_code("k7qm-9xtd"), "K7QM9XTD");
        assert_eq!(normalize_code(" o1L i-0 "), "01110");
        assert_eq!(format_code("k7qm9xtd"), "K7QM-9XTD");
        assert!(Session::new().open());
    }

    #[test]
    fn spake_needs_the_same_code() {
        let (a, ma) = start("K7QM-9XTD");
        let (b, mb) = start("k7qm9xtd");
        let (ka, kb) = (finish(a, &mb).unwrap(), finish(b, &ma).unwrap());
        assert_eq!(ka, kb);
        let sealed = seal(&ka, Dir::Phone, b"hi");
        assert_eq!(open(&kb, Dir::Phone, &sealed).as_deref(), Some(&b"hi"[..]));
        assert!(open(&kb, Dir::Desktop, &sealed).is_none(), "the other direction's nonce");

        let (a, ma) = start("K7QM9XTD");
        let (b, mb) = start("K7QM9XTE");
        let (ka, kb) = (finish(a, &mb).unwrap(), finish(b, &ma).unwrap());
        assert_ne!(ka, kb);
        assert!(open(&kb, Dir::Phone, &seal(&ka, Dir::Phone, b"hi")).is_none());
    }
}
