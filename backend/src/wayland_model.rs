//! The Shell protocol uses logical coordinates and lossless PNG frame files.
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;

#[cfg(target_os = "linux")]
pub const EXTENSION_ID: &str = "lolping-lite@ezreal-byte.github.io";
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_ENCODED_BYTES: usize = MAX_FRAME_BYTES + 65536;

pub fn encode_frame(pixels: &tiny_skia::Pixmap) -> Result<Vec<u8>, String> {
    frame_bytes(pixels.width(), pixels.height())?;
    // tiny-skia converts premultiplied pixels back to PNG RGBA. Shell's native
    // Cairo decoder premultiplies again without allocating a JS pixel array.
    // Balanced compression costs more than a whole frame on a 600px wheel.
    // Fast changes compression effort only, retaining the same lossless RGBA.
    let rgba = pixels.clone().take_demultiplied();
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, pixels.width(), pixels.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("Cannot encode Wayland frame: {e}"))?;
    writer
        .write_image_data(&rgba)
        .map_err(|e| format!("Cannot encode Wayland frame: {e}"))?;
    writer
        .finish()
        .map_err(|e| format!("Cannot encode Wayland frame: {e}"))?;
    if encoded.len() > MAX_ENCODED_BYTES {
        return Err("Wayland encoded overlay exceeds the bounded frame size".into());
    }
    Ok(encoded)
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Monitor {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}
impl Monitor {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64
            && y >= self.y as f64
            && x < self.x as f64 + self.width as f64
            && y < self.y as f64 + self.height as f64
    }
    pub fn pixels(&self) -> (u32, u32) {
        (
            (self.width as f64 * self.scale).round() as u32,
            (self.height as f64 * self.scale).round() as u32,
        )
    }
}
pub fn monitors(value: &Value) -> Result<Vec<Monitor>, String> {
    let items: Vec<Monitor> =
        serde_json::from_value(value.clone()).map_err(|_| "Invalid Wayland monitors")?;
    let mut ids = HashSet::new();
    if items.is_empty()
        || items.len() > 16
        || items.iter().any(|m| {
            !ids.insert(m.id)
                || m.id > 15
                || m.width == 0
                || m.height == 0
                || m.width > 16384
                || m.height > 16384
                || !m.scale.is_finite()
                || !(1.0..=4.0).contains(&m.scale)
                || m.x.unsigned_abs() > 131072
                || m.y.unsigned_abs() > 131072
        })
    {
        return Err("Invalid Wayland monitor geometry".into());
    }
    Ok(items)
}
pub fn frame_bytes(width: u32, height: u32) -> Result<usize, String> {
    let bytes = (width as u64)
        .checked_mul(height as u64)
        .and_then(|n| n.checked_mul(4))
        .unwrap_or(u64::MAX);
    if width == 0 || height == 0 || width > 4096 || height > 4096 || bytes > MAX_FRAME_BYTES as u64
    {
        return Err("Wayland overlay exceeds the bounded frame size".into());
    }
    Ok(bytes as usize)
}
pub fn pointer(value: &Value) -> Option<(f64, f64)> {
    let (x, y) = (value["x"].as_f64()?, value["y"].as_f64()?);
    (x.is_finite() && y.is_finite() && x.abs() <= 262144. && y.abs() <= 262144.).then_some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn negative_monitor_and_fractional_scale_keep_logical_pointer() {
        let m = monitors(&json!([{"id":0,"x":-1920,"y":0,"width":1920,"height":1080,"scale":1.5}]))
            .unwrap()
            .remove(0);
        assert!(m.contains(-1920., 0.));
        assert!(!m.contains(0., 20.));
        assert_eq!(m.pixels(), (2880, 1620));
    }
    #[test]
    fn rejects_unbounded_frames_and_invalid_monitors() {
        assert!(frame_bytes(0, 1).is_err());
        assert!(frame_bytes(u32::MAX, u32::MAX).is_err());
        assert!(frame_bytes(4096, 4096).is_err());
        assert_eq!(frame_bytes(1800, 1800).unwrap(), 12_960_000);
        assert!(monitors(&json!([])).is_err());
        assert!(monitors(&json!([{"id":0,"x":0,"y":0,"width":1,"height":1,"scale":0.}])).is_err());
        let m = json!({"id":0,"x":0,"y":0,"width":1920,"height":1080,"scale":1});
        assert!(monitors(&json!([m, m])).is_err());
        assert!(pointer(&json!({"x":1e100,"y":0})).is_none());
    }

    #[test]
    fn native_png_transport_preserves_dimensions_and_transparency() {
        let mut pixels = tiny_skia::Pixmap::new(2, 2).unwrap();
        pixels
            .data_mut()
            .copy_from_slice(&[0, 0, 0, 0, 64, 32, 16, 128, 255, 222, 0, 255, 3, 2, 1, 4]);
        let encoded = encode_frame(&pixels).unwrap();
        assert!(encoded.len() >= 33 && encoded.len() <= MAX_ENCODED_BYTES);
        assert_eq!(&encoded[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(encoded[24], 8); // bit depth
        assert_eq!(encoded[25], 6); // RGBA, as required by the Shell validator
        let decoded = tiny_skia::Pixmap::decode_png(&encoded).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 2));
        for (before, after) in pixels.data().iter().zip(decoded.data()) {
            assert!(
                before.abs_diff(*after) <= 1,
                "premultiplication must preserve the visible color"
            );
        }
        assert_eq!(&decoded.data()[..4], &[0, 0, 0, 0]);
    }
}
