use std::os::fd::IntoRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop};
use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::desktop::{PersistMode, Session};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use crate::core::types::Frame;

/// Frames older than this are treated as a dead stream and trigger a
/// pipeline reconnect (KWin's window-scoped PipeWire stream can stop
/// delivering frames when the window is dragged to a different output).
const STALE_AFTER: Duration = Duration::from_secs(2);
/// Don't hammer the portal with reconnect attempts while genuinely stuck
/// (e.g. the game window minimized).
const RECONNECT_COOLDOWN: Duration = Duration::from_secs(2);

/// Shared window-scoped screen capture + input-injection session, obtained
/// once through the XDG desktop portal (RemoteDesktop + ScreenCast combined).
///
/// Connecting is lazy: it only happens on first use, which is what makes
/// KDE show its "share your screen" dialog. On success KDE hands us a
/// restore token that we persist, so later runs reconnect silently.
pub struct Portal {
    state: Mutex<Option<Arc<Connected>>>,
    connecting: AtomicBool,
}

struct Connected {
    rt: tokio::runtime::Runtime,
    remote_desktop: RemoteDesktop<'static>,
    screencast: Screencast<'static>,
    session: Session<'static, RemoteDesktop<'static>>,
    stream_node_id: u32,
    stream_size: (i32, i32),
    latest_frame: Arc<Mutex<Option<Frame>>>,
    last_frame_at: Arc<Mutex<Instant>>,
    last_reconnect_attempt: Mutex<Instant>,
    pipeline: Mutex<gst::Pipeline>,
}

fn token_path() -> std::path::PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("gpo-autofish").join("portal_token")
}

impl Portal {
    pub fn new() -> Self {
        Self { state: Mutex::new(None), connecting: AtomicBool::new(false) }
    }

    fn connected(&self) -> Option<Arc<Connected>> {
        {
            let guard = self.state.lock().unwrap();
            if let Some(c) = guard.as_ref() {
                return Some(c.clone());
            }
        }
        // Only one caller should drive the (blocking, user-interactive) setup.
        if self.connecting.swap(true, Ordering::SeqCst) {
            // Someone else is already connecting; wait for them.
            for _ in 0..600 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let guard = self.state.lock().unwrap();
                if let Some(c) = guard.as_ref() {
                    return Some(c.clone());
                }
            }
            return None;
        }
        let result = Connected::establish();
        self.connecting.store(false, Ordering::SeqCst);
        match result {
            Ok(c) => {
                let c = Arc::new(c);
                *self.state.lock().unwrap() = Some(c.clone());
                Some(c)
            }
            Err(e) => {
                tracing::warn!("portal session failed: {e}");
                None
            }
        }
    }

    pub fn grab_window(&self) -> Option<Frame> {
        let c = self.connected()?;
        if c.last_frame_at.lock().unwrap().elapsed() > STALE_AFTER {
            c.reconnect_pipeline(false);
        }
        let f = c.latest_frame.lock().unwrap();
        f.clone()
    }

    pub fn pointer_motion_absolute(&self, x: f64, y: f64) {
        if let Some(c) = self.connected() {
            let _ = c.rt.block_on(c.remote_desktop.notify_pointer_motion_absolute(
                &c.session,
                c.stream_node_id,
                x,
                y,
            ));
        }
    }

    pub fn pointer_button(&self, evdev_button: i32, pressed: bool) {
        if let Some(c) = self.connected() {
            let state = if pressed { KeyState::Pressed } else { KeyState::Released };
            let _ = c.rt.block_on(c.remote_desktop.notify_pointer_button(&c.session, evdev_button, state));
        }
    }

    pub fn pointer_axis_discrete(&self, steps: i32) {
        if let Some(c) = self.connected() {
            let _ = c.rt.block_on(c.remote_desktop.notify_pointer_axis_discrete(
                &c.session,
                ashpd::desktop::remote_desktop::Axis::Vertical,
                steps,
            ));
        }
    }

    pub fn keyboard_keycode(&self, evdev_keycode: i32, pressed: bool) {
        if let Some(c) = self.connected() {
            let state = if pressed { KeyState::Pressed } else { KeyState::Released };
            let _ = c.rt.block_on(c.remote_desktop.notify_keyboard_keycode(&c.session, evdev_keycode, state));
        }
    }

    /// Rebuilds the capture pipeline right away, bypassing the reconnect
    /// cooldown. For a known, discrete trigger (the window changed output)
    /// rather than the generic "no frames in a while" staleness check.
    pub fn force_reconnect(&self) {
        if let Some(c) = self.connected() {
            c.reconnect_pipeline(true);
        }
    }
}

