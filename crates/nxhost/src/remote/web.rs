// Mush8 - web.rs
// Constantin Clerc (c22dev) - 0.1
//! Serves the web controller page and its WebSocket, so any phone's browser can join as a player
use super::{Link, PORT, Pacer, TIMEOUT};
use nxpad::Pad;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

const PAGE: &[u8] = include_bytes!("../../../nxwebpad/web/index.html");
const WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nxwebpad.wasm"));
const MAX_CONNECTIONS: usize = 32;
// how long a socket read waits before the connection thread checks for an ack or rumble to send
const POLL: Duration = Duration::from_millis(15);

static OPEN: AtomicUsize = AtomicUsize::new(0);
static SERVING: Mutex<Option<Result<(), String>>> = Mutex::new(None);

/// Where phones join (`http://<LAN IP>:21421`), or why they can't. None when phone controllers are off
pub fn join_url() -> Option<Result<String, String>> {
    let serving = SERVING.lock().unwrap().clone()?;
    Some(serving.and_then(|()| lan_ip().map(|ip| format!("http://{ip}:{PORT}")).ok_or_else(|| "no Wi-Fi or Ethernet".into())))
}

pub fn serve() {
    let listener = match TcpListener::bind(("0.0.0.0", PORT)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[phone] can't serve the web controller on TCP {PORT}: {e}");
            *SERVING.lock().unwrap() = Some(Err(format!("port {PORT} is busy")));
            return;
        }
    };
    let _ = std::thread::Builder::new().name("phone-web".into()).spawn(move || {
        let url = lan_ip().map_or(format!("port {PORT} (no Wi-Fi or Ethernet yet)"), |ip| format!("http://{ip}:{PORT}"));
        let blocked = firewall_blocks();
        match &blocked {
            Some(why) => eprintln!("[phone] {why}: phones can't reach {url}. Allow Mush8 in System Settings → Network → Firewall → Options"),
            None => eprintln!("[phone] phones can join from a browser at {url}"),
        }
        *SERVING.lock().unwrap() = Some(blocked.map_or(Ok(()), Err));
        accept(listener);
    });
}

fn accept(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if OPEN.fetch_add(1, Relaxed) >= MAX_CONNECTIONS {
            OPEN.fetch_sub(1, Relaxed);
            let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
            let _ = respond(&stream, "503 Service Unavailable", "text/plain", b"Too many connections");
            continue;
        }
        let spawned = std::thread::Builder::new().name("phone-web-conn".into()).spawn(move || {
            if let Err(e) = handle(stream) {
                eprintln!("[phone] web controller connection: {e}");
            }
            OPEN.fetch_sub(1, Relaxed);
        });
        if spawned.is_err() {
            OPEN.fetch_sub(1, Relaxed);
        }
    }
}

fn handle(mut stream: TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let head = read_head(&mut stream)?;
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);
    if !matches!(req.parse(&head), Ok(httparse::Status::Complete(_))) || req.method != Some("GET") {
        return respond(&stream, "400 Bad Request", "text/plain", b"Bad request");
    }
    let header = |name: &str| req.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).and_then(|h| std::str::from_utf8(h.value).ok());
    let (path, query) = req.path.unwrap_or("/").split_once('?').unwrap_or((req.path.unwrap_or("/"), ""));
    match path {
        "/" | "/index.html" => respond(&stream, "200 OK", "text/html; charset=utf-8", PAGE),
        "/pad.wasm" => respond(&stream, "200 OK", "application/wasm", WASM),
        "/favicon.ico" => respond(&stream, "204 No Content", "image/x-icon", b""),
        "/ws" if header("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket")) => {
            let Some(key) = header("sec-websocket-key") else {
                return respond(&stream, "400 Bad Request", "text/plain", b"Missing Sec-WebSocket-Key");
            };
            let accept = tungstenite::handshake::derive_accept_key(key.trim().as_bytes());
            let id = query.split('&').find_map(|kv| kv.strip_prefix("id=")).and_then(|id| u64::from_str_radix(id, 16).ok());
            let peer = stream.peer_addr()?;
            write!(stream, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n")?;
            let link = Link::Web(id.unwrap_or_else(|| fallback_id(peer.port())));
            eprintln!("[phone] {link} opened from {}", peer.ip());
            play(stream, link)
        }
        _ => respond(&stream, "404 Not Found", "text/plain", b"Not found"),
    }
}

fn fallback_id(port: u16) -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    t.as_nanos() as u64 ^ (port as u64) << 48
}

fn read_head(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        if head.len() > 16 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "request head too large"));
        }
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        head.extend_from_slice(&buf[..n]);
    }
    Ok(head)
}

fn respond(mut stream: &TcpStream, status: &str, kind: &str, body: &[u8]) -> io::Result<()> {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n", body.len())?;
    stream.write_all(body)?;
    stream.flush()
}

/// One browser's session: pads in as they come, acks and rumble out as they change. Only this thread waits on a slow phone
fn play(stream: TcpStream, link: Link) -> io::Result<()> {
    stream.set_read_timeout(Some(POLL))?;
    let mut ws = WebSocket::from_raw_socket(stream, Role::Server, None);
    let mut pacer = Pacer::default();
    let mut heard = Instant::now();
    let why = loop {
        match ws.read() {
            Ok(Message::Binary(b)) => {
                if let Some(pad) = Pad::decode(&b) {
                    heard = Instant::now();
                    super::feed(link, pad);
                }
            }
            Ok(Message::Close(_)) => break "closed".to_string(),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => break e.to_string(),
        }
        if heard.elapsed() > TIMEOUT {
            break format!("silent for {} s", TIMEOUT.as_secs());
        }
        if let Some(ack) = super::ack(link)
            && pacer.due(Instant::now(), ack)
            && let Err(e) = ws.send(Message::Binary(ack.encode().to_vec().into()))
        {
            break e.to_string();
        }
    };
    // the slot stays reserved for a moment in case the page reconnects, but nothing stays held
    super::feed(link, Pad::default());
    eprintln!("[phone] {link} socket ended ({why})");
    Ok(())
}

