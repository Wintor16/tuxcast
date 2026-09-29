use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::portal::Portal;
use super::window::KWinWindow;
use crate::core::platform::{Capture, PlatformError, Result};
use crate::core::types::{Frame, PxRect, WindowInfo};

/// Below the reel tracker's ~60Hz polling, each `grab()` querying KWin
/// fresh would mean a script load/run/journalctl round-trip (several ms
/// to tens of ms, and KWin's scripting engine visibly can't keep up at
/// that rate — see the `KWinWindow` doc comment) on every single frame.
/// The window essentially never moves mid-cast, so refreshing this rarely
/// costs nothing in practice.
const WINDOW_CACHE_TTL: Duration = Duration::from_millis(250);

/// Reads frames out of the shared PipeWire stream obtained through the
/// portal, cropping the requested screen-absolute rect down to the
/// window-local coordinates the (window-scoped) stream uses.
pub struct PortalCapture {
    portal: Arc<Portal>,
    window: Arc<KWinWindow>,
    cached: Mutex<(Instant, Option<(WindowInfo, Option<String>)>)>,
    last_output: Mutex<Option<String>>,
}

impl PortalCapture {
    pub fn new(portal: Arc<Portal>, window: Arc<KWinWindow>) -> Self {
        Self {
            portal,
            window,
            cached: Mutex::new((Instant::now() - WINDOW_CACHE_TTL, None)),
            last_output: Mutex::new(None),
        }
    }

    fn window_info(&self) -> Option<(WindowInfo, Option<String>)> {
        let mut cache = self.cached.lock().unwrap();
        if cache.0.elapsed() >= WINDOW_CACHE_TTL {
            cache.1 = self.window.find_with_output();
            cache.0 = Instant::now();
        }
        cache.1.clone()
    }
}

impl Capture for PortalCapture {
    fn grab(&self, rect: PxRect) -> Result<Frame> {
        if rect.is_empty() {
            return Err(PlatformError::Capture("empty rect".into()));
        }

        let (win, current_output) = self.window_info().ok_or(PlatformError::WindowNotFound)?;

        // KWin's window-scoped PipeWire stream doesn't reliably keep
        // following a window once it's dragged onto a different physical
        // monitor, so a detected output change forces a pipeline rebuild
        // instead of waiting for the generic staleness check.
        {
            let mut last = self.last_output.lock().unwrap();
            if current_output.is_some() && *last != current_output && last.is_some() {
                self.portal.force_reconnect();
            }
            *last = current_output;
        }

        let frame = self
            .portal
            .grab_window()
            .ok_or_else(|| PlatformError::Capture("no frame available yet".into()))?;
        let local_x = (rect.x - win.client.x).max(0) as usize;
        let local_y = (rect.y - win.client.y).max(0) as usize;
        Ok(frame.crop(local_x, local_y, rect.w.max(0) as usize, rect.h.max(0) as usize))
    }
}
