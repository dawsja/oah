//! LAN discovery is only an address hint. Pairing still requires the desktop's secret code.
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;

pub const PORT: u16 = 7788;
const MAGIC: &str = "oah-discover-v1:";

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
}
