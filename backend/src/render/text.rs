//! Fixed labels outlined from Inter once, before input begins. The cached paths
//! scale with the wheel and display DPI without a texture atlas or OS font stack.
use std::sync::OnceLock;
use tiny_skia::{Path, PathBuilder, Transform};
use ttf_parser::{Face, OutlineBuilder, Tag};

static LABELS: OnceLock<[Path; 4]> = OnceLock::new();

pub fn prepare() {
    LABELS.get_or_init(|| {
        let mut font = Face::parse(include_bytes!("../../resources/InterVariable.ttf"), 0)
            .expect("bundled Inter font");
        ["PING", "EMOTE", "BACK", "?"].map(|label| {
            font.set_variation(
                Tag::from_bytes(b"wght"),
                if label == "BACK" { 500. } else { 600. },
            );
            let mut builder = Outline {
                path: PathBuilder::new(),
                x: 0.,
            };
            // Explicit tracking for these fixed uppercase labels; no shaping is
            // needed. Advance widths retain the font's proportional spacing.
            let tracking = font.units_per_em() as f32 * if label == "BACK" { 0.025 } else { 0.06 };
            for ch in label.chars() {
                let glyph = font.glyph_index(ch).expect("bundled label glyph");
                font.outline_glyph(glyph, &mut builder)
                    .expect("label outline");
                builder.x += font.glyph_hor_advance(glyph).unwrap() as f32 + tracking;
            }
            let path = builder.path.finish().expect("nonempty label");
            let bounds = path.bounds();
            let scale = 1. / bounds.height();
            path.transform(Transform::from_row(
                scale,
                0.,
                0.,
                scale,
                -(bounds.left() + bounds.width() / 2.) * scale,
                -bounds.top() * scale,
            ))
            .unwrap()
        })
    });
}

/// Centered at x=0 with its visible cap height in y=0..1.
pub fn outline(label: &str) -> &'static Path {
    prepare();
    &LABELS.get().unwrap()[match label {
        "PING" => 0,
        "EMOTE" => 1,
        "BACK" => 2,
        _ => 3,
    }]
}

struct Outline {
    path: PathBuilder,
    x: f32,
}
impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(x + self.x, -y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(x + self.x, -y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.path.quad_to(x1 + self.x, -y1, x + self.x, -y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path
            .cubic_to(x1 + self.x, -y1, x2 + self.x, -y2, x + self.x, -y);
    }
    fn close(&mut self) {
        self.path.close();
    }
}
