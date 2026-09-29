use std::sync::Arc;

use crate::core::types::{Frame, Key, MouseButton, PxPoint, PxRect, WindowInfo};

pub mod mock;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("game window not found")]
    WindowNotFound,
    #[error("capture failed: {0}")]
    Capture(String),
    #[error("ocr unavailable")]
    OcrUnavailable,
    #[error("ocr failed: {0}")]
    Ocr(String),
    #[error("input failed: {0}")]
    Input(String),
}

pub type Result<T> = std::result::Result<T, PlatformError>;

pub trait GameWindow: Send + Sync {
    fn find(&self) -> Option<WindowInfo>;
    fn focus(&self) -> bool;
}

pub trait Capture: Send + Sync {
    fn grab(&self, rect: PxRect) -> Result<Frame>;
}

pub trait Input: Send + Sync {
    fn move_to(&self, p: PxPoint);
    fn button(&self, b: MouseButton, down: bool);
    fn key(&self, k: Key, down: bool);
    fn wheel(&self, delta: i32);
    fn type_text(&self, s: &str);
    fn cursor(&self) -> PxPoint;
    fn screen(&self) -> PxRect;
}

pub trait Ocr: Send + Sync {
    fn available(&self) -> bool;
    fn read(&self, frame: &Frame) -> Result<String>;
}

#[derive(Clone)]
pub struct Platform {
    pub window: Arc<dyn GameWindow>,
    pub capture: Arc<dyn Capture>,
    pub input: Arc<dyn Input>,
    pub ocr: Arc<dyn Ocr>,
}

#[cfg(windows)]
pub fn build() -> Platform {
    Platform {
        window: Arc::new(windows::window::Win32Window::new("Roblox")),
        capture: Arc::new(windows::capture_dxgi::DxgiCapture::new()),
        input: Arc::new(windows::input::SendInputBackend::new()),
        ocr: build_ocr(),
    }
}

#[cfg(all(windows, feature = "ocr-windows", not(feature = "ocr-none")))]
fn build_ocr() -> Arc<dyn Ocr> {
    Arc::new(windows::ocr::WindowsOcr::new())
}

#[cfg(all(windows, any(feature = "ocr-none", not(feature = "ocr-windows"))))]
fn build_ocr() -> Arc<dyn Ocr> {
    Arc::new(mock::NoOcr)
}

#[cfg(target_os = "linux")]
pub fn build() -> Platform {
    let window = Arc::new(linux::window::KWinWindow::new("Roblox"));
    let portal = Arc::new(linux::portal::Portal::new());
    Platform {
        window: window.clone() as Arc<dyn GameWindow>,
        capture: Arc::new(linux::capture::PortalCapture::new(portal.clone(), window.clone())),
        input: Arc::new(linux::input::PortalInput::new(portal, window as Arc<dyn GameWindow>)),
        ocr: Arc::new(linux::ocr::TesseractOcr::new()),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn build() -> Platform {
    mock::platform()
}
