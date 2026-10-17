//! Native overlay scene. Coordinates and selection boundaries match the original
//! wheel; drawing and audio never alter the C++ helper's gesture state.
mod blit;
pub mod media;
mod paint;
pub mod surface;
mod text;
use media::{Animation, Bank, DecoderPool};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tiny_skia::Pixmap;

pub struct PingDef {
    pub id: &'static str,
    pub icon: &'static str,
    pub art: &'static [&'static str],
    pub sound: &'static str,
    pub color: [u8; 3],
}
pub const PINGS: &[PingDef] = &[
    PingDef {
        id: "danger",
        icon: "pingwheel_retreatrender",
        art: &["pingwheel_retreatrender"],
        sound: "SRP_12",
        color: [255, 59, 85],
    },
    PingDef {
        id: "push",
        icon: "push",
        art: &["pingwheel_pushrender"],
        sound: "SRP_9",
        color: [53, 210, 122],
    },
    PingDef {
        id: "omw",
        icon: "omw",
        art: &["omw"],
        sound: "OnMyWay",
        color: [46, 139, 255],
    },
    PingDef {
        id: "allin",
        icon: "all_in",
        art: &["pingwheel_allin_render_01", "pingwheel_allin_render_02"],
        sound: "SRP_4",
        color: [242, 179, 32],
    },
    PingDef {
        id: "assist",
        icon: "assist_me",
        art: &["assist_me"],
        sound: "ComeHere",
        color: [86, 212, 60],
    },
    PingDef {
        id: "needvision",
        icon: "need_vision",
        art: &["need_vision"],
        sound: "SRP_7",
        color: [86, 212, 60],
    },
    PingDef {
        id: "missing",
        icon: "enemy_missing",
        art: &["enemy_missing"],
        sound: "MIA",
        color: [242, 197, 24],
    },
    PingDef {
        id: "enemyvision",
        icon: "pingwheel_enemyvisionrender",
        art: &["pingwheel_enemyvisionrender"],
        sound: "AreaIsWarded",
        color: [255, 59, 85],
    },
    PingDef {
        id: "generic",
        icon: "generic_ping",
        art: &["pingwheel_basicpingrender"],
        sound: "Base",
        color: [46, 139, 255],
    },
    PingDef {
        id: "bait",
        icon: "pingwheel_baitrender",
        art: &["pingwheel_baitrender"],
        sound: "SRP_11",
        color: [239, 188, 18],
    },
    PingDef {
        id: "visioncleared",
        icon: "vision_cleared",
        art: &["pingwheel_visionclearedrender"],
        sound: "SRP_6",
        color: [43, 128, 255],
    },
    PingDef {
        id: "missingflip",
        icon: "enemy_missing_flipped",
        art: &["enemy_missing_flipped"],
        sound: "MIA",
        color: [242, 197, 24],
    },
];
pub fn ping(id: &str) -> Option<&'static PingDef> {
    PINGS.iter().find(|p| p.id == id)
}
pub fn pick_slice(dx: f64, dy: f64, size: f64) -> Option<usize> {
    if dx.hypot(dy) < 86. * size / 600. {
        return None;
    }
    Some(((dy.atan2(dx).to_degrees() + 112.5).rem_euclid(360.) / 45.).floor() as usize % 8)
}
pub enum Output {
    Sound(String),
    Selected,
}
struct Wheel {
    x: f64,
    y: f64,
    emote: bool,
    hot: Option<usize>,
    start: Instant,
}
struct Ping {
    def: &'static PingDef,
    x: f64,
    y: f64,
    size: f64,
    duration: f64,
    keep: bool,
    start: Instant,
}
struct Emote {
    x: f64,
    y: f64,
    size: f64,
    custom: bool,
    preview: bool,
    start: Instant,
    still: Option<Arc<Pixmap>>,
    animation: Option<Animation>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}
