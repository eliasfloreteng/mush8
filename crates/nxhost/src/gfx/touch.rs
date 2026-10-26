// Mush8 - touch.rs
// Constantin Clerc (c22dev) - 0.1
use crate::input;
use nxpad::{Grab, Layout, Point, Touches};
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSSet, NSString};
use objc2_ui_kit::{NSTextAlignment, UIColor, UIEvent, UIFont, UILabel, UIResponder, UITouch, UIView, UIViewAutoresizing};
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::Ordering::Relaxed;

const FILL: f64 = 0.35;
const RING: f64 = 0.8;
const PRESSED: f64 = 0.7;

struct Pad {
    view: Retained<TouchView>,
    buttons: Vec<Retained<UIView>>,
    base: Retained<UIView>,
    knob: Retained<UIView>,
    touches: Touches,
    shown: bool,
    screen: HashSet<usize>,
}

thread_local! {
    static PAD: RefCell<Option<Pad>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NXTouchView"]
    struct TouchView;

    impl TouchView {
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews(&self) {
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };
            layout();
        }

        #[unsafe(method(touchesBegan:withEvent:))]
        fn touches_began(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            handle(self, touches, Phase::Began);
        }

        #[unsafe(method(touchesMoved:withEvent:))]
        fn touches_moved(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            handle(self, touches, Phase::Moved);
        }

        #[unsafe(method(touchesEnded:withEvent:))]
        fn touches_ended(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            handle(self, touches, Phase::Ended);
        }

        #[unsafe(method(touchesCancelled:withEvent:))]
        fn touches_cancelled(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            handle(self, touches, Phase::Ended);
        }
    }
);

#[derive(PartialEq)]
enum Phase {
    Began,
    Moved,
    Ended,
}

fn circle(mtm: MainThreadMarker, parent: &UIView, title: &str, fill: (f64, f64), ring: f64) -> Retained<UIView> {
    let v = UIView::new(mtm);
    v.setUserInteractionEnabled(false);
    v.setBackgroundColor(Some(&UIColor::colorWithWhite_alpha(fill.0, fill.1)));
    let layer = v.layer();
    layer.setBorderWidth(2.0);
    let border = unsafe { UIColor::colorWithWhite_alpha(1.0, ring).CGColor() };
    layer.setBorderColor(Some(&border));
    if !title.is_empty() {
        let l = UILabel::new(mtm);
        l.setText(Some(&NSString::from_str(title)));
        unsafe { l.setTextColor(Some(&UIColor::whiteColor())) };
        l.setShadowColor(Some(&UIColor::colorWithWhite_alpha(0.0, 0.8)));
        l.setShadowOffset(CGSize::new(0.0, 1.0));
        l.setTextAlignment(NSTextAlignment::Center);
        l.setAdjustsFontSizeToFitWidth(true);
        v.addSubview(&l);
    }
    parent.addSubview(&v);
    v
}

fn place(v: &UIView, c: Point, r: f64) {
    place_capsule(v, c, r, 0.0);
}

fn place_capsule(v: &UIView, c: Point, r: f64, half: f64) {
    let w = 2.0 * (r + half);
    v.setFrame(CGRect::new(CGPoint::new(c.x - r - half, c.y - r), CGSize::new(w, 2.0 * r)));
    v.layer().setCornerRadius(r);
    if let Some(l) = v.subviews().firstObject().and_then(|l| l.downcast::<UILabel>().ok()) {
        l.setFrame(CGRect::new(CGPoint::new(2.0, 0.0), CGSize::new(w - 4.0, 2.0 * r)));
        unsafe { l.setFont(Some(&UIFont::boldSystemFontOfSize((r * 0.55).clamp(12.0, 22.0)))) };
    }
}

pub fn install(parent: &UIView, mtm: MainThreadMarker) {
    let view: Retained<TouchView> = unsafe { msg_send![TouchView::alloc(mtm), initWithFrame: parent.bounds()] };
    view.setMultipleTouchEnabled(true);
    view.setAutoresizingMask(UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight);
    parent.addSubview(&view);
    let buttons = nxpad::BUTTONS.iter().map(|&(t, _)| circle(mtm, &view, t, (0.0, FILL), RING)).collect();
    let base = circle(mtm, &view, "", (0.0, 0.25), 0.6);
    let knob = circle(mtm, &view, "", (1.0, 0.6), RING);
    let p = Pad { view, buttons, base, knob, touches: Touches::default(), shown: true, screen: HashSet::new() };
    show_controls(&p, false);
    PAD.with(|pad| *pad.borrow_mut() = Some(Pad { shown: false, ..p }));
}

fn show_controls(p: &Pad, visible: bool) {
    for v in p.buttons.iter().chain([&p.base, &p.knob]) {
        v.setHidden(!visible);
    }
}

pub fn set_visible(visible: bool) {
    PAD.with(|p| {
        let mut p = p.borrow_mut();
        let Some(p) = p.as_mut() else { return };
        show_controls(p, visible);
        p.shown = visible;
        if !visible {
            p.touches.release_all();
            publish(p);
        }
    });
}

fn layout() {
    PAD.with(|p| {
        let mut p = p.borrow_mut();
        let Some(p) = p.as_mut() else { return };
        let b = p.view.bounds().size;
        let inset = p.view.safeAreaInsets();
        p.touches.set_layout(Layout::new(b.width, b.height, inset.left, inset.right));
        for (v, c) in p.buttons.iter().zip(&p.touches.layout().controls) {
            place_capsule(v, c.center, c.radius, c.half);
        }
        publish(p);
    });
}

fn handle(view: &TouchView, touches: &NSSet<UITouch>, phase: Phase) {
    PAD.with(|p| {
        let mut p = p.borrow_mut();
        let Some(p) = p.as_mut() else { return };
        let b = view.bounds().size;
        for t in touches.allObjects().iter() {
            let id = &*t as *const UITouch as usize;
            let pt = t.locationInView(Some(view));
            let at = Point::new(pt.x, pt.y);
            match phase {
                Phase::Began => {
                    let grab = if p.shown { p.touches.begin(id as u64, at) } else { Grab::Nothing };
                    if grab == Grab::Nothing && !super::ios::CONTROLLER_MODE.load(Relaxed) {
                        let at = super::screen_point(pt.x, pt.y, b.width, b.height);
                        if super::on_screen(at) {
                            p.screen.insert(id);
                            input::screen_touch(id, Some(at));
                        }
                    }
                }
                Phase::Moved => {
                    if !p.touches.moved(id as u64, at) && p.screen.contains(&id) {
                        input::screen_touch(id, Some(super::screen_point(pt.x, pt.y, b.width, b.height)));
                    }
                }
                Phase::Ended => {
                    p.touches.end(id as u64);
                    if p.screen.remove(&id) {
                        input::screen_touch(id, None);
                    }
                }
            }
        }
        publish(p);
    });
}

fn publish(p: &Pad) {
    let held = p.touches.buttons();
    input::TOUCH_BUTTONS.store(held, Relaxed);
    let [x, y] = p.touches.axes();
    input::set_touch_stick(x, y);
    let layout = p.touches.layout();
    place(&p.base, p.touches.stick_origin(), layout.stick_radius);
    place(&p.knob, p.touches.knob(), layout.knob_radius);
    for (v, c) in p.buttons.iter().zip(&layout.controls) {
        let color = if held & c.bit != 0 { UIColor::colorWithWhite_alpha(1.0, PRESSED) } else { UIColor::colorWithWhite_alpha(0.0, FILL) };
        v.setBackgroundColor(Some(&color));
    }
}