impl Connected {
    fn establish() -> ashpd::Result<Self> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| {
            ashpd::Error::Portal(ashpd::PortalError::Failed(e.to_string()))
        })?;

        let (remote_desktop, screencast, session, stream_node_id, size, pw_fd) = rt.block_on(async {
            let remote_desktop = RemoteDesktop::new().await?;
            let screencast = Screencast::new().await?;
            let session = remote_desktop.create_session().await?;

            let saved_token = std::fs::read_to_string(token_path()).ok();

            // xdg-desktop-portal-kde rejects a persistent remote-input
            // grant ("Remote desktop sessions cannot persist") even though
            // it happily persists screen capture below, so device access
            // is re-approved every run while the capture grant survives.
            remote_desktop
                .select_devices(
                    &session,
                    DeviceType::Keyboard | DeviceType::Pointer,
                    None,
                    PersistMode::DoNot,
                )
                .await?;
            screencast
                .select_sources(
                    &session,
                    CursorMode::Metadata,
                    SourceType::Window.into(),
                    false,
                    saved_token.as_deref(),
                    PersistMode::DoNot,
                )
                .await?;

            let response = remote_desktop.start(&session, &ashpd::WindowIdentifier::default()).await?.response()?;

            if let Some(token) = response.restore_token() {
                if let Some(parent) = token_path().parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(token_path(), token);
            }

            let streams = response.streams().ok_or_else(|| {
                ashpd::Error::Portal(ashpd::PortalError::Failed("no screencast stream granted".into()))
            })?;
            let stream = streams.first().ok_or_else(|| {
                ashpd::Error::Portal(ashpd::PortalError::Failed("empty stream list".into()))
            })?;
            let node_id = stream.pipe_wire_node_id();
            let size = stream.size().unwrap_or((1920, 1080));

            let fd = screencast.open_pipe_wire_remote(&session).await?;

            ashpd::Result::Ok((remote_desktop, screencast, session, node_id, size, fd))
        })?;

        let latest_frame: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
        let last_frame_at = Arc::new(Mutex::new(Instant::now()));
        let pipeline = start_capture_pipeline(
            pw_fd.into_raw_fd(),
            stream_node_id,
            size,
            latest_frame.clone(),
            last_frame_at.clone(),
        )
        .map_err(|e| ashpd::Error::Portal(ashpd::PortalError::Failed(e.to_string())))?;

        Ok(Self {
            rt,
            remote_desktop,
            screencast,
            session,
            stream_node_id,
            stream_size: size,
            latest_frame,
            last_frame_at,
            last_reconnect_attempt: Mutex::new(Instant::now() - RECONNECT_COOLDOWN),
            pipeline: Mutex::new(pipeline),
        })
    }

    /// KWin's window-scoped PipeWire stream can go quiet when the window
    /// crosses to a different monitor (output change on the compositor
    /// side); re-opening the PipeWire remote and rebuilding just the
    /// GStreamer pipeline recovers without re-running the portal dialog.
    fn reconnect_pipeline(&self, force: bool) {
        {
            let mut last = self.last_reconnect_attempt.lock().unwrap();
            if !force && last.elapsed() < RECONNECT_COOLDOWN {
                return;
            }
            *last = Instant::now();
        }

        let fd = match self.rt.block_on(self.screencast.open_pipe_wire_remote(&self.session)) {
            Ok(fd) => fd,
            Err(e) => {
                tracing::warn!("capture reconnect: could not reopen pipewire remote: {e}");
                return;
            }
        };

        match start_capture_pipeline(
            fd.into_raw_fd(),
            self.stream_node_id,
            self.stream_size,
            self.latest_frame.clone(),
            self.last_frame_at.clone(),
        ) {
            Ok(new_pipeline) => {
                let old = {
                    let mut guard = self.pipeline.lock().unwrap();
                    std::mem::replace(&mut *guard, new_pipeline)
                };
                let _ = old.set_state(gst::State::Null);
                tracing::info!("capture pipeline reconnected after a stale frame");
            }
            Err(e) => tracing::warn!("capture reconnect: pipeline rebuild failed: {e}"),
        }
    }
}

