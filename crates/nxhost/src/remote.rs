// Mush8 - remote.rs
// Constantin Clerc (c22dev) - 0.1
use nxpad::{Ack, Pad};
use std::collections::HashMap;
use std::fmt;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

mod web;

pub use web::join_url;

const TYPE: &str = "_mush8-pad._udp";
const PORT: u16 = 21421;
const TIMEOUT: Duration = Duration::from_secs(3);
const ACK_EVERY: Duration = Duration::from_millis(500);

/// How a phone reaches the game: the Mush8 app over UDP, or a browser over a WebSocket (keyed by the id the page keeps across reconnects)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Link {
    App(SocketAddr),
    Web(u64),
}

impl Link {
    pub fn name(&self) -> &'static str {
        match self {
            Link::App(_) => "iPhone",
            Link::Web(_) => "Phone",
        }
    }
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Link::App(addr) => write!(f, "iPhone controller at {addr}"),
            Link::Web(id) => write!(f, "browser controller {id:016x}"),
        }
    }
}

struct Phone {
    link: Link,
    pad: Pad,
    seen: Instant,
    player: Option<u8>,
}

/// Every connected phone, whatever its transport. Input gives each one a player slot, the transports ack it back
struct Registry(Vec<Phone>);

impl Registry {
    fn sweep(&mut self, now: Instant) {
        self.0.retain(|p| {
            let live = now.duration_since(p.seen) < TIMEOUT;
            if !live {
                eprintln!("[phone] {} disconnected", p.link);
            }
            live
        });
    }

    fn feed(&mut self, link: Link, pad: Pad, now: Instant) {
        match self.0.iter_mut().find(|p| p.link == link) {
            Some(p) => (p.pad, p.seen) = (pad, now),
            None => {
                eprintln!("[phone] {link} connected");
                self.0.push(Phone { link, pad, seen: now, player: None });
            }
        }
    }

    fn player(&self, link: Link) -> Option<u8> {
        self.0.iter().find(|p| p.link == link)?.player
    }

    fn set_player(&mut self, link: Link, player: u8) {
        if let Some(p) = self.0.iter_mut().find(|p| p.link == link) {
            p.player = Some(player);
        }
    }
}

static PHONES: Mutex<Registry> = Mutex::new(Registry(Vec::new()));

fn phones() -> MutexGuard<'static, Registry> {
    let mut phones = PHONES.lock().unwrap();
    phones.sweep(Instant::now());
    phones
}

/// Takes a pad from any transport: a new link joins, a known one is refreshed
pub fn feed(link: Link, pad: Pad) {
    phones().feed(link, pad, Instant::now());
}

/// What to tell a phone once input has given it a slot (player 0 when the game is full)
pub fn ack(link: Link) -> Option<Ack> {
    let player = phones().player(link)?;
    let rumble = if player == 0 { 0.0 } else { crate::haptics::strength(player as usize - 1) };
    Some(Ack { player, rumble: (rumble.clamp(0.0, 1.0) * 15.0).round() as u8 * 17 })
}

pub fn connected() -> Vec<(Link, Pad)> {
    phones().0.iter().map(|p| (p.link, p.pad)).collect()
}

pub fn set_player(link: Link, player: u8) {
    phones().set_player(link, player);
}

/// Sends an ack when it changes, and every ACK_EVERY so the phone knows the game is still there
#[derive(Default)]
struct Pacer(Option<(Instant, Ack)>);

impl Pacer {
    fn due(&mut self, now: Instant, ack: Ack) -> bool {
        let due = self.0.is_none_or(|(at, was)| was != ack || now.duration_since(at) >= ACK_EVERY);
        if due {
            self.0 = Some((now, ack));
        }
        due
    }
}

pub fn enabled() -> bool {
    let default = cfg!(target_os = "macos");
    std::env::var("NX_PHONE_PADS").map_or(default, |v| v != "0")
}

