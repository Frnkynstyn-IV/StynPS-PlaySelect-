//! Pure input-remapping logic (no I/O), so it can be unit-tested.

use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::Arc,
};

use evdev::{EventType, InputEvent, Key};

pub const ABS_Z: u16 = 2;
pub const ABS_RZ: u16 = 5;
pub const ABS_HAT0X: u16 = 16;
pub const ABS_HAT0Y: u16 = 17;

const TRIGGER_PRESS: i32 = 128;
const TRIGGER_RELEASE: i32 = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Src {
    Key(u16),
    Hat(u16, i32),
    Trigger(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dst {
    Key(u16),
    Hat(u16, i32),
}

fn parse_hat(name: &str) -> Option<(u16, i32)> {
    match name {
        "ABS_HAT0Y_UP" => Some((ABS_HAT0Y, -1)),
        "ABS_HAT0Y_DOWN" => Some((ABS_HAT0Y, 1)),
        "ABS_HAT0X_LEFT" => Some((ABS_HAT0X, -1)),
        "ABS_HAT0X_RIGHT" => Some((ABS_HAT0X, 1)),
        _ => None,
    }
}

pub fn parse_src(name: &str) -> Option<Src> {
    if let Some((axis, dir)) = parse_hat(name) { return Some(Src::Hat(axis, dir)); }
    match name {
        "ABS_Z" => return Some(Src::Trigger(ABS_Z)),
        "ABS_RZ" => return Some(Src::Trigger(ABS_RZ)),
        _ => {}
    }
    Key::from_str(name).ok().map(|k| Src::Key(k.code()))
}

pub fn parse_dst(name: &str) -> Option<Dst> {
    if let Some((axis, dir)) = parse_hat(name) { return Some(Dst::Hat(axis, dir)); }
    Key::from_str(name).ok().map(|k| Dst::Key(k.code()))
}

#[derive(Default, Debug)]
pub struct RemapConfig {
    map: HashMap<Src, Dst>,
}

impl RemapConfig {
    pub fn from_names(names: &HashMap<String, String>) -> Arc<Self> {
        let mut map = HashMap::new();
        for (from, to) in names {
            if from == to { continue; }
            if let (Some(src), Some(dst)) = (parse_src(from), parse_dst(to)) { map.insert(src, dst); }
        }
        Arc::new(Self { map })
    }

    #[cfg(test)]
    pub fn len(&self) -> usize { self.map.len() }
}

fn ev(kind: EventType, code: u16, value: i32) -> InputEvent { InputEvent::new(kind, code, value) }

pub struct Remapper {
    cfg: Arc<RemapConfig>,
    hat: HashMap<u16, i32>,
    trig_down: HashSet<u16>,
    held: HashSet<Dst>,
}

impl Remapper {
    pub fn new(cfg: Arc<RemapConfig>) -> Self { Self { cfg, hat: HashMap::new(), trig_down: HashSet::new(), held: HashSet::new() } }

    pub fn set_config(&mut self, cfg: Arc<RemapConfig>, out: &mut Vec<InputEvent>) {
        self.release_all(out); self.trig_down.clear(); self.cfg = cfg;
    }

    pub fn release_all(&mut self, out: &mut Vec<InputEvent>) {
        let held: Vec<Dst> = self.held.drain().collect();
        for d in held {
            match d {
                Dst::Key(c) => out.push(ev(EventType::KEY, c, 0)),
                Dst::Hat(axis, _) => out.push(ev(EventType::ABSOLUTE, axis, 0)),
            }
        }
    }

    fn set_dst(&mut self, dst: Dst, pressed: bool, out: &mut Vec<InputEvent>) {
        match dst {
            Dst::Key(c) => out.push(ev(EventType::KEY, c, pressed as i32)),
            Dst::Hat(axis, dir) => out.push(ev(EventType::ABSOLUTE, axis, if pressed { dir } else { 0 })),
        }
        if pressed { self.held.insert(dst); } else { self.held.remove(&dst); }
    }

    pub fn process(&mut self, e: InputEvent, out: &mut Vec<InputEvent>) {
        match e.event_type() {
            EventType::KEY => match self.cfg.map.get(&Src::Key(e.code())).copied() {
                Some(dst) => { if e.value() != 2 { self.set_dst(dst, e.value() != 0, out); } }
                None => out.push(e),
            },
            EventType::ABSOLUTE => match e.code() {
                ABS_HAT0X | ABS_HAT0Y => self.hat_event(e.code(), e.value().signum(), out),
                ABS_Z | ABS_RZ => self.trigger_event(e, out),
                _ => out.push(e),
            },
            _ => {}
        }
    }

    fn hat_event(&mut self, axis: u16, v: i32, out: &mut Vec<InputEvent>) {
        let prev = self.hat.insert(axis, v).unwrap_or(0);
        if prev != 0 { if let Some(d) = self.cfg.map.get(&Src::Hat(axis, prev)).copied() { self.set_dst(d, false, out); } }
        let mapped = v != 0 && self.cfg.map.contains_key(&Src::Hat(axis, v));
        out.push(ev(EventType::ABSOLUTE, axis, if mapped { 0 } else { v }));
        if mapped { if let Some(d) = self.cfg.map.get(&Src::Hat(axis, v)).copied() { self.set_dst(d, true, out); } }
    }

    fn trigger_event(&mut self, e: InputEvent, out: &mut Vec<InputEvent>) {
        let axis = e.code();
        match self.cfg.map.get(&Src::Trigger(axis)).copied() {
            None => out.push(e),
            Some(dst) => {
                let down = self.trig_down.contains(&axis);
                if !down && e.value() >= TRIGGER_PRESS { self.trig_down.insert(axis); self.set_dst(dst, true, out); }
                else if down && e.value() <= TRIGGER_RELEASE { self.trig_down.remove(&axis); self.set_dst(dst, false, out); }
                out.push(ev(EventType::ABSOLUTE, axis, 0));
            }
        }
    }
}
