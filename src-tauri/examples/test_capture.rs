//! Standalone, screencast-only capture probe used to iterate on the
//! monitor-follow / reconnect logic without needing a fresh portal consent
//! dialog on every run: unlike the real app's combined RemoteDesktop +
//! ScreenCast session (which KDE refuses to persist), a capture-only
//! session *can* persist, so after the first approval this keeps working
//! non-interactively. Writes the latest frame to /tmp/gpoaf_test_frame.png
//! once a second and logs window/output/frame-size on every write so a
//! monitor move shows up immediately in the log.

use std::io::Write as _;
use std::os::fd::IntoRawFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::desktop::PersistMode;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use gpo_autofish_lib::core::platform::linux::window::KWinWindow;
use gpo_autofish_lib::core::platform::GameWindow;
use gpo_autofish_lib::core::types::Frame;

fn token_path() -> std::path::PathBuf {
    std::env::temp_dir().join("gpoaf_test_capture_token")
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    let (node_id, size, fd) = rt
        .block_on(async {
            let screencast = Screencast::new().await?;
            let session = screencast.create_session().await?;
            let saved_token = std::fs::read_to_string(token_path()).ok();
            screencast
                .select_sources(
                    &session,
                    CursorMode::Metadata,
                    SourceType::Window.into(),
                    false,
                    saved_token.as_deref(),
                    PersistMode::ExplicitlyRevoked,
                )
                .await?;
            let response = screencast.start(&session, &ashpd::WindowIdentifier::default()).await?.response()?;
            if let Some(token) = response.restore_token() {
                let _ = std::fs::write(token_path(), token);
                println!("saved restore token ({} bytes)", token.len());
            }
            let stream = response.streams().first().cloned().ok_or_else(|| {
                ashpd::Error::Portal(ashpd::PortalError::Failed("no stream".into()))
            })?;
            let node_id = stream.pipe_wire_node_id();
            let size = stream.size().unwrap_or((1920, 1080));
            let fd = screencast.open_pipe_wire_remote(&session).await?;
            ashpd::Result::Ok((node_id, size, fd))
        })
        .expect("portal handshake failed");

    println!("connected: node_id={node_id} size={size:?}");

    gst::init().unwrap();
    let pipeline = gst::Pipeline::new();
    let src = gst::ElementFactory::make("pipewiresrc")
        .property("fd", fd.into_raw_fd())
        .property("path", node_id.to_string())
        .build()
        .unwrap();
    let convert = gst::ElementFactory::make("videoconvert").build().unwrap();
    let caps = gst::Caps::builder("video/x-raw").field("format", "RGBA").build();
    let capsfilter = gst::ElementFactory::make("capsfilter").property("caps", &caps).build().unwrap();
    let sink = gst_app::AppSink::builder().caps(&caps).max_buffers(1).drop(true).sync(false).build();
    pipeline.add_many([&src, &convert, &capsfilter, sink.upcast_ref()]).unwrap();
    gst::Element::link_many([&src, &convert, &capsfilter, sink.upcast_ref()]).unwrap();

    let latest: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
    let last_frame_at = Arc::new(Mutex::new(Instant::now()));
    let latest_cb = latest.clone();
    let last_frame_at_cb = last_frame_at.clone();
    let (w0, h0) = size;
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                let s = caps.structure(0).ok_or(gst::FlowError::Error)?;
                let width: i32 = s.get("width").unwrap_or(w0);
                let height: i32 = s.get("height").unwrap_or(h0);
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let expected = (width as usize) * (height as usize) * 4;
                if map.len() >= expected {
                    *latest_cb.lock().unwrap() =
                        Some(Frame::new(width as usize, height as usize, map[..expected].to_vec()));
                    *last_frame_at_cb.lock().unwrap() = Instant::now();
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    pipeline.set_state(gst::State::Playing).unwrap();

    let window = KWinWindow::new("Roblox");
    let mut last_output = String::new();
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let age = last_frame_at.lock().unwrap().elapsed();
        let output = window.current_output().unwrap_or_default();
        let win = GameWindow::find(&window);
        if output != last_output {
            println!(">>> output changed: {last_output:?} -> {output:?}");
            last_output = output.clone();
        }
        println!(
            "frame_age={:?} output={output:?} window={:?}",
            age,
            win.map(|w| (w.client.x, w.client.y, w.client.w, w.client.h))
        );
        if let Some(frame) = latest.lock().unwrap().clone() {
            let img = image::RgbaImage::from_raw(frame.w as u32, frame.h as u32, frame.rgba.clone()).unwrap();
            let mut f = std::fs::File::create("/tmp/gpoaf_test_frame.png").unwrap();
            let mut buf = std::io::Cursor::new(Vec::new());
            img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
            f.write_all(&buf.into_inner()).unwrap();
        }
    }
}
