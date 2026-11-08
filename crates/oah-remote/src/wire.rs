//! What goes over the WebSocket: binary frames whose first byte says what they are, and the
//! Noise session that encrypts everything after the handshake.

use std::sync::Mutex;

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use crate::Identity;

/// A pairing step (see [`crate::pairing`]).
pub const PAIR: u8 = 1;
/// A Noise handshake message.
pub const HANDSHAKE: u8 = 2;
/// An encrypted chunk of a message.
pub const DATA: u8 = 3;
/// The sender is closing, and says why (UTF-8).
pub const ERROR: u8 = 0x7f;

const PROLOGUE: &[u8] = b"oah-remote-v1";
/// The most a Noise message can carry, less the tag and the "more" byte.
/// Noise's limits (snow keeps its constants private).
const MAXMSGLEN: usize = 65535;
const TAGLEN: usize = 16;
const CHUNK: usize = MAXMSGLEN - TAGLEN - 1;
/// The largest message (after reassembly) either side takes.
pub const MAX_MESSAGE: usize = 64 << 20;

/// The phone's half of an IK handshake with the desktop whose key it knows.
pub fn initiator(me: &Identity, desktop: &[u8; 32]) -> Result<snow::HandshakeState, String> {
    snow::Builder::new(crate::NOISE.parse().unwrap())
        .prologue(PROLOGUE)
        .and_then(|b| b.local_private_key(&me.private))
        .and_then(|b| b.remote_public_key(desktop))
        .and_then(|b| b.build_initiator())
        .map_err(|e| e.to_string())
}

pub fn responder(me: &Identity) -> Result<snow::HandshakeState, String> {
    snow::Builder::new(crate::NOISE.parse().unwrap())
        .prologue(PROLOGUE)
        .and_then(|b| b.local_private_key(&me.private))
        .and_then(|b| b.build_responder())
        .map_err(|e| e.to_string())
}

/// Writes a handshake message (with an optional payload).
pub fn hs_write(hs: &mut snow::HandshakeState, payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; MAXMSGLEN];
    let n = hs.write_message(payload, &mut buf).map_err(|e| e.to_string())?;
    buf.truncate(n);
    Ok(buf)
}

pub fn hs_read(hs: &mut snow::HandshakeState, msg: &[u8]) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; MAXMSGLEN];
    let n = hs.read_message(msg, &mut buf).map_err(|_| "the handshake didn't check out".to_string())?;
    buf.truncate(n);
    Ok(buf)
}

/// An established session: seals messages into `DATA` frames and opens them again.
pub struct Transport {
    state: Mutex<snow::TransportState>,
    partial: Mutex<Vec<u8>>,
}

impl Transport {
    pub fn new(hs: snow::HandshakeState) -> Result<Transport, String> {
        Ok(Transport { state: Mutex::new(hs.into_transport_mode().map_err(|e| e.to_string())?), partial: Mutex::new(Vec::new()) })
    }

    /// The frames (each `DATA` + ciphertext) that carry `msg`.
    pub fn seal(&self, msg: &[u8]) -> Vec<Vec<u8>> {
        let mut st = self.state.lock().unwrap();
        let chunks: Vec<&[u8]> = if msg.is_empty() { vec![&[][..]] } else { msg.chunks(CHUNK).collect() };
        let last = chunks.len() - 1;
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let mut plain = Vec::with_capacity(c.len() + 1);
                plain.push(u8::from(i != last));
                plain.extend_from_slice(c);
                let mut out = vec![0u8; plain.len() + TAGLEN + 1];
                out[0] = DATA;
                let n = st.write_message(&plain, &mut out[1..]).expect("sealing a chunk");
                out.truncate(n + 1);
                out
            })
            .collect()
    }

    /// Opens one `DATA` frame's body. Returns the whole message once its last chunk is in.
    pub fn open(&self, body: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let mut plain = vec![0u8; body.len()];
        let n = self.state.lock().unwrap().read_message(body, &mut plain).map_err(|_| "a message didn't decrypt".to_string())?;
        let Some((&more, data)) = plain[..n].split_first() else { return Err("an empty chunk".into()) };
        let mut partial = self.partial.lock().unwrap();
        if partial.len() + data.len() > MAX_MESSAGE {
            return Err("a message was too large".into());
        }
        partial.extend_from_slice(data);
        Ok(if more == 0 { Some(std::mem::take(&mut *partial)) } else { None })
    }
}

