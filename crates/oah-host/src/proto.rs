//! What oah and `oah-host` say to each other over the SSH connection. Every message is a frame:
//! a 4-byte big-endian length, then a tag byte and the fields (integers big-endian, bytes and
//! strings with a 4-byte length first).
//!
//! Both sides count every byte of a program's stdin, stdout and stderr, so a connection that
//! drops loses nothing: on reconnecting oah says how much output it has (`Attach`) and sends the
//! stdin the host hasn't acknowledged again; the host skips what it already has.

use std::io::{self, Read};

/// Bumped when a message changes. The binary's name follows its contents, so both ends of one
/// connection always come from the same build; this only guards against a hand-copied binary.
pub const PROTO: u32 = 1;

/// What `oah-host proxy` prints before its first frame. Anything before it (a login script that
/// prints to stdout) is skipped.
pub const MAGIC: &[u8] = b"\0oah-host-1\n";

/// The most data one frame carries; longer writes are split.
pub const CHUNK: usize = 64 * 1024;

/// The longest script a `Spawn` carries.
pub const MAX_SCRIPT: usize = 256 * 1024;

/// A frame bigger than this is a broken stream, not a message.
const MAX_FRAME: usize = MAX_SCRIPT + 4096;

pub const STDOUT: u8 = 1;
pub const STDERR: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// From `oah-host proxy` to the daemon, first on each connection: the SSH session's agent
    /// socket (empty without agent forwarding).
    Proxy {
        auth_sock: String,
    },

    // ---- oah → host
    /// `client` names one oah install and `instance` one run of it; a new instance of the same
    /// install ends what the old one left behind (it can never reattach to it).
    Hello {
        proto: u32,
        client: u64,
        instance: u64,
    },
    /// Run `sh -lc <script>` in a new session (process group). Sent again for the same `id` when
    /// the connection dropped before `Spawned` came back; it isn't started twice.
    Spawn {
        id: u64,
        script: String,
    },
    /// Stdin from byte `off` on.
    Stdin {
        id: u64,
        off: u64,
        data: Vec<u8>,
    },
    /// Close stdin once `off` bytes have been written to it.
    CloseStdin {
        id: u64,
        off: u64,
    },
    /// After a reconnect: send output from these offsets on, and the exit status again.
    Attach {
        id: u64,
        out: u64,
        err: u64,
    },
    /// oah has the output up to these offsets; the host can drop it.
    Ack {
        id: u64,
        out: u64,
        err: u64,
    },
    /// SIGKILL its whole process group.
    Kill {
        id: u64,
    },
    /// oah is done with it: stdin is closed, and two seconds later the group is killed.
    Forget {
        id: u64,
    },
    Ping {
        n: u64,
    },

    // ---- host → oah
    HelloOk {
        proto: u32,
        daemon: u64,
    },
    Spawned {
        id: u64,
    },
    SpawnFailed {
        id: u64,
        msg: String,
    },
    Out {
        id: u64,
        stream: u8,
        off: u64,
        data: Vec<u8>,
    },
    /// The stream ended after `off` bytes.
    Eof {
        id: u64,
        stream: u8,
        off: u64,
    },
    /// The host has stdin up to `off`.
    StdinAck {
        id: u64,
        off: u64,
    },
    Exited {
        id: u64,
        code: Option<i32>,
        signal: Option<i32>,
    },
    /// The host doesn't know this id (its daemon restarted, or it was forgotten), or has
    /// heard a `Forget` for it.
    Gone {
        id: u64,
    },
    Pong {
        n: u64,
    },
}

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(vec![0, 0, 0, 0]);
        match self {
            Msg::Proxy { auth_sock } => w.u8(1).str(auth_sock),
            Msg::Hello { proto, client, instance } => w.u8(2).u32(*proto).u64(*client).u64(*instance),
            Msg::Spawn { id, script } => w.u8(3).u64(*id).str(script),
            Msg::Stdin { id, off, data } => w.u8(4).u64(*id).u64(*off).bytes(data),
            Msg::CloseStdin { id, off } => w.u8(5).u64(*id).u64(*off),
            Msg::Attach { id, out, err } => w.u8(6).u64(*id).u64(*out).u64(*err),
            Msg::Ack { id, out, err } => w.u8(7).u64(*id).u64(*out).u64(*err),
            Msg::Kill { id } => w.u8(8).u64(*id),
            Msg::Forget { id } => w.u8(9).u64(*id),
            Msg::Ping { n } => w.u8(10).u64(*n),
            Msg::HelloOk { proto, daemon } => w.u8(101).u32(*proto).u64(*daemon),
            Msg::Spawned { id } => w.u8(102).u64(*id),
            Msg::SpawnFailed { id, msg } => w.u8(103).u64(*id).str(msg),
            Msg::Out { id, stream, off, data } => w.u8(104).u64(*id).u8(*stream).u64(*off).bytes(data),
            Msg::Eof { id, stream, off } => w.u8(105).u64(*id).u8(*stream).u64(*off),
            Msg::StdinAck { id, off } => w.u8(106).u64(*id).u64(*off),
            Msg::Exited { id, code, signal } => w.u8(107).u64(*id).opt(*code).opt(*signal),
            Msg::Gone { id } => w.u8(108).u64(*id),
            Msg::Pong { n } => w.u8(109).u64(*n),
        };
        let len = (w.0.len() - 4) as u32;
        w.0[..4].copy_from_slice(&len.to_be_bytes());
        w.0
    }

    fn decode(b: &[u8]) -> io::Result<Msg> {
        let mut r = R(b);
        let m = match r.u8()? {
            1 => Msg::Proxy { auth_sock: r.str()? },
            2 => Msg::Hello { proto: r.u32()?, client: r.u64()?, instance: r.u64()? },
            3 => Msg::Spawn { id: r.u64()?, script: r.str()? },
            4 => Msg::Stdin { id: r.u64()?, off: r.u64()?, data: r.bytes()? },
            5 => Msg::CloseStdin { id: r.u64()?, off: r.u64()? },
            6 => Msg::Attach { id: r.u64()?, out: r.u64()?, err: r.u64()? },
            7 => Msg::Ack { id: r.u64()?, out: r.u64()?, err: r.u64()? },
            8 => Msg::Kill { id: r.u64()? },
            9 => Msg::Forget { id: r.u64()? },
            10 => Msg::Ping { n: r.u64()? },
            101 => Msg::HelloOk { proto: r.u32()?, daemon: r.u64()? },
            102 => Msg::Spawned { id: r.u64()? },
            103 => Msg::SpawnFailed { id: r.u64()?, msg: r.str()? },
            104 => Msg::Out { id: r.u64()?, stream: r.u8()?, off: r.u64()?, data: r.bytes()? },
            105 => Msg::Eof { id: r.u64()?, stream: r.u8()?, off: r.u64()? },
            106 => Msg::StdinAck { id: r.u64()?, off: r.u64()? },
            107 => Msg::Exited { id: r.u64()?, code: r.opt()?, signal: r.opt()? },
            108 => Msg::Gone { id: r.u64()? },
            109 => Msg::Pong { n: r.u64()? },
            t => return Err(bad(&format!("unknown message {t}"))),
        };
        if !r.0.is_empty() {
            return Err(bad("trailing bytes in a message"));
        }
        Ok(m)
    }
}

