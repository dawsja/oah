//! Desktops near this phone. Android asks with a UDP broadcast (`oah_remote::discovery`). iOS
//! doesn't let an app broadcast, so there the desktop's Bonjour service (`_oah._tcp`) is browsed
//! instead, by the Swift plugin; its TXT record lists the desktop's addresses, and the one that
//! answers is the one shown. Either way it's only a hint: pairing still needs the code.

use std::net::SocketAddr;
use std::time::Duration;

use oah_remote::discovery::Found;
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpStream;
use tokio::task::JoinSet;

/// How long an advertised address gets to accept a connection.
const ANSWER: Duration = Duration::from_millis(800);
/// What the UI shows (and offers Settings for) when iOS keeps the local network from the app.
pub const NO_LOCAL_NETWORK: &str =
    "oah needs Local Network access to find computers. If the iPhone just asked, tap Allow and search again. Otherwise turn it on in Settings.";

#[derive(Deserialize)]
struct Browsed {
    found: Vec<Service>,
}

/// A `_oah._tcp` service as the Swift plugin read it: its TXT record's `name` and `addrs`.
#[derive(Deserialize)]
struct Service {
    name: String,
    addrs: String,
}

pub async fn discover(app: &tauri::AppHandle) -> Result<Vec<Found>, String> {
    if !cfg!(target_os = "ios") {
        return oah_remote::discovery::discover().await;
    }
    let browsed: Browsed = crate::native(app, "browse", json!({ "seconds": 2.5 })).await.map_err(|e| {
        if e.contains("local-network-denied") {
            NO_LOCAL_NETWORK.to_string()
        } else {
            e
        }
    })?;
    let mut found = Vec::new();
    for service in browsed.found {
        if let Some(desktop) = desktop(&service.name, &service.addrs).await {
            if !found.iter().any(|f: &Found| f.address == desktop.address) {
                found.push(desktop);
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name).then(a.address.cmp(&b.address)));
    Ok(found)
}

/// A browsed service as a desktop to show, at the address that answers (else its first).
async fn desktop(name: &str, addrs: &str) -> Option<Found> {
    let name = name.trim();
    if name.is_empty() || name.len() > 320 || name.chars().any(char::is_control) {
        return None;
    }
    let addresses = parse_addrs(addrs);
    let address = match reachable(&addresses).await {
        Some(a) => a,
        None => addresses.first()?.clone(),
    };
    Some(Found { name: name.to_string(), address })
}

/// `ip:port,ip:port`: only real socket addresses, a few at most.
fn parse_addrs(addrs: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in addrs.split(',').map(str::trim) {
        let Ok(addr) = a.parse::<SocketAddr>() else { continue };
        if addr.port() != 0 && !out.contains(&addr.to_string()) {
            out.push(addr.to_string());
        }
        if out.len() == 8 {
            break;
        }
    }
    out
}

/// The first of `addresses` to accept a TCP connection.
async fn reachable(addresses: &[String]) -> Option<String> {
    let mut tries = JoinSet::new();
    for a in addresses.iter().cloned() {
        tries.spawn(async move { matches!(tokio::time::timeout(ANSWER, TcpStream::connect(&a)).await, Ok(Ok(_))).then_some(a) });
    }
    while let Some(done) = tries.join_next().await {
        if let Ok(Some(address)) = done {
            return Some(address);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_socket_addresses() {
        assert_eq!(parse_addrs("192.168.1.20:7787, 100.64.0.3:7787"), ["192.168.1.20:7787", "100.64.0.3:7787"]);
        assert_eq!(parse_addrs("pc.local:7787,192.168.1.20,10.0.0.2:0,,10.0.0.2:7787,10.0.0.2:7787"), ["10.0.0.2:7787"]);
        assert_eq!(parse_addrs("[fe80::1]:7787"), ["[fe80::1]:7787"]);
        assert_eq!(parse_addrs("10.0.0.1:1,10.0.0.1:1,10.0.0.1:1").len(), 1);
        let many: Vec<String> = (1..=20).map(|i| format!("10.0.0.{i}:7787")).collect();
        assert_eq!(parse_addrs(&many.join(",")).len(), 8);
    }

    #[tokio::test]
    async fn shows_the_address_that_answers() {
        let live = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live = live.local_addr().unwrap().to_string();
        // A port nothing listens on, then the live one.
        let d = desktop(" studio ", &format!("127.0.0.1:1,{live}")).await.unwrap();
        assert_eq!((d.name.as_str(), d.address.as_str()), ("studio", live.as_str()));
        // Nothing answers: still listed, at its first address.
        assert_eq!(desktop("studio", "127.0.0.1:1,127.0.0.1:2").await.unwrap().address, "127.0.0.1:1");
        // No usable address, or a name that isn't one.
        assert!(desktop("studio", "pc.local").await.is_none());
        assert!(desktop("  ", &live).await.is_none());
        assert!(desktop("a\nb", &live).await.is_none());
    }
}