/// The next binary frame (kind, body). Pings and pongs are skipped; a close or a text frame
/// ends the connection; an `ERROR` frame becomes the error it carries.
pub async fn recv<S>(ws: &mut S) -> Result<(u8, Vec<u8>), String>
where
    S: Stream<Item = Result<Message, WsError>> + Unpin,
{
    loop {
        match ws.next().await {
            None | Some(Ok(Message::Close(_))) => return Err("the connection closed".into()),
            Some(Err(e)) => return Err(format!("the connection broke: {e}")),
            Some(Ok(Message::Binary(b))) => match b.split_first() {
                Some((&ERROR, msg)) => return Err(String::from_utf8_lossy(msg).into_owned()),
                Some((&k, body)) => return Ok((k, body.to_vec())),
                None => return Err("an empty frame".into()),
            },
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
            Some(Ok(Message::Text(_))) => return Err("unexpected text frame".into()),
        }
    }
}

pub async fn send<S>(ws: &mut S, kind: u8, body: &[u8]) -> Result<(), String>
where
    S: Sink<Message, Error = WsError> + Unpin,
{
    let mut f = Vec::with_capacity(body.len() + 1);
    f.push(kind);
    f.extend_from_slice(body);
    ws.send(Message::Binary(f.into())).await.map_err(|e| format!("the connection broke: {e}"))
}

/// Says why, then closes.
pub async fn fail<S>(ws: &mut S, why: &str)
where
    S: Sink<Message, Error = WsError> + Unpin,
{
    let _ = send(ws, ERROR, why.as_bytes()).await;
    let _ = ws.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Transport, Transport) {
        let (phone, desktop) = (Identity::generate(), Identity::generate());
        let mut i = initiator(&phone, &desktop.public).unwrap();
        let mut r = responder(&desktop).unwrap();
        let m1 = hs_write(&mut i, b"hello").unwrap();
        assert_eq!(hs_read(&mut r, &m1).unwrap(), b"hello");
        assert_eq!(r.get_remote_static().unwrap(), &phone.public);
        let m2 = hs_write(&mut r, &[]).unwrap();
        hs_read(&mut i, &m2).unwrap();
        (Transport::new(i).unwrap(), Transport::new(r).unwrap())
    }

    #[test]
    fn chunks_large_messages() {
        let (a, b) = pair();
        for len in [0, 10, CHUNK, CHUNK + 1, 3 * CHUNK + 7] {
            let msg: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let frames = a.seal(&msg);
            assert_eq!(frames.len(), len.div_ceil(CHUNK).max(1));
            let mut got = None;
            for (i, f) in frames.iter().enumerate() {
                assert_eq!(f[0], DATA);
                got = b.open(&f[1..]).unwrap();
                assert_eq!(got.is_some(), i == frames.len() - 1);
            }
            assert_eq!(got.unwrap(), msg);
        }
        // And back the other way.
        assert_eq!(a.open(&b.seal(b"ok")[0][1..]).unwrap().unwrap(), b"ok");
    }

    #[test]
    fn rejects_tampering_and_the_wrong_key() {
        let (a, b) = pair();
        let mut f = a.seal(b"secret").remove(0);
        *f.last_mut().unwrap() ^= 1;
        assert!(b.open(&f[1..]).is_err());

        let (phone, desktop, other) = (Identity::generate(), Identity::generate(), Identity::generate());
        let mut i = initiator(&phone, &other.public).unwrap();
        let mut r = responder(&desktop).unwrap();
        let m1 = hs_write(&mut i, &[]).unwrap();
        assert!(hs_read(&mut r, &m1).is_err(), "IK to a different desktop key fails");
    }
}