pub fn host() {
    web::serve();
    let sock = match UdpSocket::bind(("0.0.0.0", PORT)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[phone] can't listen for iPhone controllers on UDP {PORT}: {e}");
            return;
        }
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = std::thread::Builder::new().name("phone-pads".into()).spawn(move || {
        let mut buf = [0u8; 256];
        let mut pacers: HashMap<SocketAddr, Pacer> = HashMap::new();
        loop {
            if let Ok((n, addr)) = sock.recv_from(&mut buf)
                && let Some(pad) = Pad::decode(&buf[..n])
            {
                feed(Link::App(addr), pad);
            }
            let now = Instant::now();
            let apps: Vec<SocketAddr> = phones().0.iter().filter_map(|p| if let Link::App(a) = p.link { Some(a) } else { None }).collect();
            pacers.retain(|a, _| apps.contains(a));
            for addr in apps {
                if let Some(ack) = ack(Link::App(addr))
                    && pacers.entry(addr).or_default().due(now, ack)
                {
                    let _ = sock.send_to(&ack.encode(), addr);
                }
            }
        }
    });
    let _ = std::thread::Builder::new().name("phone-bonjour".into()).spawn(|| loop {
        advertise();
        std::thread::sleep(Duration::from_secs(5));
    });
}

fn advertise() {
    use crate::dnssd::*;
    let typ = std::ffi::CString::new(TYPE).unwrap();
    unsafe {
        let mut sd: DNSServiceRef = std::ptr::null_mut();
        let r = DNSServiceRegister(&mut sd, 0, 0, std::ptr::null(), typ.as_ptr(), std::ptr::null(), std::ptr::null(), PORT.to_be(), 0, std::ptr::null(), None, std::ptr::null_mut());
        if r != 0 {
            eprintln!("[phone] bonjour did not start ({r}){}", denied(r));
            return;
        }
        eprintln!("[phone] iPhones can join as controllers ({TYPE})");
        while wait(sd, -1) && DNSServiceProcessResult(sd) == 0 {}
        DNSServiceRefDeallocate(sd);
    }
}

#[cfg(target_os = "ios")]
pub use phone::run as run_controller;

#[cfg(target_os = "ios")]
mod phone {
    use super::*;
    use crate::dnssd::*;
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::sync::atomic::Ordering::Relaxed;

    struct Host {
        name: String,
        addr: SocketAddr,
    }

