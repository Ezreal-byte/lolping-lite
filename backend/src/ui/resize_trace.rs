//! Opt-in resize diagnostics. Normal sessions do no sampling or filesystem I/O.
use eframe::egui::Vec2;
use serde_json::json;
use std::{path::PathBuf, time::Instant};

pub(super) struct Trace {
    path: PathBuf,
    start: Instant,
    frames: Vec<(f64, f32, f32, Option<f32>, f64)>,
}

impl Trace {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("LOLPING_UI_TEST_RESIZE_TRACE")?;
        Some(Self {
            path: path.into(),
            start: Instant::now(),
            frames: Vec::new(),
        })
    }

    pub fn record(&mut self, size: Vec2, previous_cpu: Option<f32>, ui_cpu_ms: f64) {
        if self.frames.len() < 20_000 {
            self.frames.push((
                self.start.elapsed().as_secs_f64() * 1000.0,
                size.x,
                size.y,
                previous_cpu.map(|seconds| seconds * 1000.0),
                ui_cpu_ms,
            ));
        }
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        // Write after the event loop closes; logging must not itself stall resize.
        let report = json!({
            "columns": ["elapsedMs", "width", "height", "previousEframeMs", "currentUiMs"],
            "note": "UI timestamps include event scheduling and swap waits. previousEframeMs includes rendering/GPU calls but excludes vsync. currentUiMs measures only App::ui, before tessellation/OpenGL. Wall-clock durations, not CPU utilization or GPU presentation timestamps.",
            "frames": self.frames,
        });
        if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
            let _ = std::fs::write(&self.path, bytes);
        }
    }
}
