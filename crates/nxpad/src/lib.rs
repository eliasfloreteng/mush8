// Mush8 - lib.rs
// Constantin Clerc (c22dev) - 0.1
//! What a phone controller sends and shows, shared by the host, the iOS touch controls and the web controller

pub const A: u64 = 1 << 0;
pub const B: u64 = 1 << 1;
pub const X: u64 = 1 << 2;
pub const Y: u64 = 1 << 3;
pub const STICK_L: u64 = 1 << 4;
pub const STICK_R: u64 = 1 << 5;
pub const L: u64 = 1 << 6;
pub const R: u64 = 1 << 7;
pub const ZL: u64 = 1 << 8;
pub const ZR: u64 = 1 << 9;
pub const PLUS: u64 = 1 << 10;
pub const MINUS: u64 = 1 << 11;
pub const LEFT: u64 = 1 << 12;
pub const UP: u64 = 1 << 13;
pub const RIGHT: u64 = 1 << 14;
pub const DOWN: u64 = 1 << 15;

const PAD: [u8; 4] = *b"M8P1";
const ACK: [u8; 4] = *b"M8A1";
pub const PAD_LEN: usize = 4 + 8 + 16 + 1 + 24;
pub const ACK_LEN: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub accel: [f32; 3],
    pub gyro: [f32; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    pub buttons: u64,
    pub sticks: [f32; 4],
    pub motion: Option<Motion>,
}

impl Pad {
    pub fn encode(&self) -> [u8; PAD_LEN] {
        let mut b = [0u8; PAD_LEN];
        b[..4].copy_from_slice(&PAD);
        b[4..12].copy_from_slice(&self.buttons.to_le_bytes());
        let m = self.motion.unwrap_or(Motion { accel: [0.0; 3], gyro: [0.0; 3] });
        for (i, f) in self.sticks.iter().chain(&m.accel).chain(&m.gyro).enumerate() {
            let o = if i < 4 { 12 + 4 * i } else { 29 + 4 * (i - 4) };
            b[o..o + 4].copy_from_slice(&f.to_le_bytes());
        }
        b[28] = self.motion.is_some() as u8;
        b
    }

    pub fn decode(b: &[u8]) -> Option<Pad> {
        if b.len() < PAD_LEN || b[..4] != PAD {
            return None;
        }
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let finite = |o: usize| Some(f(o)).filter(|x| x.is_finite()).unwrap_or(0.0).clamp(-64.0, 64.0);
        let v = |o: usize| [finite(o), finite(o + 4), finite(o + 8)];
        Some(Pad {
            buttons: u64::from_le_bytes(b[4..12].try_into().unwrap()),
            sticks: [finite(12), finite(16), finite(20), finite(24)].map(|x| x.clamp(-1.0, 1.0)),
            motion: (b[28] != 0).then(|| Motion { accel: v(29), gyro: v(41) }),
        })
    }
}

/// The host's answer to a pad: its player number (0 when every slot is taken) and how hard it rumbles
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ack {
    pub player: u8,
    pub rumble: u8,
}

impl Ack {
    pub fn encode(&self) -> [u8; ACK_LEN] {
        let [a, b, c, d] = ACK;
        [a, b, c, d, self.player, self.rumble]
    }

    pub fn decode(b: &[u8]) -> Option<Ack> {
        if b.len() < 5 || b[..4] != ACK {
            return None;
        }
        Some(Ack { player: b[4], rumble: b.get(5).copied().unwrap_or(0) })
    }
}

pub const BUTTONS: [(&str, u64); 8] = [("A", A), ("B", B), ("X", X), ("Y", Y), ("L", L), ("R", R), ("+", PLUS), ("−", MINUS)];
const HIT: f64 = 1.3;
const STICK_REACH: f64 = 2.5;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn distance(self, o: Point) -> f64 {
        (self.x - o.x).hypot(self.y - o.y)
    }
}

/// A round button, or a capsule `half` wider on each side
#[derive(Clone, Copy, Debug, Default)]
pub struct Control {
    pub bit: u64,
    pub label: &'static str,
    pub center: Point,
    pub radius: f64,
    pub half: f64,
}

