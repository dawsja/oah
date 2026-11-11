//! LAN discovery is only an address hint. Pairing still requires the desktop's secret code.
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;

pub const PORT: u16 = 7788;
const MAGIC: &str = "oah-discover-v1:";

#[cfg(feature = "mdns")]
pub use bonjour::{advertise_bonjour, Bonjour};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Found {
    pub name: String,
    pub address: String,
}

#[derive(Serialize, Deserialize)]
struct Reply {
    nonce: String,
    name: String,
    port: u16,
}

pub struct Beacon(tokio::task::JoinHandle<()>);
impl Drop for Beacon {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn advertise(name: String, port: u16) -> std::io::Result<Beacon> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT)).await?;
    Ok(serve(socket, name, port))
}

fn serve(socket: UdpSocket, name: String, port: u16) -> Beacon {
    Beacon(tokio::spawn(async move {
        let mut buf = [0u8; 256];
        loop {
            let Ok((n, peer)) = socket.recv_from(&mut buf).await else { break };
            let Ok(query) = std::str::from_utf8(&buf[..n]) else { continue };
            let Some(nonce) = query.strip_prefix(MAGIC) else { continue };
            if nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let reply = Reply { nonce: nonce.into(), name: name.chars().take(80).collect(), port };
            let _ = socket.send_to(&serde_json::to_vec(&reply).unwrap(), peer).await;
        }
    }))
}

pub async fn discover() -> Result<Vec<Found>, String> {
    let targets = [SocketAddr::from((Ipv4Addr::BROADCAST, PORT))];
    scan(&targets, Duration::from_secs(3)).await
}

async fn scan(targets: &[SocketAddr], duration: Duration) -> Result<Vec<Found>, String> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.map_err(|e| e.to_string())?;
    socket.set_broadcast(true).map_err(|e| e.to_string())?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let nonce: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let query = format!("{MAGIC}{nonce}");
    let deadline = tokio::time::Instant::now() + duration;
    let mut retry = tokio::time::interval(Duration::from_millis(700));
    let mut found: Vec<Found> = vec![];
    let mut buf = [0u8; 1024];
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            _ = retry.tick() => {
                for target in targets { let _ = socket.send_to(query.as_bytes(), target).await; }
            }
            result = socket.recv_from(&mut buf) => {
                let (n, peer) = result.map_err(|e| e.to_string())?;
                let Some(reply) = parse_reply(&buf[..n], peer, &nonce) else { continue };
                if !found.iter().any(|d| d.address == reply.address) {
                    found.push(reply);
                }
                if found.len() >= 64 { break; }
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name).then(a.address.cmp(&b.address)));
    Ok(found)
}

fn parse_reply(bytes: &[u8], peer: SocketAddr, nonce: &str) -> Option<Found> {
    let reply: Reply = serde_json::from_slice(bytes).ok()?;
    if reply.nonce != nonce
        || reply.port == 0
        || reply.name.trim().is_empty()
        || reply.name.len() > 320
        || reply.name.chars().any(char::is_control)
    {
        return None;
    }
    Some(Found { name: reply.name, address: SocketAddr::new(peer.ip(), reply.port).to_string() })
}

/// The desktop as a Bonjour service (`_oah._tcp`), for phones that can't send the broadcast above:
/// an iPhone may browse Bonjour, but not broadcast. The service's TXT record carries what the
/// broadcast's reply does, the desktop's `name` and where it listens (`addrs`: `ip:port,…`), so
/// the phone needs nothing but the record.
#[cfg(feature = "mdns")]
mod bonjour {
    use std::net::IpAddr;
    use std::time::Duration;

    use mdns_sd::{ServiceDaemon, ServiceInfo};

    pub(super) const TYPE: &str = "_oah._tcp.local.";
    /// How often the desktop's addresses are looked at again (a new Wi-Fi, a VPN coming up).
    const REFRESH: Duration = Duration::from_secs(20);

    /// The advertisement: dropping it withdraws the service.
    pub struct Bonjour {
        daemon: ServiceDaemon,
        fullname: String,
        refresh: tokio::task::JoinHandle<()>,
    }

    impl Drop for Bonjour {
        fn drop(&mut self) {
            self.refresh.abort();
            let _ = self.daemon.unregister(&self.fullname);
            let _ = self.daemon.shutdown();
        }
    }

