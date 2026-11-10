//! `oah-host`: the small helper oah copies to an SSH host (to `~/.oah/bin/oah-host-<tag>`, the
//! tag following the binary's contents). oah keeps one SSH connection per host, running
//! `oah-host proxy`, which joins a per-user daemon (`oah-host serve`) on a Unix socket. The daemon
//! runs the agents and commands, keeps their output until oah has it, and outlives the
//! connection: a laptop that sleeps or changes networks reconnects to agents that kept working.
//!
//! The daemon ends a program's process group when oah is done with it, when a newer run of the
//! same oah install connects (the old run can't come back for it), or after it has had no
//! connection for [`DETACHED_LIMIT`]. With nothing left it exits by itself.

pub mod proto;

/// How long a program may go without a connection before the daemon ends it (and how long oah
/// keeps trying to reconnect): a night with the laptop shut, not a week.
pub const DETACHED_LIMIT: std::time::Duration = std::time::Duration::from_secs(12 * 3600);

#[cfg(unix)]
pub mod daemon;
#[cfg(unix)]
pub mod proxy;

/// A tag is what `oah-host-<tag>` and its socket are named by: letters, digits, `-` and `_`.
pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 40 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A short, stable name for a binary's contents: FNV-1a, 64 bits, in hex. It only tells builds
/// apart (the SSH connection already guards the copy), so it needn't be cryptographic.
pub fn tag_of(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(unix)]
pub use paths::{paths, Paths};

#[cfg(unix)]
mod paths {
    use std::io;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    /// Where one tag's daemon keeps its socket, lock, log and agent link.
    #[derive(Debug, Clone)]
    pub struct Paths {
        pub dir: PathBuf,
        pub sock: PathBuf,
        pub lock: PathBuf,
        pub log: PathBuf,
        /// A link to the newest connection's forwarded SSH agent, so programs started by an
        /// earlier connection can still use it (`SSH_AUTH_SOCK` points here).
        pub agent: PathBuf,
    }

    /// `~/.oah/run/<machine>`, or `/tmp/oah-<uid>` when that path is too long for a socket.
    /// Per machine, since a home folder can be shared (NFS) while sockets and locks belong to
    /// one machine. The folder is private to this user; one that isn't is refused.
    pub fn paths(tag: &str) -> io::Result<Paths> {
        if !crate::valid_tag(tag) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not a valid tag: {tag:?}")));
        }
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).ok_or_else(|| io::Error::other("HOME isn't set"))?;
        let mut dir = Path::new(&home).join(".oah").join("run").join(machine());
        // A socket's path has to fit in sun_path (108 bytes on Linux, 104 on macOS).
        if dir.join(format!("{tag}.agent")).as_os_str().len() > 100 {
            // SAFETY: geteuid can't fail.
            dir = PathBuf::from(format!("/tmp/oah-{}", unsafe { libc::geteuid() }));
        }
        private_dir(&dir)?;
        Ok(Paths {
            sock: dir.join(format!("{tag}.sock")),
            lock: dir.join(format!("{tag}.lock")),
            log: dir.join(format!("{tag}.log")),
            agent: dir.join(format!("{tag}.agent")),
            dir,
        })
    }

    /// This machine's name, as a safe folder name.
    fn machine() -> String {
        let mut buf = [0u8; 256];
        // SAFETY: gethostname writes at most buf.len() bytes into buf.
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        let name = if ok {
            String::from_utf8_lossy(&buf[..buf.iter().position(|b| *b == 0).unwrap_or(buf.len())]).into_owned()
        } else {
            String::new()
        };
        let name: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).take(64).collect();
        if name.is_empty() || name.starts_with('.') {
            "host".into()
        } else {
            name
        }
    }

    fn private_dir(dir: &Path) -> io::Result<()> {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        let meta = std::fs::symlink_metadata(dir)?;
        // SAFETY: geteuid can't fail.
        let me = unsafe { libc::geteuid() };
        if !meta.is_dir() || meta.uid() != me {
            return Err(io::Error::other(format!("{} isn't a folder of this user's", dir.display())));
        }
        if meta.mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert!(valid_tag("0123abcdef-_X"));
        assert!(!valid_tag(""));
        assert!(!valid_tag("../x"));
        assert!(!valid_tag("a b"));
        assert_eq!(tag_of(b""), "cbf29ce484222325");
        assert_ne!(tag_of(b"a"), tag_of(b"b"));
        assert!(valid_tag(&tag_of(b"anything")));
    }
}
