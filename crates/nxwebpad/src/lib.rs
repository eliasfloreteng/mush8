// Mush8 - lib.rs
// Constantin Clerc (c22dev) - 0.1
//! The web controller's touch handling and wire format, compiled to WebAssembly. index.html only draws and talks to the socket
use nxpad::{Ack, Grab, Layout, PAD_LEN, Pad, Point, Touches};
use std::cell::RefCell;

struct State {
    touches: Touches,
    sent: Option<Pad>,
    packet: [u8; PAD_LEN],
    inbox: [u8; 64],
    ack: Option<Ack>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State { touches: Touches::default(), sent: None, packet: [0; PAD_LEN], inbox: [0; 64], ack: None });
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn control(i: u32) -> nxpad::Control {
    with(|s| s.touches.layout().controls.get(i as usize).copied().unwrap_or_default())
}

#[unsafe(no_mangle)]
pub extern "C" fn resize(width: f64, height: f64, inset_left: f64, inset_right: f64) {
    with(|s| s.touches.set_layout(Layout::new(width, height, inset_left, inset_right)));
}

#[unsafe(no_mangle)]
pub extern "C" fn controls() -> u32 {
    nxpad::BUTTONS.len() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn control_x(i: u32) -> f64 {
    control(i).center.x
}

#[unsafe(no_mangle)]
pub extern "C" fn control_y(i: u32) -> f64 {
    control(i).center.y
}

#[unsafe(no_mangle)]
pub extern "C" fn control_radius(i: u32) -> f64 {
    control(i).radius
}

#[unsafe(no_mangle)]
pub extern "C" fn control_half(i: u32) -> f64 {
    control(i).half
}

#[unsafe(no_mangle)]
pub extern "C" fn control_bit(i: u32) -> u32 {
    nxpad::BUTTONS.get(i as usize).map_or(0, |b| b.1 as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn control_label(i: u32) -> *const u8 {
    nxpad::BUTTONS.get(i as usize).map_or("", |b| b.0).as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn control_label_len(i: u32) -> u32 {
    nxpad::BUTTONS.get(i as usize).map_or(0, |b| b.0.len() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_x() -> f64 {
    with(|s| s.touches.stick_origin().x)
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_y() -> f64 {
    with(|s| s.touches.stick_origin().y)
}

#[unsafe(no_mangle)]
pub extern "C" fn knob_x() -> f64 {
    with(|s| s.touches.knob().x)
}

#[unsafe(no_mangle)]
pub extern "C" fn knob_y() -> f64 {
    with(|s| s.touches.knob().y)
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_radius() -> f64 {
    with(|s| s.touches.layout().stick_radius)
}

#[unsafe(no_mangle)]
pub extern "C" fn knob_radius() -> f64 {
    with(|s| s.touches.layout().knob_radius)
}

/// 0 for a touch on nothing, 1 for a button, 2 for the stick
#[unsafe(no_mangle)]
pub extern "C" fn touch_start(id: u32, x: f64, y: f64) -> u32 {
    match with(|s| s.touches.begin(id as u64, Point::new(x, y))) {
        Grab::Nothing => 0,
        Grab::Button => 1,
        Grab::Stick => 2,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn touch_move(id: u32, x: f64, y: f64) -> u32 {
    with(|s| s.touches.moved(id as u64, Point::new(x, y))) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn touch_end(id: u32) {
    with(|s| s.touches.end(id as u64));
}

#[unsafe(no_mangle)]
pub extern "C" fn release() {
    with(|s| s.touches.release_all());
}

#[unsafe(no_mangle)]
pub extern "C" fn buttons() -> u32 {
    with(|s| s.touches.buttons() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn pad_len() -> u32 {
    PAD_LEN as u32
}

/// The pad packet to send, or null when nothing changed since the last one and `force` (a heartbeat) is 0
#[unsafe(no_mangle)]
pub extern "C" fn packet(force: u32) -> *const u8 {
    with(|s| {
        let pad = s.touches.pad();
        if force == 0 && s.sent == Some(pad) {
            return std::ptr::null();
        }
        s.sent = Some(pad);
        s.packet = pad.encode();
        s.packet.as_ptr()
    })
}

/// Where the page copies a message from the host before calling `ack`
#[unsafe(no_mangle)]
pub extern "C" fn inbox() -> *mut u8 {
    with(|s| s.inbox.as_mut_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn inbox_len() -> u32 {
    with(|s| s.inbox.len() as u32)
}

/// Reads an ack from the inbox: the player number (0 when the game is full), or -1 when it isn't one
#[unsafe(no_mangle)]
pub extern "C" fn ack(len: u32) -> i32 {
    with(|s| {
        let len = (len as usize).min(s.inbox.len());
        s.ack = Ack::decode(&s.inbox[..len]);
        s.ack.map_or(-1, |a| a.player as i32)
    })
}

/// How hard the last ack asked this phone to rumble, 0 to 1
#[unsafe(no_mangle)]
pub extern "C" fn rumble() -> f64 {
    with(|s| s.ack.map_or(0.0, |a| a.rumble as f64 / 255.0))
}

/// Forgets what was last sent, so a new connection gets the pad straight away
#[unsafe(no_mangle)]
pub extern "C" fn reset() {
    with(|s| {
        s.sent = None;
        s.ack = None;
    })
}