/// Packaged builds (AppImage in particular) bundle a copy of libgstreamer
/// itself, which then looks for plugins relative to *its own* location —
/// not the distro's `/usr/lib/gstreamer-1.0` — so `pipewiresrc` (from the
/// separately-packaged `gstreamer1.0-pipewire` /
/// `gst-plugin-pipewire` distro package) silently fails to resolve even
/// though it's genuinely installed on the system.
///
/// A first attempt at this fix skipped entirely whenever
/// `GST_PLUGIN_SYSTEM_PATH(_1_0)` was already set, meaning to avoid
/// clobbering a deliberate user override — but linuxdeploy's own GStreamer
/// bundling plugin *always* sets these (pointing only at the AppImage's
/// own bundled plugin dir, which doesn't include pipewiresrc), so that
/// guard made the fix a no-op in the one build that actually needed it
/// (confirmed: reproduced from the published AppImage with this exact
/// env already set to the bundled-only path). Appending the system
/// directories instead — rather than skipping or overwriting — fixes
/// that case while still leaving a real user override intact.
fn ensure_gst_plugin_path() {
    const CANDIDATES: &[&str] = &[
        "/usr/lib/gstreamer-1.0",
        "/usr/lib/x86_64-linux-gnu/gstreamer-1.0",
        "/usr/lib64/gstreamer-1.0",
        "/usr/lib/aarch64-linux-gnu/gstreamer-1.0",
    ];
    let existing: Vec<&str> = CANDIDATES.iter().copied().filter(|p| std::path::Path::new(p).is_dir()).collect();
    if existing.is_empty() {
        return;
    }
    let extra = existing.join(":");
    for key in ["GST_PLUGIN_SYSTEM_PATH_1_0", "GST_PLUGIN_SYSTEM_PATH"] {
        let combined = match std::env::var(key) {
            Ok(current) if !current.is_empty() => format!("{current}:{extra}"),
            _ => extra.clone(),
        };
        std::env::set_var(key, combined);
    }
}

fn start_capture_pipeline(
    fd: i32,
    node_id: u32,
    (w, h): (i32, i32),
    latest: Arc<Mutex<Option<Frame>>>,
    last_frame_at: Arc<Mutex<Instant>>,
) -> Result<gst::Pipeline, String> {
    ensure_gst_plugin_path();
    gst::init().map_err(|e| e.to_string())?;

    let pipeline = gst::Pipeline::new();
    let src = gst::ElementFactory::make("pipewiresrc")
        .property("fd", fd)
        .property("path", node_id.to_string())
        .build()
        .map_err(|e| format!("pipewiresrc: {e}"))?;
    let convert = gst::ElementFactory::make("videoconvert").build().map_err(|e| e.to_string())?;
    let caps = gst::Caps::builder("video/x-raw").field("format", "RGBA").build();
    let capsfilter =
        gst::ElementFactory::make("capsfilter").property("caps", &caps).build().map_err(|e| e.to_string())?;
    let sink = gst_app::AppSink::builder().caps(&caps).max_buffers(1).drop(true).sync(false).build();

    pipeline
        .add_many([&src, &convert, &capsfilter, sink.upcast_ref()])
        .map_err(|e| e.to_string())?;
    gst::Element::link_many([&src, &convert, &capsfilter, sink.upcast_ref()]).map_err(|e| e.to_string())?;

    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                let s = caps.structure(0).ok_or(gst::FlowError::Error)?;
                let width: i32 = s.get("width").unwrap_or(w);
                let height: i32 = s.get("height").unwrap_or(h);
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let expected = (width as usize) * (height as usize) * 4;
                if map.len() >= expected {
                    let frame = Frame::new(width as usize, height as usize, map[..expected].to_vec());
                    *latest.lock().unwrap() = Some(frame);
                    *last_frame_at.lock().unwrap() = Instant::now();
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    pipeline.set_state(gst::State::Playing).map_err(|e| e.to_string())?;

    // appsink's new-sample callback fires from GStreamer's own streaming
    // thread, so frames keep landing in `latest` without us running any
    // main loop. This thread just logs pipeline errors/EOS for diagnostics.
    let pipeline_weak = pipeline.downgrade();
    std::thread::spawn(move || {
        let Some(pipeline) = pipeline_weak.upgrade() else { return };
        let Some(bus) = pipeline.bus() else { return };
        for msg in bus.iter_timed(gst::ClockTime::NONE) {
            use gst::MessageView;
            match msg.view() {
                MessageView::Eos(_) => {
                    tracing::warn!("capture pipeline reached EOS");
                    break;
                }
                MessageView::Error(e) => {
                    tracing::warn!("capture pipeline error: {} ({:?})", e.error(), e.debug());
                    break;
                }
                _ => {}
            }
        }
    });

    Ok(pipeline)
}
