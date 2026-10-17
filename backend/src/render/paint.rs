use super::{media::Bank, Bounds, Emote, Ping, Wheel};
use serde_json::Value;
use std::time::Instant;
use tiny_skia::*;

pub struct Painter<'a> {
    canvas: &'a mut Pixmap,
    transform: Transform,
    pub rasters: Option<&'a mut crate::cache::Cache<std::sync::Arc<Pixmap>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gradients_fade_to_transparent_edges() {
        let mut pixels = Pixmap::new(128, 128).unwrap();
        let mut painter = Painter::new(
            &mut pixels,
            Bounds {
                x: 0,
                y: 0,
                w: 128,
                h: 128,
            },
            1.,
        );
        painter.glow(64., 64., 60., [255, 30, 50], 1., Transform::identity());
        let center = pixels.pixel(64, 64).unwrap().alpha();
        let middle = pixels.pixel(94, 64).unwrap().alpha();
        let edge = pixels.pixel(122, 64).unwrap().alpha();
        assert!(
            center > 240 && (100..150).contains(&middle) && edge < 15,
            "{center}, {middle}, {edge}"
        );
    }
}
fn color(c: [u8; 3], a: f32) -> Color {
    Color::from_rgba(
        c[0] as f32 / 255.,
        c[1] as f32 / 255.,
        c[2] as f32 / 255.,
        a.clamp(0., 1.),
    )
    .unwrap()
}
fn solid(c: [u8; 3], a: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color(c, a));
    p
}
fn polar(r: f32, a: f32) -> (f32, f32) {
    (r * a.to_radians().cos(), r * a.to_radians().sin())
}
fn sector(r0: f32, r1: f32, a0: f32, a1: f32) -> Path {
    let mut p = PathBuilder::new();
    let (x, y) = polar(r0, a0);
    p.move_to(x, y);
    let steps = ((a1 - a0).abs() / 2.).ceil().max(1.) as usize;
    for i in 0..=steps {
        let (x, y) = polar(r1, a0 + (a1 - a0) * i as f32 / steps as f32);
        p.line_to(x, y);
    }
    for i in 0..=steps {
        let (x, y) = polar(r0, a1 - (a1 - a0) * i as f32 / steps as f32);
        p.line_to(x, y);
    }
    p.close();
    p.finish().unwrap()
}
pub(super) fn effect_sprites(c: [u8; 3]) -> super::media::Effects {
    let mut ring = Pixmap::new(320, 320).unwrap();
    let mut painter = Painter::new(
        &mut ring,
        Bounds {
            x: 0,
            y: 0,
            w: 320,
            h: 320,
        },
        2.,
    );
    let t = painter.transform.pre_translate(80., 80.);
    for (width, mul) in [(16., 0.025), (10., 0.06), (6., 0.15), (3., 1.)] {
        painter.ring(0., 0., 60., c, mul, width, t);
    }
    let mut glow = Pixmap::new(192, 192).unwrap();
    let mut painter = Painter::new(
        &mut glow,
        Bounds {
            x: 0,
            y: 0,
            w: 192,
            h: 192,
        },
        1.,
    );
    painter.glow(96., 96., 96., c, 1., Transform::identity());
    super::media::Effects { ring, glow }
}
impl<'a> Painter<'a> {
    pub fn new(canvas: &'a mut Pixmap, b: Bounds, scale: f64) -> Self {
        Self {
            canvas,
            rasters: None,
            transform: Transform::from_row(
                scale as f32,
                0.,
                0.,
                scale as f32,
                -b.x as f32,
                -b.y as f32,
            ),
        }
    }
    fn fill(&mut self, path: &Path, paint: &Paint<'_>, t: Transform) {
        self.canvas
            .fill_path(path, paint, FillRule::Winding, t, None);
    }
    fn circle(&mut self, x: f32, y: f32, r: f32, c: [u8; 3], a: f32, t: Transform) {
        if r > 0. {
            self.fill(&PathBuilder::from_circle(x, y, r).unwrap(), &solid(c, a), t);
        }
    }
    fn stroke(&mut self, path: &Path, c: [u8; 3], a: f32, width: f32, t: Transform) {
        self.canvas.stroke_path(
            path,
            &solid(c, a),
            &Stroke {
                width,
                line_cap: LineCap::Round,
                ..Default::default()
            },
            t,
            None,
        );
    }
    fn ring(&mut self, x: f32, y: f32, r: f32, c: [u8; 3], a: f32, width: f32, t: Transform) {
        if r > 0. {
            self.stroke(&PathBuilder::from_circle(x, y, r).unwrap(), c, a, width, t);
        }
    }
    fn glow(&mut self, x: f32, y: f32, r: f32, c: [u8; 3], a: f32, t: Transform) {
        if r <= 0. {
            return;
        }
        let shader = RadialGradient::new(
            Point::from_xy(x, y),
            0.,
            Point::from_xy(x, y),
            r,
            vec![
                GradientStop::new(0., color(c, a)),
                GradientStop::new(1., color(c, 0.)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        let p = Paint {
            shader,
            ..Default::default()
        };
        self.fill(&PathBuilder::from_circle(x, y, r).unwrap(), &p, t);
    }
    fn image(&mut self, image: &Pixmap, x: f32, y: f32, w: f32, h: f32, alpha: f32, t: Transform) {
        if w <= 0. || h <= 0. || alpha <= 0. {
            return;
        }
        super::blit::draw(
            self.canvas,
            image,
            t.sx * x + t.tx,
            t.sy * y + t.ty,
            t.sx * w,
            t.sy * h,
            alpha,
        );
    }
    fn sprite(&mut self, image: &Pixmap, x: f32, y: f32, w: f32, h: f32, alpha: f32, t: Transform) {
        let width = (t.sx * w).round().max(1.) as u32;
        let height = (t.sy * h).round().max(1.) as u32;
        if alpha <= 0. || width as usize * height as usize > 1024 * 1024 {
            return self.image(image, x, y, w, h, alpha, t);
        }
        let key = format!("{:p}/{width}/{height}", image.data().as_ptr());
        let now = Instant::now();
        let cached = self
            .rasters
            .as_mut()
            .and_then(|cache| cache.get(&key, now).cloned());
        let scaled = match cached {
            Some(p) => p,
            None => {
                let Some(mut p) = Pixmap::new(width, height) else {
                    return;
                };
                super::blit::draw(&mut p, image, 0., 0., width as f32, height as f32, 1.);
                let p = std::sync::Arc::new(p);
                if let Some(cache) = self.rasters.as_mut() {
                    cache.insert(key, p.clone(), p.data().len(), now);
                }
                p
            }
        };
        super::blit::copy(
            self.canvas,
            &scaled,
            (t.sx * x + t.tx).round() as i32,
            (t.sy * y + t.ty).round() as i32,
            alpha,
        );
    }
    pub(super) fn wheel(&mut self, w: &Wheel, s: &Value, bank: &Bank, now: Instant) {
        let age = now.duration_since(w.start).as_secs_f32();
        let open = 1. - (1. - (age / 0.12).min(1.)).powi(3);
        let scale = s["wheelSizePx"].as_f64().unwrap_or(600.) as f32 / 600. * (0.85 + 0.15 * open);
        let t = self
            .transform
            .pre_translate(w.x as f32, w.y as f32)
            .pre_scale(scale, scale);
        for i in (0..8).step_by(2) {
            let mid = -90. + i as f32 * 45.;
            let shader = RadialGradient::new(
                Point::zero(),
                0.,
                Point::zero(),
                288.,
                vec![
                    GradientStop::new(188. / 288., color([92, 96, 102], 0.55)),
                    GradientStop::new(1., color([92, 96, 102], 0.)),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            )
            .unwrap();
            self.fill(
                &sector(188., 288., mid - 22.5, mid + 22.5),
                &Paint {
                    shader,
                    ..Default::default()
                },
                t,
            );
        }
        let shader = RadialGradient::new(
            Point::zero(),
            0.,
            Point::zero(),
            188.,
            vec![
                GradientStop::new(86. / 188., color([42, 45, 49], 0.92)),
                GradientStop::new(1., color([74, 78, 84], 0.92)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        self.fill(
            &sector(86., 188., 0., 360.),
            &Paint {
                shader,
                ..Default::default()
            },
            t,
        );
        let ids = s[if w.emote { "emoteWheel" } else { "wheel" }].as_array();
        for i in 0..8 {
            let mid = -90. + i as f32 * 45.;
            if w.hot == Some(i) {
                let shader = RadialGradient::new(
                    Point::zero(),
                    0.,
                    Point::zero(),
                    288.,
                    vec![
                        GradientStop::new(86. / 288., color([138, 143, 150], 0.55)),
                        GradientStop::new(188. / 288., color([154, 160, 168], 0.55)),
                        GradientStop::new(1., color([154, 160, 168], 0.)),
                    ],
                    SpreadMode::Pad,
                    Transform::identity(),
                )
                .unwrap();
                self.fill(
                    &sector(
                        86.,
                        if i % 2 == 0 { 288. } else { 188. },
                        mid - 22.5,
                        mid + 22.5,
                    ),
                    &Paint {
                        shader,
                        ..Default::default()
                    },
                    t,
                );
            }
            let (x0, y0) = polar(86., mid - 22.5);
            let (x1, y1) = polar(188., mid - 22.5);
            let mut path = PathBuilder::new();
            path.move_to(x0, y0);
            path.line_to(x1, y1);
            self.stroke(&path.finish().unwrap(), [160, 138, 92], 0.45, 1.2, t);
            let id = ids
                .and_then(|v| v.get(i))
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = if w.emote {
                id
            } else {
                super::ping(id).map(|p| p.icon).unwrap_or("generic_ping")
            };
            let variant = if w.emote {
                format!("{key}#round")
            } else if w.hot != Some(i) {
                format!("{key}#muted")
            } else {
                key.into()
            };
            if let Some(image) = bank.images.get(&variant).or_else(|| bank.images.get(key)) {
                let (x, y) = polar(137., mid);
                let size = if w.hot == Some(i) { 56.64 } else { 48. };
                self.image(image, x - size / 2., y - size / 2., size, size, 1., t);
            }
        }
        self.ring(0., 0., 90., [26, 26, 26], 0.6, 6., t);
        let shader = RadialGradient::new(
            Point::from_xy(0., -10.),
            0.,
            Point::from_xy(0., -10.),
            86.,
            vec![
                GradientStop::new(0., color([44, 93, 96], 1.)),
                GradientStop::new(1., color([17, 41, 44], 1.)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        self.fill(
            &PathBuilder::from_circle(0., 0., 86.).unwrap(),
            &Paint {
                shader,
                ..Default::default()
            },
            t,
        );
        self.ring(0., 0., 86., [200, 170, 110], 1., 3., t);
        self.ring(0., 0., 79., [138, 116, 72], 0.6, 1., t);
        if let Some(image) = bank.images.get(if w.emote {
            "facepalm#round"
        } else {
            "generic_ping"
        }) {
            self.image(image, -20., -58., 40., 40., 1., t);
        }
        self.label(
            if w.emote { "EMOTE" } else { "PING" },
            0.,
            -8.,
            16.,
            [31, 169, 224],
            t,
        );
        let back_width = super::text::outline("BACK").bounds().width() * 10.;
        let mouse_x = -(10. + 6. + back_width) / 2.;
        let hint_color = [149, 169, 171];
        self.label(
            "BACK",
            mouse_x + 16. + back_width / 2.,
            42.,
            10.,
            hint_color,
            t,
        );
        let mouse =
            PathBuilder::from_oval(Rect::from_xywh(mouse_x, 39., 10., 16.).unwrap()).unwrap();
        self.stroke(&mouse, hint_color, 0.85, 1.2, t);
        let mut scroll = PathBuilder::new();
        scroll.move_to(mouse_x + 5., 42.);
        scroll.line_to(mouse_x + 5., 45.);
        self.stroke(&scroll.finish().unwrap(), hint_color, 0.85, 1.2, t);
    }
    pub(super) fn ping(&mut self, p: &Ping, bank: &Bank, now: Instant) {
        let age = now.duration_since(p.start).as_secs_f32();
        let duration = p.duration as f32;
        let size = p.size as f32;
        let t = self.transform.pre_translate(p.x as f32, p.y as f32);
        let c = p.def.color;
        let ground = if age < duration * 0.1 {
            age / (duration * 0.1) * 0.75
        } else if age < duration * 0.85 {
            0.75 - (age / duration - 0.1) * 0.267
        } else {
            (1. - age / duration).max(0.) / 0.15 * 0.55
        };
        let fx = bank.effects.get(&c).unwrap();
        self.sprite(
            &fx.glow,
            -size * 0.42,
            size * 0.2,
            size * 0.84,
            size * 0.24,
            ground,
            t,
        );
        for delay in [0., 0.22, 1.4] {
            let progress = (age - delay) / 0.65;
            if (0. ..1.).contains(&progress) {
                let eased = 1. - (1. - progress).powi(3);
                let radius = size * 0.545 * (0.25 + eased * 1.1);
                let alpha = (1. - eased) * 0.95;
                let extent = radius * 80. / 60.;
                self.image(
                    &fx.ring,
                    -extent,
                    -extent,
                    extent * 2.,
                    extent * 2.,
                    alpha,
                    t,
                );
            }
        }
        let life = if p.keep {
            1.
        } else {
            ((duration - age) / (duration * 0.13)).clamp(0., 1.)
        };
        let bob = if age > 0.3 {
            -2.5 * (1. - ((age - 0.3) / 1.6 * std::f32::consts::TAU).cos())
        } else {
            0.
        };
        let center_y = -size * 0.23 + bob;
        if p.def.art.len() == 2 {
            let slide = (age / 0.22).clamp(0., 1.).powf(1.6);
            let shift = size * 0.64 * (1. - slide);
            let shake = if (0.22..0.47).contains(&age) {
                ((age - 0.22) * 100.).sin() * 3.
            } else {
                0.
            };
            for (i, key) in p.def.art.iter().enumerate() {
                if let Some(image) = bank.images.get(*key) {
                    self.sprite(
                        image,
                        -size / 2. + if i == 0 { shift } else { -shift } + shake,
                        center_y - size / 2. - shift,
                        size,
                        size,
                        life * slide,
                        t,
                    );
                }
            }
            let flash = (age - 0.2) / 0.35;
            if (0. ..1.).contains(&flash) {
                let r = size * 0.25 * (0.3 + 1.9 * flash);
                self.image(
                    &bank.effects.get(&[255, 255, 255]).unwrap().glow,
                    -r,
                    center_y - r,
                    r * 2.,
                    r * 2.,
                    (1. - flash) * 0.85,
                    t,
                );
            }
        } else {
            let progress = (age / 0.28).clamp(0., 1.);
            let pop = if progress < 0.6 {
                0.4 + 0.75 * (1. - (1. - progress / 0.6).powi(2))
            } else {
                1. + 0.15 * (1. - (progress - 0.6) / 0.4)
            };
            if let Some(image) = bank.images.get(p.def.art[0]) {
                self.sprite(
                    &fx.glow,
                    -size * 0.6,
                    center_y - size * 0.6,
                    size * 1.2,
                    size * 1.2,
                    life * 0.18,
                    t,
                );
                let draw = if progress >= 1. {
                    Self::sprite
                } else {
                    Self::image
                };
                draw(
                    self,
                    image,
                    -size * pop / 2.,
                    center_y - size * pop / 2.,
                    size * pop,
                    size * pop,
                    life * (progress / 0.6).min(1.),
                    t,
                );
            }
        }
    }
    pub(super) fn emote(&mut self, e: &Emote, now: Instant) {
        let age = now.duration_since(e.start).as_secs_f32();
        let image = e
            .animation
            .as_ref()
            .and_then(|a| a.current.as_ref())
            .or(e.still.as_ref());
        let mut size = e.size as f32;
        let mut alpha = 1.;
        let mut y = e.y as f32;
        if e.custom {
            let enter = (age / 0.25).min(1.);
            let pop = if enter < 0.6 {
                1.15 * (1. - (1. - enter / 0.6).powi(2))
            } else {
                1. + 0.15 * (1. - enter) / 0.4
            };
            let exit = ((age - 2.65) / 0.35).clamp(0., 1.);
            size *= pop * (1. - 0.15 * exit);
            alpha = 1. - exit * exit;
            y -= 4. * (age / 1.4 * std::f32::consts::TAU).sin();
        } else {
            size *= 1.25;
        }
        if let Some(image) = image {
            let aspect = image.width() as f32 / image.height() as f32;
            let (width, height) = if aspect > 1. {
                (size, size / aspect)
            } else {
                (size * aspect, size)
            };
            self.image(
                image,
                e.x as f32 - width / 2.,
                y - height / 2.,
                width,
                height,
                alpha,
                self.transform,
            );
        } else {
            self.circle(
                e.x as f32,
                y,
                size * 0.4,
                [244, 239, 226],
                alpha,
                self.transform,
            );
            self.label(
                "?",
                e.x as f32,
                y - size * 0.16,
                size * 0.3,
                [31, 42, 48],
                self.transform,
            );
        }
    }
    fn label(&mut self, text: &str, x: f32, y: f32, height: f32, c: [u8; 3], t: Transform) {
        self.fill(
            super::text::outline(text),
            &solid(c, 1.),
            t.pre_translate(x, y).pre_scale(height, height),
        );
    }
}