    /// The service's record for a desktop called `name` listening on `port` at `ips`.
    pub(super) fn record(name: &str, port: u16, ips: &[IpAddr]) -> Result<ServiceInfo, String> {
        let name: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
        let name = if name.trim().is_empty() { "oah desktop" } else { name.trim() };
        // A service's instance name is at most 63 bytes.
        let mut instance = name.to_string();
        while instance.len() > 63 {
            instance.pop();
        }
        // Its own host name: the computer's is the system's to answer for.
        let label: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).take(40).collect();
        let host = format!("oah-{}.local.", label.trim_matches('-'));
        let addrs: Vec<String> = ips.iter().take(8).map(|ip| std::net::SocketAddr::new(*ip, port).to_string()).collect();
        let txt = [("name", name.to_string()), ("addrs", addrs.join(",")), ("v", "1".to_string())];
        let info = ServiceInfo::new(TYPE, &instance, &host, ips, port, &txt[..]).map_err(|e| e.to_string())?;
        // With no address of its own to give, the host's are filled in.
        Ok(if ips.is_empty() { info.enable_addr_auto() } else { info })
    }

    /// Advertises the desktop until the result is dropped. `addresses` is asked again every so
    /// often, and the record follows it. Must run inside a tokio runtime.
    pub fn advertise_bonjour(name: String, port: u16, addresses: impl Fn() -> Vec<IpAddr> + Send + 'static) -> Result<Bonjour, String> {
        let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
        let mut current = addresses();
        let info = record(&name, port, &current)?;
        let fullname = info.get_fullname().to_string();
        daemon.register(info).map_err(|e| e.to_string())?;
        let again = daemon.clone();
        let refresh = tokio::spawn(async move {
            loop {
                tokio::time::sleep(REFRESH).await;
                let now = addresses();
                if now != current {
                    // Registering again announces the updated record.
                    if let Ok(info) = record(&name, port, &now) {
                        let _ = again.register(info);
                    }
                    current = now;
                }
            }
        });
        Ok(Bonjour { daemon, fullname, refresh })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_replies_to_this_scan_and_uses_the_sender_address() {
        let peer = "192.168.1.20:7788".parse().unwrap();
        let bytes = br#"{"nonce":"current", "name":"PC", "port":9012, "address":"evil.invalid:80"}"#;
        assert_eq!(parse_reply(bytes, peer, "current").unwrap().address, "192.168.1.20:9012");
        assert!(parse_reply(bytes, peer, "another scan").is_none());
        assert!(parse_reply(br#"{"nonce":"current", "name":"PC", "port":0}"#, peer, "current").is_none());
        assert!(parse_reply(b"not json", peer, "current").is_none());
    }

    #[tokio::test]
    async fn discovers_actual_port_without_disclosing_pairing_secrets() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = socket.local_addr().unwrap();
        let beacon = serve(socket, "Desktop".into(), 9012);
        let found = scan(&[target], Duration::from_millis(100)).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].address, "127.0.0.1:9012");
        assert_eq!(found[0].name, "Desktop");
        drop(beacon);
    }

    #[cfg(feature = "mdns")]
    #[test]
    fn the_bonjour_record_says_where_the_desktop_listens() {
        let ips: Vec<std::net::IpAddr> = vec!["192.168.1.20".parse().unwrap(), "100.64.0.3".parse().unwrap()];
        let info = bonjour::record("Dawson's PC", 7787, &ips).unwrap();
        assert_eq!(info.get_fullname(), "Dawson's PC._oah._tcp.local.");
        assert_eq!(info.get_hostname(), "oah-dawson-s-pc.local.");
        assert_eq!(info.get_port(), 7787);
        assert_eq!(info.get_property_val_str("name"), Some("Dawson's PC"));
        assert_eq!(info.get_property_val_str("addrs"), Some("192.168.1.20:7787,100.64.0.3:7787"));

        // No name, a long one, many addresses: still a record a phone can read.
        assert_eq!(bonjour::record(" \n ", 7787, &ips).unwrap().get_property_val_str("name"), Some("oah desktop"));
        let long = bonjour::record(&"é".repeat(200), 9000, &[]).unwrap();
        assert!(long.get_fullname().len() <= 63 + "._oah._tcp.local.".len());
        assert_eq!(long.get_property_val_str("addrs"), Some(""));
        let many: Vec<std::net::IpAddr> = (1..=20).map(|i| format!("10.0.0.{i}").parse().unwrap()).collect();
        let addrs = bonjour::record("pc", 7787, &many).unwrap().get_property_val_str("addrs").unwrap().to_string();
        assert_eq!(addrs.split(',').count(), 8);
        assert!(addrs.len() < 250, "it fits in a TXT entry");
    }

    /// The service as another program on this network sees it. Needs multicast on a real
    /// interface, so it's run by hand: `cargo test -p oah-remote --features mdns -- --ignored`.
    #[cfg(feature = "mdns")]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn real_bonjour_is_found_by_a_browser() {
        let name = format!("oah-test-{}", std::process::id());
        // This computer's address on its way out (no packet is sent to find it): a service is
        // announced on the interfaces its addresses belong to.
        let probe = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        probe.connect("192.0.2.1:9").unwrap();
        let ip = probe.local_addr().unwrap().ip();
        let ips = vec![ip];
        let _advert = advertise_bonjour(name.clone(), 7787, move || ips.clone()).unwrap();
        let browser = mdns_sd::ServiceDaemon::new().unwrap();
        let found = browser.browse(bonjour::TYPE).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.checked_duration_since(std::time::Instant::now()).expect("the service was found");
            if let Ok(mdns_sd::ServiceEvent::ServiceResolved(s)) = found.recv_timeout(left) {
                if s.get_property_val_str("name") == Some(name.as_str()) {
                    assert_eq!(s.get_property_val_str("addrs"), Some(format!("{ip}:7787").as_str()));
                    assert_eq!(s.get_port(), 7787);
                    break;
                }
            }
        }
        let _ = browser.shutdown();
    }
}
