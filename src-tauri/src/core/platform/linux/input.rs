use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::portal::Portal;
use crate::core::platform::{GameWindow, Input};
use crate::core::types::{Key, MouseButton, PxPoint, PxRect};

/// See the matching constant in `capture.rs`: a KWin scripting round-trip
/// on every single call doesn't keep up with how often `move_to()` gets
/// called during reel tracking. The window essentially never moves
/// mid-cast, so a short-lived cache costs nothing in practice.
const WINDOW_CACHE_TTL: Duration = Duration::from_millis(250);

// Linux evdev codes (linux/input-event-codes.h). Stable across kernels.
const BTN_LEFT: i32 = 0x110;
const BTN_RIGHT: i32 = 0x111;

const KEY_ESC: i32 = 1;
const KEY_BACKSPACE: i32 = 14;
const KEY_ENTER: i32 = 28;
const KEY_LEFTCTRL: i32 = 29;
const KEY_LEFTSHIFT: i32 = 42;
const KEY_DELETE: i32 = 111;

/// Sends absolute pointer motion, buttons and keys through the
/// `org.freedesktop.portal.RemoteDesktop` session shared with capture.
///
/// Coordinates are translated from screen-absolute (what the rest of the
/// bot works in, matching the Windows backend) to window-local, since
/// `notify_pointer_motion_absolute` positions relative to the captured
/// stream, which is scoped to the game window.
pub struct PortalInput {
    portal: Arc<Portal>,
    window: Arc<dyn GameWindow>,
    last: (AtomicI32, AtomicI32),
    cached_origin: Mutex<(Instant, (i32, i32))>,
}

impl PortalInput {
    pub fn new(portal: Arc<Portal>, window: Arc<dyn GameWindow>) -> Self {
        Self {
            portal,
            window,
            last: (AtomicI32::new(0), AtomicI32::new(0)),
            cached_origin: Mutex::new((Instant::now() - WINDOW_CACHE_TTL, (0, 0))),
        }
    }

    fn to_local(&self, p: PxPoint) -> (f64, f64) {
        let mut cache = self.cached_origin.lock().unwrap();
        if cache.0.elapsed() >= WINDOW_CACHE_TTL {
            if let Some(w) = self.window.find() {
                cache.1 = (w.client.x, w.client.y);
            }
            cache.0 = Instant::now();
        }
        let (ox, oy) = cache.1;
        ((p.x - ox) as f64, (p.y - oy) as f64)
    }
}

impl Input for PortalInput {
    fn move_to(&self, p: PxPoint) {
        self.last.0.store(p.x, Ordering::Relaxed);
        self.last.1.store(p.y, Ordering::Relaxed);
        let (x, y) = self.to_local(p);
        self.portal.pointer_motion_absolute(x, y);
    }

    fn button(&self, b: MouseButton, down: bool) {
        let code = match b {
            MouseButton::Left => BTN_LEFT,
            MouseButton::Right => BTN_RIGHT,
        };
        self.portal.pointer_button(code, down);
    }

    fn key(&self, k: Key, down: bool) {
        match key_to_evdev(k) {
            Some((code, shift)) => {
                if shift && down {
                    self.portal.keyboard_keycode(KEY_LEFTSHIFT, true);
                }
                self.portal.keyboard_keycode(code, down);
                if shift && !down {
                    self.portal.keyboard_keycode(KEY_LEFTSHIFT, false);
                }
            }
            None => {}
        }
    }

    fn wheel(&self, delta: i32) {
        // Windows wheel deltas are multiples of 120; portal axis steps are
        // small integers (roughly one notch each).
        let steps = (delta / 120).clamp(-20, 20);
        if steps != 0 {
            self.portal.pointer_axis_discrete(-steps);
        }
    }

    fn type_text(&self, s: &str) {
        for c in s.chars() {
            if let Some((code, shift)) = key_to_evdev(Key::Char(c)) {
                if shift {
                    self.portal.keyboard_keycode(KEY_LEFTSHIFT, true);
                }
                self.portal.keyboard_keycode(code, true);
                self.portal.keyboard_keycode(code, false);
                if shift {
                    self.portal.keyboard_keycode(KEY_LEFTSHIFT, false);
                }
            }
        }
    }

    fn cursor(&self) -> PxPoint {
        PxPoint { x: self.last.0.load(Ordering::Relaxed), y: self.last.1.load(Ordering::Relaxed) }
    }

    fn screen(&self) -> PxRect {
        // Unused by bot logic today (only screen-relative absolute moves
        // matter, and those go through the window-scoped portal stream), so
        // a generous placeholder is enough.
        PxRect { x: 0, y: 0, w: 7680, h: 4320 }
    }
}

/// Returns the evdev keycode and whether Shift is needed.
fn key_to_evdev(k: Key) -> Option<(i32, bool)> {
    match k {
        Key::Backspace => Some((KEY_BACKSPACE, false)),
        Key::Delete => Some((KEY_DELETE, false)),
        Key::Enter => Some((KEY_ENTER, false)),
        Key::Escape => Some((KEY_ESC, false)),
        Key::Control => Some((KEY_LEFTCTRL, false)),
        Key::Char(c) => char_to_evdev(c),
    }
}

fn char_to_evdev(c: char) -> Option<(i32, bool)> {
    const ROW1: [i32; 10] = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11]; // 1..0
    let lower = c.to_ascii_lowercase();
    if lower.is_ascii_digit() {
        let idx = (lower as u8 - b'0') as usize;
        let code = if idx == 0 { ROW1[9] } else { ROW1[idx - 1] };
        return Some((code, false));
    }
    if lower.is_ascii_lowercase() {
        let code = match lower {
            'q' => 16, 'w' => 17, 'e' => 18, 'r' => 19, 't' => 20, 'y' => 21, 'u' => 22,
            'i' => 23, 'o' => 24, 'p' => 25, 'a' => 30, 's' => 31, 'd' => 32, 'f' => 33,
            'g' => 34, 'h' => 35, 'j' => 36, 'k' => 37, 'l' => 38, 'z' => 44, 'x' => 45,
            'c' => 46, 'v' => 47, 'b' => 48, 'n' => 49, 'm' => 50,
            _ => return None,
        };
        return Some((code, c.is_ascii_uppercase()));
    }
    let (code, shift) = match c {
        ' ' => (57, false),
        '-' => (12, false),
        '_' => (12, true),
        '=' => (13, false),
        '+' => (13, true),
        '[' => (26, false),
        '{' => (26, true),
        ']' => (27, false),
        '}' => (27, true),
        ';' => (39, false),
        ':' => (39, true),
        '\'' => (40, false),
        '"' => (40, true),
        '`' => (41, false),
        '~' => (41, true),
        '\\' => (43, false),
        '|' => (43, true),
        ',' => (51, false),
        '<' => (51, true),
        '.' => (52, false),
        '>' => (52, true),
        '/' => (53, false),
        '?' => (53, true),
        '!' => (2, true),
        '@' => (3, true),
        '#' => (4, true),
        '$' => (5, true),
        '%' => (6, true),
        '^' => (7, true),
        '&' => (8, true),
        '*' => (9, true),
        '(' => (10, true),
        ')' => (11, true),
        _ => return None,
    };
    Some((code, shift))
}
