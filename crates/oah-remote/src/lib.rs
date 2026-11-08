//! Phones controlling the desktop app (the oah Android app).
//!
//! - **Pairing** ([`pairing`]): the desktop shows a one-time code, and a QR with the same code,
//!   its addresses and its public key. The phone and the desktop run SPAKE2 on the code, so an
//!   attacker on the network learns nothing beyond one wrong guess, and five wrong guesses close
//!   the code. Under the key SPAKE2 gives them, they swap long-term X25519 public keys.
//! - **Sessions** ([`wire`]): every later connection is a Noise `IK` handshake. The phone knows
//!   the desktop's key, the desktop only takes keys of paired phones, and each session has its
//!   own keys (forward secrecy). Traffic is plain `ws://`; everything inside it is encrypted,
//!   so a relay could carry the same bytes later.
//! - **RPC** ([`proto`]): JSON requests, responses and pushes (chat events) inside the session.
//!
//! [`server`] is the desktop's half (it calls a [`server::Host`]), [`client`] the phone's.

pub mod client;
pub mod pairing;
pub mod proto;
pub mod server;
pub mod wire;

use base64::Engine;

/// The Noise pattern for sessions.
pub const NOISE: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
/// Bumped when the protocol changes in a way an older phone or desktop can't follow.
pub const PROTOCOL: u32 = 1;
/// The WebSocket path the desktop serves.
pub const PATH: &str = "/oah";

/// A long-term X25519 key pair: the desktop's, or a phone's.
#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    pub private: [u8; 32],
    pub public: [u8; 32],
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Identity({})", b64(&self.public))
    }
}

impl Identity {
    pub fn generate() -> Identity {
        Identity::from_private(random())
    }

    /// From the private key alone (the public half is derived).
    pub fn from_private(private: [u8; 32]) -> Identity {
        let secret = x25519_dalek::StaticSecret::from(private);
        let public = x25519_dalek::PublicKey::from(&secret).to_bytes();
        Identity { private: secret.to_bytes(), public }
    }

    /// The private key as base64, for storing it.
    pub fn private_b64(&self) -> String {
        b64(&self.private)
    }

    pub fn from_private_b64(s: &str) -> Option<Identity> {
        key_from_b64(s).map(Identity::from_private)
    }
}

/// URL-safe base64 without padding (keys in QR codes, settings and files).
pub fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Either base64 alphabet, padded or not.
pub fn unb64(s: &str) -> Option<Vec<u8>> {
    let s = s.trim().trim_end_matches('=');
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(s)).ok()
}

pub fn key_from_b64(s: &str) -> Option<[u8; 32]> {
    unb64(s)?.try_into().ok()
}

/// `N` random bytes from the OS.
pub(crate) fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("OS randomness");
    b
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        let a = Identity::generate();
        assert_eq!(Identity::from_private(a.private), a);
        assert_eq!(Identity::from_private_b64(&a.private_b64()), Some(a.clone()));
        assert_eq!(key_from_b64(&b64(&a.public)), Some(a.public));
        assert_ne!(Identity::generate(), a);
    }
}