pub struct Scene {
    root: PathBuf,
    data: PathBuf,
    settings: Value,
    bank: Arc<Bank>,
    rasters: crate::cache::Cache<Arc<Pixmap>>,
    wheel: Option<Wheel>,
    pings: Vec<Ping>,
    emotes: Vec<Emote>,
    pub dirty: bool,
}
impl Scene {
    pub fn new(root: PathBuf, data: PathBuf, settings: Value, bank: Arc<Bank>) -> Self {
        text::prepare();
        Self {
            root,
            data,
            settings,
            bank,
            rasters: crate::cache::Cache::new(8 * 1024 * 1024, Duration::from_secs(120)),
            wheel: None,
            pings: vec![],
            emotes: vec![],
            dirty: true,
        }
    }
    pub fn settings(&mut self, value: Value, bank: Arc<Bank>) {
        if !Arc::ptr_eq(&self.bank, &bank) {
            self.rasters.clear();
        }
        self.settings = value;
        self.bank = bank;
        if self.settings["keepLastPing"] != true {
            self.pings.retain(|p| !p.keep);
        }
        self.dirty = true;
    }
    fn number(&self, key: &str, default: f64) -> f64 {
        self.settings[key].as_f64().unwrap_or(default)
    }
    pub fn event(
        &mut self,
        name: &str,
        p: &Value,
        pool: &DecoderPool,
        now: Instant,
    ) -> Vec<Output> {
        let mut out = vec![];
        let (x, y) = (p["x"].as_f64().unwrap_or(0.), p["y"].as_f64().unwrap_or(0.));
        let emote = p["wheel"] == "emote";
        let size = self.number("wheelSizePx", 600.);
        match name {
            "wheelOpen" => {
                self.wheel = Some(Wheel {
                    x,
                    y,
                    emote,
                    hot: None,
                    start: now,
                })
            }
            "wheelMove" => {
                if let Some(w) = &mut self.wheel {
                    let hot = pick_slice(x - w.x, y - w.y, size);
                    if hot != w.hot {
                        w.hot = hot;
                        if hot.is_some() {
                            out.push(Output::Sound("button".into()));
                        }
                    } else {
                        return out;
                    }
                }
            }
            "wheelRelease" => {
                if let Some(w) = self.wheel.take() {
                    if let Some(index) = pick_slice(x - w.x, y - w.y, size) {
                        if let Some(id) = self.settings
                            [if w.emote { "emoteWheel" } else { "wheel" }][index]
                            .as_str()
                            .map(str::to_owned)
                        {
                            self.place(&id, w.x, w.y, w.emote, false, pool, now, &mut out);
                        }
                    }
                }
            }
            "click" | "preview" => {
                let id = if name == "preview" {
                    p["id"].as_str()
                } else {
                    self.settings[if emote { "clickEmoteId" } else { "clickPingId" }].as_str()
                }
                .unwrap_or("")
                .to_owned();
                self.place(&id, x, y, emote, name == "preview", pool, now, &mut out);
            }
            "cancel" => self.wheel = None,
            "library" => self.emotes.clear(),
            "clear" => {
                self.wheel = None;
                self.pings.clear();
                self.emotes.clear();
            }
            _ => return out,
        }
        self.dirty = true;
        out
    }
    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        id: &str,
        x: f64,
        y: f64,
        emote: bool,
        preview: bool,
        pool: &DecoderPool,
        now: Instant,
        out: &mut Vec<Output>,
    ) {
        if emote {
            let custom = crate::assets::custom_id(id).is_some();
            let item = self.settings["customEmotes"]
                .as_array()
                .and_then(|v| v.iter().find(|v| v["id"] == id));
            if custom && item.is_none() {
                return;
            }
            let animated = !custom || item.is_some_and(|v| v["animated"] == true);
            let size = self.number("emoteSizePx", 150.);
            let still = self.bank.images.get(id).cloned().or_else(|| {
                media::emote_path(&self.root, &self.data, id, false)
                    .and_then(|p| media::still(&p, 256))
            });
            // Old receiver is dropped before queuing a replacement, releasing a blocked decoder immediately.
            self.emotes.retain(|e| e.preview != preview);
            let animation = if animated {
                media::emote_path(&self.root, &self.data, id, true)
                    .map(|p| pool.play(p, 768, custom))
            } else {
                None
            };
            self.emotes.push(Emote {
                x,
                y,
                size,
                custom,
                preview,
                start: now,
                still,
                animation,
            });
            if !custom || item.is_some_and(|v| v["sound"] == true) {
                out.push(Output::Sound(format!("emote:{id}")));
            }
        } else if let Some(def) = ping(id) {
            self.pings.retain(|p| !p.keep);
            if self.pings.len() >= 60 {
                self.pings.remove(0);
            }
            self.pings.push(Ping {
                def,
                x,
                y,
                size: self.number("pingSizePx", 110.),
                duration: self.number("pingDurationS", 3.2),
                keep: !preview && self.settings["keepLastPing"] == true,
                start: now,
            });
            out.push(Output::Sound(def.sound.into()));
        } else {
            return;
        }
        if !preview {
            out.push(Output::Selected);
        }
    }
    #[cfg(test)]
    pub fn active(&self) -> bool {
        self.wheel.is_some() || !self.pings.is_empty() || !self.emotes.is_empty()
    }
    pub fn animating(&self, now: Instant) -> bool {
        !self.pings.is_empty()
            || !self.emotes.is_empty()
            || self
                .wheel
                .as_ref()
                .is_some_and(|w| now.duration_since(w.start) < Duration::from_millis(120))
    }
    pub fn advance(&mut self, now: Instant) {
        let before = (self.pings.len(), self.emotes.len());
        self.pings
            .retain(|p| p.keep || now.duration_since(p.start).as_secs_f64() < p.duration + 0.15);
        self.emotes
            .retain(|e| now.duration_since(e.start) < Duration::from_millis(3150));
        for e in &mut self.emotes {
            if let Some(a) = &mut e.animation {
                a.advance(now.duration_since(e.start));
            }
        }
        self.dirty |= before != (self.pings.len(), self.emotes.len());
    }
    pub fn bounds(&self, scale: f64, width: u32, height: u32) -> Option<Bounds> {
        let mut rects = vec![];
        if let Some(w) = &self.wheel {
            let r = self.number("wheelSizePx", 600.) / 2. + 4.;
            rects.push((w.x - r, w.y - r, w.x + r, w.y + r));
        }
        for p in &self.pings {
            let r = p.size * 1.5 + 24.;
            rects.push((p.x - r, p.y - r, p.x + r, p.y + r));
        }
        for e in &self.emotes {
            let r = e.size * 0.75 + 16.;
            rects.push((e.x - r, e.y - r, e.x + r, e.y + r));
        }
        clipped_bounds(&rects, scale, width, height)
    }
    pub fn draw(&mut self, pixmap: &mut Pixmap, bounds: Bounds, scale: f64, now: Instant) {
        pixmap.fill(tiny_skia::Color::TRANSPARENT);
        let mut p = paint::Painter::new(pixmap, bounds, scale);
        p.rasters = Some(&mut self.rasters);
        for ping in &self.pings {
            p.ping(ping, &self.bank, now);
        }
        if let Some(w) = &self.wheel {
            p.wheel(w, &self.settings, &self.bank, now);
        }
        for e in &self.emotes {
            p.emote(e, now);
        }
        self.dirty = false;
    }
    pub fn release_buffers(&mut self) {
        self.rasters.clear();
    }
}
fn clipped_bounds(
    rects: &[(f64, f64, f64, f64)],
    scale: f64,
    width: u32,
    height: u32,
) -> Option<Bounds> {
    if rects.is_empty() {
        return None;
    }
    let x = rects
        .iter()
        .map(|r| r.0 * scale)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .clamp(0., width as f64) as i32;
    let y = rects
        .iter()
        .map(|r| r.1 * scale)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .clamp(0., height as f64) as i32;
    let right = rects
        .iter()
        .map(|r| r.2 * scale)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .clamp(0., width as f64) as i32;
    let bottom = rects
        .iter()
        .map(|r| r.3 * scale)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .clamp(0., height as f64) as i32;
    (right > x && bottom > y).then_some(Bounds {
        x,
        y,
        w: (right - x).max(0) as u32,
        h: (bottom - y).max(0) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_bank() -> Arc<Bank> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources");
        let mut bank = Bank::default();
        for def in PINGS {
            for key in std::iter::once(def.icon).chain(def.art.iter().copied()) {
                bank.images.insert(
                    key.into(),
                    media::still(&root.join(format!("textures/{key}.png")), 512)
                        .expect("ping texture"),
                );
            }
        }
        for id in crate::settings::core_emotes() {
            bank.images.insert(
                id.clone(),
                media::still(&root.join(format!("emotes/{id}.png")), 256).expect("emote thumbnail"),
            );
        }
        bank.wheel_variants();
        bank.into()
    }
    #[test]
    fn native_frames_draw_transparency_at_all_sizes_and_create_review_fixtures() {
        let bank = fixture_bank();
        assert!(bank.bytes() < 16 * 1024 * 1024);
        let pool = DecoderPool::new();
        let now = Instant::now();
        let mut scene = Scene::new(
            PathBuf::new(),
            PathBuf::new(),
            crate::settings::defaults(),
            bank.clone(),
        );
        for (name, event, value, age) in [
            (
                "wheel",
                "wheelOpen",
                serde_json::json!({"x":400,"y":400}),
                0.2,
            ),
            (
                "wheel-hot",
                "wheelMove",
                serde_json::json!({"x":400,"y":200}),
                0.2,
            ),
            (
                "danger",
                "preview",
                serde_json::json!({"id":"danger","x":400,"y":400}),
                0.4,
            ),
            (
                "allin",
                "preview",
                serde_json::json!({"id":"allin","x":400,"y":400}),
                0.35,
            ),
            (
                "emote-wheel",
                "wheelOpen",
                serde_json::json!({"wheel":"emote","x":400,"y":400}),
                0.2,
            ),
        ] {
            if !name.starts_with("wheel") {
                scene.event("clear", &Value::Null, &pool, now);
            }
            scene.event(event, &value, &pool, now);
            let t = now + Duration::from_secs_f64(age);
            scene.advance(t);
            let bounds = scene.bounds(1., 800, 800).unwrap();
            let mut pixels = Pixmap::new(bounds.w, bounds.h).unwrap();
            scene.draw(&mut pixels, bounds, 1., t);
            assert_eq!(
                pixels.pixel(0, 0).unwrap().alpha(),
                0,
                "{name} corners must be transparent"
            );
            assert!(
                pixels.pixels().iter().filter(|p| p.alpha() > 0).count() > 200,
                "{name} must contain art"
            );
            if let Some(dir) = std::env::var_os("LOLPING_RENDER_FIXTURES") {
                let dir = PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                pixels.save_png(dir.join(format!("{name}.png"))).unwrap();
            }
        }
        for size in [300., 600., 900.] {
            let mut settings = crate::settings::defaults();
            settings["wheelSizePx"] = serde_json::json!(size);
            scene.settings(settings, bank.clone());
            scene.event(
                "wheelOpen",
                &serde_json::json!({"x":400,"y":400}),
                &pool,
                now,
            );
            let outputs = scene.event(
                "wheelRelease",
                &serde_json::json!({"x":400,"y":400.-size/3.}),
                &pool,
                now,
            );
            assert!(outputs
                .iter()
                .any(|o| matches!(o,Output::Sound(s) if s=="SRP_12")));
        }
    }
    #[test]
    fn ping_floods_expiry_and_settings_do_not_cancel_a_gesture() {
        let pool = DecoderPool::new();
        let now = Instant::now();
        let bank = Arc::new(Bank::default());
        let mut scene = Scene::new(
            PathBuf::new(),
            PathBuf::new(),
            crate::settings::defaults(),
            bank.clone(),
        );
        for _ in 0..100 {
            scene.event("click", &serde_json::json!({"x":100,"y":100}), &pool, now);
        }
        assert_eq!(scene.pings.len(), 60);
        scene.advance(now + Duration::from_secs(10));
        assert!(!scene.active());
        scene.event(
            "wheelOpen",
            &serde_json::json!({"x":100,"y":100}),
            &pool,
            now,
        );
        let mut settings = crate::settings::defaults();
        settings["volume"] = serde_json::json!(20);
        scene.settings(settings, bank);
        let out = scene.event(
            "wheelRelease",
            &serde_json::json!({"x":100,"y":0}),
            &pool,
            now,
        );
        assert!(out.iter().any(|v| matches!(v, Output::Selected)));
        let out = scene.event(
            "preview",
            &serde_json::json!({"id":"danger","x":100,"y":100}),
            &pool,
            now,
        );
        assert!(!out.iter().any(|v| matches!(v, Output::Selected)));
    }
    #[test]
    fn selection_keeps_original_dead_zone_and_clockwise_boundaries() {
        assert_eq!(pick_slice(0., -85.99, 600.), None);
        assert_eq!(pick_slice(0., -86., 600.), Some(0));
        for i in 0..8 {
            let a = (-90. + i as f64 * 45.).to_radians();
            assert_eq!(pick_slice(a.cos() * 200., a.sin() * 200., 600.), Some(i));
        }
        assert_eq!(pick_slice(0., -43., 300.), Some(0));
    }
    #[test]
    fn bounds_clip_negative_positions_and_scale_without_losing_edge_pixels() {
        assert_eq!(
            clipped_bounds(&[(-10., -20., 100.1, 50.1)], 1.5, 300, 200),
            Some(Bounds {
                x: 0,
                y: 0,
                w: 151,
                h: 76
            })
        );
        assert_eq!(
            clipped_bounds(&[(400., 0., 500., 100.)], 1., 300, 200),
            None
        );
    }
    #[test]
    fn cancel_release_and_kept_ping_lifecycle() {
        let mut settings = crate::settings::defaults();
        settings["keepLastPing"] = Value::Bool(true);
        let pool = DecoderPool::new();
        let now = Instant::now();
        let mut scene = Scene::new(
            PathBuf::new(),
            PathBuf::new(),
            settings,
            Arc::new(Bank::default()),
        );
        scene.event(
            "wheelOpen",
            &serde_json::json!({"x":200,"y":200}),
            &pool,
            now,
        );
        assert!(scene
            .event(
                "wheelRelease",
                &serde_json::json!({"x":200,"y":200}),
                &pool,
                now
            )
            .is_empty());
        assert!(!scene.active());
        scene.event("click", &serde_json::json!({"x":200,"y":200}), &pool, now);
        scene.advance(now + Duration::from_secs(10));
        assert!(scene.active());
        scene.event("clear", &Value::Null, &pool, now);
        assert!(!scene.active());
        scene.event(
            "wheelRelease",
            &serde_json::json!({"x":200,"y":0}),
            &pool,
            now,
        );
        assert!(!scene.active());
    }

    #[test]
    fn edge_cancel_does_not_leak_into_next_wheel_selection() {
        let pool = DecoderPool::new();
        let now = Instant::now();
        let mut scene = Scene::new(
            PathBuf::new(),
            PathBuf::new(),
            crate::settings::defaults(),
            Arc::new(Bank::default()),
        );
        for (x, y) in [(0., 0.), (0., 450.), (1439., 0.), (1439., 899.)] {
            scene.event("clear", &Value::Null, &pool, now);
            scene.event("wheelOpen", &serde_json::json!({"x":x,"y":y}), &pool, now);
            scene.event(
                "wheelMove",
                &serde_json::json!({"x":x+120.,"y":y}),
                &pool,
                now,
            );
            // Returning to the original press cancels, including at a screen edge.
            let out = scene.event(
                "wheelRelease",
                &serde_json::json!({"x":x,"y":y}),
                &pool,
                now,
            );
            assert!(out.is_empty());
            assert!(!scene.active());
            scene.event(
                "wheelOpen",
                &serde_json::json!({"x":720.,"y":450.}),
                &pool,
                now,
            );
            for scale in [1., 1.25, 1.5, 2.] {
                let b = scene
                    .bounds(scale, (1440. * scale) as u32, (900. * scale) as u32)
                    .unwrap();
                // Fractional DPI rounds both edges outward, adding at most two pixels.
                let diameter = (608. * scale) as u32;
                assert!((diameter..=diameter + 2).contains(&b.w));
                assert!((diameter..=diameter + 2).contains(&b.h));
            }
            let out = scene.event(
                "wheelRelease",
                &serde_json::json!({"x":570.,"y":450.}),
                &pool,
                now,
            );
            assert!(out.iter().any(|o| matches!(o, Output::Selected)));
            assert!(scene.wheel.is_none());
            assert_eq!(scene.pings.len(), 1);
            assert_eq!(scene.pings[0].def.id, "missing");
            assert_eq!((scene.pings[0].x, scene.pings[0].y), (720., 450.));
        }
    }
}