    pub fn run(status: fn(&str)) {
        let sock = UdpSocket::bind(("0.0.0.0", 0)).expect("udp socket");
        let _ = sock.set_nonblocking(true);
        let mut motion = None;
        eprintln!("[phone] controller mode: looking for {TYPE}");
        status(&format!("Controller mode\n\n{LOOKING}"));
        loop {
            let host = match find() {
                Ok(host) => host,
                Err(why) => {
                    status(&format!("Controller mode\n\n{why}"));
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };
            eprintln!("[phone] found {} at {}", host.name, host.addr);
            status(&format!("Controller mode\n\nConnecting to {}…", host.name));
            let mut heard = Instant::now();
            let mut player = None;
            while heard.elapsed() < TIMEOUT {
                crate::gfx::wait_foreground();
                let (buttons, stick) = (crate::input::TOUCH_BUTTONS.load(Relaxed), crate::input::TOUCH_STICK.load(Relaxed));
                let (lx, ly) = (f32::from_bits((stick >> 32) as u32), f32::from_bits(stick as u32));
                let pad = Pad { buttons, sticks: [lx, ly, 0.0, 0.0], motion: crate::input::phone_motion_now(&mut motion) };
                let _ = sock.send_to(&pad.encode(), host.addr);
                let mut buf = [0u8; 16];
                while let Ok((n, from)) = sock.recv_from(&mut buf) {
                    if from == host.addr && let Some(ack) = Ack::decode(&buf[..n]) {
                        heard = Instant::now();
                        if player != Some(ack.player) {
                            player = Some(ack.player);
                            status(&match ack.player {
                                0 => format!("Controller mode\n\n{} has no free player slot", host.name),
                                n => format!("Player {n} on {}", host.name),
                            });
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(8));
            }
            eprintln!("[phone] lost {}: no answer for {} s", host.name, TIMEOUT.as_secs());
        }
    }

    const LOOKING: &str = "Looking for a Mac running Mush8 on this Wi-Fi…\n(start the game on the Mac, with phone controllers on)";
    const DENIED: &str = "Local network access is off for Mush8\n\nTurn it on in Settings → Privacy & Security → Local Network";

    #[derive(Default)]
    struct Browse {
        seen: Option<(CString, CString, CString, u32)>,
        err: i32,
    }

    fn find() -> Result<Host, String> {
        let typ = CString::new(TYPE).unwrap();
        let mut b = Browse::default();
        unsafe {
            let mut sd: DNSServiceRef = std::ptr::null_mut();
            let ctx = &mut b as *mut Browse as *mut c_void;
            let r = DNSServiceBrowse(&mut sd, 0, 0, typ.as_ptr(), std::ptr::null(), Some(on_browse), ctx);
            if r != 0 {
                eprintln!("[phone] bonjour browse failed ({r}){}", denied(r));
                return Err(if r == POLICY_DENIED { DENIED.into() } else { format!("Bonjour is unavailable ({r})") });
            }
            let start = Instant::now();
            while b.seen.is_none() && b.err == 0 && start.elapsed() < Duration::from_secs(3) {
                if wait(sd, 500) && DNSServiceProcessResult(sd) != 0 {
                    break;
                }
            }
            DNSServiceRefDeallocate(sd);
        }
        if b.err != 0 {
            eprintln!("[phone] bonjour browse error {}{}", b.err, denied(b.err));
            return Err(if b.err == POLICY_DENIED { DENIED.into() } else { format!("Bonjour error {}", b.err) });
        }
        let Some((name, regtype, domain, iface)) = b.seen else { return Err(LOOKING.into()) };
        let shown = name.to_string_lossy().into_owned();
        let Some(r) = resolve(&name, &regtype, &domain, iface) else {
            eprintln!("[phone] found {shown} but could not resolve its address");
            return Err(format!("Found {shown} but couldn't reach it\n\nCheck both are on the same Wi-Fi"));
        };
        Ok(Host { name: shown, addr: SocketAddr::from((r.ip, r.port)) })
    }

    unsafe extern "C" fn on_browse(_: DNSServiceRef, flags: u32, iface: u32, err: i32, name: *const c_char, regtype: *const c_char, domain: *const c_char, ctx: *mut c_void) {
        let b = unsafe { &mut *(ctx as *mut Browse) };
        if err != 0 {
            b.err = err;
            return;
        }
        if flags & ADD == 0 || name.is_null() || regtype.is_null() || domain.is_null() || b.seen.is_some() {
            return;
        }
        b.seen = unsafe { Some((CStr::from_ptr(name).to_owned(), CStr::from_ptr(regtype).to_owned(), CStr::from_ptr(domain).to_owned(), iface)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_tracks_every_transport() {
        let mut r = Registry(Vec::new());
        let t = Instant::now();
        let (app, web) = (Link::App("192.168.1.20:5000".parse().unwrap()), Link::Web(7));
        let pad = Pad { buttons: nxpad::A, ..Default::default() };
        r.feed(app, Pad::default(), t);
        r.feed(web, pad, t);
        assert_eq!(r.0.len(), 2);
        assert_eq!(r.player(web), None, "no slot until input gives one");
        r.set_player(web, 2);
        r.set_player(app, 0);
        assert_eq!((r.player(web), r.player(app)), (Some(2), Some(0)));
        r.feed(web, Pad::default(), t + Duration::from_secs(2));
        assert_eq!(r.0.len(), 2, "a refresh doesn't add a phone");
        assert_eq!(r.player(web), Some(2), "and keeps its slot");
        r.sweep(t + TIMEOUT);
        assert_eq!(r.0.iter().map(|p| p.link).collect::<Vec<_>>(), [web], "silent phones time out");
        r.sweep(t + Duration::from_secs(2) + TIMEOUT);
        assert!(r.0.is_empty());
    }

    #[test]
    fn acks_go_out_on_change_and_every_half_second() {
        let mut p = Pacer::default();
        let t = Instant::now();
        let ack = Ack { player: 1, rumble: 0 };
        assert!(p.due(t, ack));
        assert!(!p.due(t + Duration::from_millis(100), ack));
        assert!(p.due(t + Duration::from_millis(120), Ack { rumble: 34, ..ack }));
        assert!(!p.due(t + Duration::from_millis(200), Ack { rumble: 34, ..ack }));
        assert!(p.due(t + Duration::from_millis(620), Ack { rumble: 34, ..ack }));
    }
}