/// The next frame, or `None` when the stream ended cleanly between frames.
pub fn read(r: &mut impl Read) -> io::Result<Option<Msg>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(bad(&format!("a {len}-byte frame")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Msg::decode(&body).map(Some)
}

/// Reads up to and including [`MAGIC`], giving up after `limit` bytes. Returns what came before
/// it (for an error message when the helper didn't start).
pub fn skip_to_magic(r: &mut impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut seen = Vec::new();
    let mut b = [0u8; 1];
    loop {
        if r.read(&mut b)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, String::from_utf8_lossy(&seen).into_owned()));
        }
        seen.push(b[0]);
        if seen.ends_with(MAGIC) {
            seen.truncate(seen.len() - MAGIC.len());
            return Ok(seen);
        }
        if seen.len() > limit {
            return Err(bad("no greeting from oah-host"));
        }
    }
}

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
        self
    }
    fn str(&mut self, v: &str) -> &mut Self {
        self.bytes(v.as_bytes())
    }
    fn opt(&mut self, v: Option<i32>) -> &mut Self {
        match v {
            Some(v) => self.u8(1).u32(v as u32),
            None => self.u8(0),
        }
    }
}

struct R<'a>(&'a [u8]);

impl R<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.0.len() < n {
            return Err(bad("a message ended early"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| bad("a string that isn't UTF-8"))
    }
    fn opt(&mut self) -> io::Result<Option<i32>> {
        Ok(match self.u8()? {
            0 => None,
            _ => Some(self.u32()? as i32),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_round_trips() {
        let all = [
            Msg::Proxy { auth_sock: "/tmp/ssh-x/agent.1".into() },
            Msg::Hello { proto: PROTO, client: 7, instance: u64::MAX },
            Msg::Spawn { id: 1, script: "cd ~ && exec 'claude'".into() },
            Msg::Stdin { id: 1, off: 42, data: b"{\"a\":1}\n".to_vec() },
            Msg::CloseStdin { id: 1, off: 50 },
            Msg::Attach { id: 1, out: 3, err: 4 },
            Msg::Ack { id: 1, out: 5, err: 6 },
            Msg::Kill { id: 1 },
            Msg::Forget { id: 1 },
            Msg::Ping { n: 9 },
            Msg::HelloOk { proto: PROTO, daemon: 3 },
            Msg::Spawned { id: 2 },
            Msg::SpawnFailed { id: 2, msg: "no sh".into() },
            Msg::Out { id: 2, stream: STDERR, off: 0, data: vec![0, 255, 10] },
            Msg::Eof { id: 2, stream: STDOUT, off: 10 },
            Msg::StdinAck { id: 2, off: 8 },
            Msg::Exited { id: 2, code: Some(-1), signal: None },
            Msg::Exited { id: 2, code: None, signal: Some(9) },
            Msg::Gone { id: 2 },
            Msg::Pong { n: 9 },
        ];
        let mut stream = Vec::new();
        for m in &all {
            stream.extend(m.encode());
        }
        let mut r = &stream[..];
        for m in &all {
            assert_eq!(read(&mut r).unwrap().as_ref(), Some(m));
        }
        assert_eq!(read(&mut r).unwrap(), None);
    }

    #[test]
    fn junk_before_the_greeting_is_skipped() {
        let mut s = b"Welcome to the box!\n".to_vec();
        s.extend_from_slice(MAGIC);
        s.extend(Msg::Pong { n: 1 }.encode());
        let mut r = &s[..];
        assert_eq!(skip_to_magic(&mut r, 1000).unwrap(), b"Welcome to the box!\n");
        assert_eq!(read(&mut r).unwrap(), Some(Msg::Pong { n: 1 }));
    }

    #[test]
    fn broken_frames_are_errors() {
        assert!(read(&mut &[0u8, 0, 0, 0][..]).is_err());
        assert!(read(&mut &[0u8, 0xff, 0xff, 0xff][..]).is_err());
        assert!(read(&mut &[0u8, 0, 0, 2, 250, 0][..]).is_err());
        assert!(read(&mut &[0u8, 0][..]).is_err());
    }
}