impl Control {
    fn reach(&self, p: Point) -> f64 {
        ((p.x - self.center.x).abs() - self.half).max(0.0).hypot(p.y - self.center.y) / self.radius
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Layout {
    pub controls: [Control; BUTTONS.len()],
    pub stick: Point,
    pub stick_radius: f64,
    pub knob_radius: f64,
}

impl Layout {
    /// Landscape layout in points: stick bottom left, face buttons bottom right, shoulders and +/− along the top
    pub fn new(width: f64, height: f64, inset_left: f64, inset_right: f64) -> Layout {
        let (left, right) = (inset_left, width - inset_right);
        let u = height.min(width) / 10.0;
        let face = Point::new(right - 3.0 * u, height - 3.2 * u);
        let at = |dx: f64, dy: f64| Point::new(face.x + dx, face.y + dy);
        let spots = [
            (at(1.35 * u, 0.0), 0.75 * u, 0.0),
            (at(0.0, 1.35 * u), 0.75 * u, 0.0),
            (at(0.0, -1.35 * u), 0.75 * u, 0.0),
            (at(-1.35 * u, 0.0), 0.75 * u, 0.0),
            (Point::new(left + 2.6 * u, 1.2 * u), 0.75 * u, 1.0 * u),
            (Point::new(right - 2.6 * u, 1.2 * u), 0.75 * u, 1.0 * u),
            (Point::new(width / 2.0 + 1.2 * u, 0.9 * u), 0.5 * u, 0.0),
            (Point::new(width / 2.0 - 1.2 * u, 0.9 * u), 0.5 * u, 0.0),
        ];
        let controls = std::array::from_fn(|i| {
            let ((label, bit), (center, radius, half)) = (BUTTONS[i], spots[i]);
            Control { bit, label, center, radius, half }
        });
        Layout { controls, stick: Point::new(left + 3.2 * u, height - 3.2 * u), stick_radius: 1.6 * u, knob_radius: 0.7 * u }
    }

    pub fn hit(&self, p: Point) -> Option<&Control> {
        self.controls.iter().map(|c| (c, c.reach(p))).filter(|(_, d)| *d < HIT).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(c, _)| c)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    Button,
    Stick,
    Nothing,
}

/// Which finger holds what: buttons stay held until their finger lifts, the stick follows its finger from where it landed
#[derive(Clone, Debug, Default)]
pub struct Touches {
    layout: Layout,
    held: Vec<(u64, u64)>,
    stick: Option<u64>,
    origin: Point,
    knob: Point,
    axes: [f32; 2],
}

impl Touches {
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        if self.stick.is_none() {
            (self.origin, self.knob) = (layout.stick, layout.stick);
        }
    }

    pub fn begin(&mut self, id: u64, p: Point) -> Grab {
        if let Some(c) = self.layout.hit(p) {
            self.held.push((id, c.bit));
            Grab::Button
        } else if self.stick.is_none() && p.distance(self.layout.stick) / self.layout.stick_radius < STICK_REACH {
            self.stick = Some(id);
            self.origin = p;
            self.steer(p);
            Grab::Stick
        } else {
            Grab::Nothing
        }
    }

    pub fn moved(&mut self, id: u64, p: Point) -> bool {
        let ours = self.stick == Some(id);
        if ours {
            self.steer(p);
        }
        ours
    }

    pub fn end(&mut self, id: u64) {
        self.held.retain(|&(t, _)| t != id);
        if self.stick == Some(id) {
            self.stick = None;
            (self.origin, self.knob, self.axes) = (self.layout.stick, self.layout.stick, [0.0; 2]);
        }
    }

    pub fn release_all(&mut self) {
        self.held.clear();
        if let Some(id) = self.stick {
            self.end(id);
        }
    }

    fn steer(&mut self, p: Point) {
        let r = self.layout.stick_radius;
        let (mut dx, mut dy) = (p.x - self.origin.x, p.y - self.origin.y);
        let len = dx.hypot(dy);
        if len > r {
            (dx, dy) = (dx * r / len, dy * r / len);
        }
        self.knob = Point::new(self.origin.x + dx, self.origin.y + dy);
        self.axes = if r > 0.0 { [(dx / r) as f32, (-dy / r) as f32] } else { [0.0; 2] };
    }

    pub fn buttons(&self) -> u64 {
        self.held.iter().fold(0, |m, (_, b)| m | b)
    }

    pub fn axes(&self) -> [f32; 2] {
        self.axes
    }

    pub fn holds_stick(&self) -> bool {
        self.stick.is_some()
    }

    pub fn stick_origin(&self) -> Point {
        self.origin
    }

    pub fn knob(&self) -> Point {
        self.knob
    }

    pub fn pad(&self) -> Pad {
        let [x, y] = self.axes;
        Pad { buttons: self.buttons(), sticks: [x, y, 0.0, 0.0], motion: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_roundtrip() {
        let p = Pad { buttons: 0x8421, sticks: [0.5, -1.0, 0.25, 0.0], motion: Some(Motion { accel: [0.1, -0.98, 0.2], gyro: [1.5, -2.0, 0.3] }) };
        assert_eq!(Pad::decode(&p.encode()), Some(p));
        let still = Pad { motion: None, ..p };
        assert_eq!(Pad::decode(&still.encode()), Some(still));
        assert!(Pad::decode(b"nope").is_none());
        assert!(Pad::decode(&p.encode()[..PAD_LEN - 1]).is_none());
    }

    #[test]
    fn pad_is_clamped() {
        let mut b = Pad { sticks: [3.0, -3.0, f32::NAN, f32::INFINITY], ..Default::default() }.encode();
        b[28] = 1;
        b[29..33].copy_from_slice(&1e9f32.to_le_bytes());
        let p = Pad::decode(&b).unwrap();
        assert_eq!(p.sticks, [1.0, -1.0, 0.0, 0.0]);
        assert_eq!(p.motion.unwrap().accel[0], 64.0);
    }

    #[test]
    fn pad_wire_layout() {
        let b = Pad { buttons: A | PLUS, sticks: [1.0, 0.0, 0.0, 0.0], motion: None }.encode();
        assert_eq!(&b[..4], b"M8P1");
        assert_eq!(u64::from_le_bytes(b[4..12].try_into().unwrap()), 1 | 1 << 10);
        assert_eq!(f32::from_le_bytes(b[12..16].try_into().unwrap()), 1.0);
        assert_eq!(b[28], 0);
    }

    #[test]
    fn ack_roundtrip() {
        let a = Ack { player: 3, rumble: 200 };
        assert_eq!(Ack::decode(&a.encode()), Some(a));
        assert_eq!(Ack::decode(b"M8A1\x02"), Some(Ack { player: 2, rumble: 0 }));
        assert!(Ack::decode(b"M8P1\x02\x00").is_none());
    }

    fn touches() -> Touches {
        let mut t = Touches::default();
        t.set_layout(Layout::new(844.0, 390.0, 47.0, 47.0));
        t
    }

    #[test]
    fn layout_hits_every_button_at_its_center() {
        let l = Layout::new(844.0, 390.0, 47.0, 47.0);
        for c in &l.controls {
            assert_eq!(l.hit(c.center).map(|h| h.bit), Some(c.bit), "{}", c.label);
        }
        assert!(l.hit(Point::new(422.0, 200.0)).is_none());
        let a = l.controls.iter().find(|c| c.bit == A).unwrap();
        assert!(a.center.x > 600.0 && a.center.x < 797.0);
    }

    #[test]
    fn buttons_hold_until_their_finger_lifts() {
        let mut t = touches();
        let (a, b) = (t.layout().controls[0].center, t.layout().controls[1].center);
        assert_eq!(t.begin(1, a), Grab::Button);
        assert_eq!(t.begin(2, b), Grab::Button);
        assert_eq!(t.buttons(), A | B);
        assert!(!t.moved(1, Point::new(0.0, 0.0)));
        t.end(1);
        assert_eq!(t.buttons(), B);
        t.release_all();
        assert_eq!(t.buttons(), 0);
    }

    #[test]
    fn stick_follows_its_finger_and_clamps() {
        let mut t = touches();
        let home = t.layout().stick;
        let r = t.layout().stick_radius;
        let at = Point::new(home.x + 10.0, home.y);
        assert_eq!(t.begin(7, at), Grab::Stick);
        assert_eq!(t.stick_origin(), at);
        assert_eq!(t.axes(), [0.0, 0.0]);
        assert_eq!(t.begin(8, Point::new(home.x, home.y - 5.0)), Grab::Nothing, "one finger steers");
        assert!(t.moved(7, Point::new(at.x + r / 2.0, at.y - r / 2.0)));
        let [x, y] = t.axes();
        assert!((x - 0.5).abs() < 1e-6 && (y - 0.5).abs() < 1e-6);
        t.moved(7, Point::new(at.x + 10.0 * r, at.y));
        assert_eq!(t.axes(), [1.0, 0.0]);
        assert_eq!(t.knob(), Point::new(at.x + r, at.y));
        assert_eq!(t.pad().sticks, [1.0, 0.0, 0.0, 0.0]);
        t.end(7);
        assert_eq!((t.axes(), t.stick_origin(), t.knob()), ([0.0, 0.0], home, home));
        assert_eq!(t.begin(9, Point::new(home.x + 3.0 * r, home.y - 3.0 * r)), Grab::Nothing, "too far from the stick");
    }
}