/// The address phones should type: Wi-Fi or Ethernet first, never a VPN or link-local one
fn lan_ip() -> Option<Ipv4Addr> {
    let mut found: Vec<(String, Ipv4Addr)> = Vec::new();
    unsafe {
        let mut ifa: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifa) != 0 {
            return None;
        }
        let mut p = ifa;
        while !p.is_null() {
            let i = &*p;
            let up = i.ifa_flags & (libc::IFF_UP | libc::IFF_RUNNING) as u32 == (libc::IFF_UP | libc::IFF_RUNNING) as u32;
            if up && !i.ifa_addr.is_null() && (*i.ifa_addr).sa_family as i32 == libc::AF_INET {
                let name = std::ffi::CStr::from_ptr(i.ifa_name).to_string_lossy().into_owned();
                let ip = Ipv4Addr::from((*(i.ifa_addr as *const libc::sockaddr_in)).sin_addr.s_addr.to_ne_bytes());
                found.push((name, ip));
            }
            p = i.ifa_next;
        }
        libc::freeifaddrs(ifa);
    }
    pick_lan_ip(&found)
}

fn pick_lan_ip(found: &[(String, Ipv4Addr)]) -> Option<Ipv4Addr> {
    let usable = |ip: &Ipv4Addr| !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified();
    let rank = |(name, ip): &(String, Ipv4Addr)| (name != "en0", !name.starts_with("en"), !ip.is_private());
    found.iter().filter(|(name, ip)| usable(ip) && !["utun", "bridge", "awdl", "llw", "anpi", "ap", "pdp_ip"].iter().any(|v| name.starts_with(v))).min_by_key(|f| rank(f)).map(|(_, ip)| *ip)
}

/// The macOS application firewall drops incoming connections silently, so ask it instead of waiting for phones that never arrive
#[cfg(target_os = "macos")]
fn firewall_blocks() -> Option<String> {
    let ask = |args: &[&str]| {
        std::process::Command::new("/usr/libexec/ApplicationFirewall/socketfilterfw").args(args).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    if !ask(&["--getglobalstate"])?.contains("enabled") {
        return None;
    }
    if ask(&["--getblockall"]).is_some_and(|s| s.contains("enabled")) {
        return Some("the macOS firewall blocks all incoming connections".into());
    }
    let exe = std::env::current_exe().ok()?;
    ask(&["--getappblocked", &exe.to_string_lossy()])
        .is_some_and(|s| s.contains("is blocked"))
        .then(|| "the macOS firewall blocks Mush8".into())
}

#[cfg(not(target_os = "macos"))]
fn firewall_blocks() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn picks_the_wifi_address() {
        let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap();
        let found = |v: &[(&str, &str)]| v.iter().map(|(n, a)| (n.to_string(), ip(a))).collect::<Vec<_>>();
        assert_eq!(pick_lan_ip(&found(&[("lo0", "127.0.0.1"), ("utun4", "100.101.1.2"), ("en0", "192.168.1.20")])), Some(ip("192.168.1.20")));
        assert_eq!(pick_lan_ip(&found(&[("bridge100", "192.168.64.1"), ("en7", "10.0.0.4"), ("en0", "169.254.3.3")])), Some(ip("10.0.0.4")));
        assert_eq!(pick_lan_ip(&found(&[("lo0", "127.0.0.1")])), None);
    }

    fn get(port: u16, path: &str) -> (String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&out[..split]).into_owned(), out[split + 4..].to_vec())
    }

    #[test]
    fn browser_joins_through_the_registry() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || accept(listener));

        let (head, body) = get(port, "/");
        assert!(head.starts_with("HTTP/1.1 200") && head.contains("text/html"));
        assert_eq!(body, PAGE);
        let (head, body) = get(port, "/pad.wasm");
        assert!(head.contains("application/wasm"));
        assert_eq!(&body[..4], b"\0asm");
        assert!(get(port, "/nope").0.starts_with("HTTP/1.1 404"));

        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (mut ws, _) = tungstenite::client(format!("ws://127.0.0.1:{port}/ws?id=00000000c0ffee01"), stream).unwrap();
        let link = Link::Web(0xc0ffee01);
        let pad = Pad { buttons: nxpad::A | nxpad::R, sticks: [0.5, -0.25, 0.0, 0.0], motion: None };
        ws.send(Message::Binary(pad.encode().to_vec().into())).unwrap();
        let joined = Instant::now();
        while !super::super::connected().iter().any(|&(l, p)| l == link && p == pad) {
            assert!(joined.elapsed() < Duration::from_secs(2), "the pad never reached the registry");
            std::thread::sleep(Duration::from_millis(5));
        }
        super::super::set_player(link, 3);
        let ack = loop {
            if let Message::Binary(b) = ws.read().unwrap()
                && let Some(ack) = nxpad::Ack::decode(&b)
            {
                break ack;
            }
        };
        assert_eq!(ack.player, 3);
        ws.close(None).unwrap();
        while !super::super::connected().iter().any(|&(l, p)| l == link && p == Pad::default()) {
            assert!(joined.elapsed() < Duration::from_secs(2), "closing the page didn't let go of the buttons");
            let _ = ws.read();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

