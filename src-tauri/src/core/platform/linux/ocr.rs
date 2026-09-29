use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::core::platform::{Ocr, PlatformError, Result};
use crate::core::types::Frame;

/// Shells out to the `tesseract` CLI (5.x). No Windows-OCR equivalent ships
/// on Linux, and `tesseract` is the standard, widely-packaged alternative.
pub struct TesseractOcr {
    available: OnceLock<bool>,
}

impl TesseractOcr {
    pub fn new() -> Self {
        Self { available: OnceLock::new() }
    }
}

impl Ocr for TesseractOcr {
    fn available(&self) -> bool {
        *self.available.get_or_init(|| {
            let ok = Command::new("tesseract")
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                tracing::warn!("tesseract not found on PATH; fruit detection disabled. Install the 'tesseract' package.");
            }
            ok
        })
    }

    fn read(&self, frame: &Frame) -> Result<String> {
        if !self.available() {
            return Err(PlatformError::OcrUnavailable);
        }
        if frame.w < 8 || frame.h < 8 {
            return Ok(String::new());
        }

        let png = encode_png(frame).map_err(|e| PlatformError::Ocr(e.to_string()))?;

        let mut child = Command::new("tesseract")
            .args(["stdin", "stdout", "--psm", "6"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| PlatformError::Ocr(e.to_string()))?;

        child
            .stdin
            .take()
            .ok_or_else(|| PlatformError::Ocr("no stdin".into()))?
            .write_all(&png)
            .map_err(|e| PlatformError::Ocr(e.to_string()))?;

        let out = child.wait_with_output().map_err(|e| PlatformError::Ocr(e.to_string()))?;
        if !out.status.success() {
            return Err(PlatformError::Ocr(String::from_utf8_lossy(&out.stderr).into_owned()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

fn encode_png(frame: &Frame) -> std::result::Result<Vec<u8>, image::ImageError> {
    let img = image::RgbaImage::from_raw(frame.w as u32, frame.h as u32, frame.rgba.clone())
        .expect("Frame invariant: rgba.len() == w*h*4");
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}
