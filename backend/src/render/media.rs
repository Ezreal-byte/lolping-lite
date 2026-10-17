//! Bounded image storage. Animation decoders run off the input/event thread and
//! use a two-frame channel; a three-second emote never becomes a full frame cache.
use crate::assets;
use image::{AnimationDecoder, ImageDecoder, ImageFormat, ImageReader};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::{BufReader, Cursor},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};
use tiny_skia::{IntSize, Pixmap};

pub fn pixmap(image: image::RgbaImage, max: u32) -> Option<Pixmap> {
    let (w, h) = image.dimensions();
    let image = if w.max(h) > max {
        let scale = max as f64 / w.max(h) as f64;
        image::imageops::resize(
            &image,
            (w as f64 * scale).round().max(1.) as u32,
            (h as f64 * scale).round().max(1.) as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    let (w, h) = image.dimensions();
    let mut bytes = image.into_raw();
    for p in bytes.chunks_exact_mut(4) {
        for c in 0..3 {
            p[c] = ((p[c] as u16 * p[3] as u16 + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(bytes, IntSize::from_wh(w, h)?)
}

pub fn still(path: &Path, max: u32) -> Option<Arc<Pixmap>> {
    let mut reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    pixmap(reader.decode().ok()?.into_rgba8(), max).map(Arc::new)
}

pub fn emote_path(root: &Path, data: &Path, id: &str, animated: bool) -> Option<PathBuf> {
    let name = if let Some(hash) = assets::custom_id(id) {
        format!("custom/{hash}.{}", if animated { "anim" } else { "png" })
    } else {
        // Only catalog/settings-validated ids reach here; resolve_asset still confines paths.
        format!("emotes/{id}.{}", if animated { "webp" } else { "png" })
    };
    assets::resolve_asset(root, data, &name)
}

pub struct Effects {
    pub ring: Pixmap,
    pub glow: Pixmap,
}
pub struct Bank {
    pub images: HashMap<String, Arc<Pixmap>>,
    pub effects: HashMap<[u8; 3], Effects>,
}
impl Default for Bank {
    fn default() -> Self {
        let mut effects = HashMap::new();
        for color in super::PINGS
            .iter()
            .map(|p| p.color)
            .chain(std::iter::once([255, 255, 255]))
        {
            effects
                .entry(color)
                .or_insert_with(|| super::paint::effect_sprites(color));
        }
        Self {
            images: HashMap::new(),
            effects,
        }
    }
}
impl Bank {
    pub fn load(root: &Path, data: &Path, settings: &serde_json::Value) -> Self {
        let mut bank = Self::default();
        for def in super::PINGS {
            for key in std::iter::once(def.icon).chain(def.art.iter().copied()) {
                if bank.images.contains_key(key) {
                    continue;
                }
                if let Some(p) = assets::resolve_asset(root, data, &format!("textures/{key}.png"))
                    .and_then(|p| still(&p, 512))
                {
                    bank.images.insert(key.into(), p);
                }
            }
        }
        for id in settings["emoteWheel"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
            .chain(settings["clickEmoteId"].as_str())
            .chain(std::iter::once("facepalm"))
        {
            if bank.images.contains_key(id) {
                continue;
            }
            if let Some(p) = emote_path(root, data, id, false).and_then(|p| still(&p, 256)) {
                bank.images.insert(id.into(), p);
            }
        }
        bank.wheel_variants();
        bank
    }
    pub fn wheel_variants(&mut self) {
        let ping_icons: std::collections::HashSet<_> =
            super::PINGS.iter().map(|p| p.icon).collect();
        let mut variants = Vec::new();
        for (key, image) in &self.images {
            if key.contains('#') {
                continue;
            }
            if ping_icons.contains(key.as_str()) {
                let mut tinted = (**image).clone();
                for pixel in tinted.data_mut().chunks_exact_mut(4) {
                    let grey = 0.2126 * pixel[0] as f32
                        + 0.7152 * pixel[1] as f32
                        + 0.0722 * pixel[2] as f32;
                    let rgb = [grey * 1.26325, grey * 1.15225, grey * 0.95275];
                    let luma = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
                    for c in 0..3 {
                        pixel[c] = ((luma + (rgb[c] - luma) * 1.3) * 1.15)
                            .round()
                            .clamp(0., pixel[3] as f32) as u8;
                    }
                }
                variants.push((format!("{key}#muted"), Arc::new(tinted)));
            } else if !super::PINGS.iter().any(|p| p.art.contains(&key.as_str())) {
                let mut round = (**image).clone();
                let (w, h) = (round.width(), round.height());
                for (i, pixel) in round.data_mut().chunks_exact_mut(4).enumerate() {
                    let dx = (i as u32 % w) as f32 + 0.5 - w as f32 / 2.;
                    let dy = (i as u32 / w) as f32 + 0.5 - h as f32 / 2.;
                    let coverage = (w.min(h) as f32 / 2. - dx.hypot(dy) + 0.5).clamp(0., 1.);
                    for channel in pixel {
                        *channel = (*channel as f32 * coverage).round() as u8;
                    }
                }
                variants.push((format!("{key}#round"), Arc::new(round)));
            }
        }
        self.images.extend(variants);
    }
    pub fn bytes(&self) -> usize {
        self.images.values().map(|p| p.data().len()).sum::<usize>()
            + self
                .effects
                .values()
                .map(|e| e.ring.data().len() + e.glow.data().len())
                .sum::<usize>()
    }
}

pub struct Frame {
    pub pixels: Arc<Pixmap>,
    pub at: Duration,
}
pub struct Animation {
    rx: Receiver<Frame>,
    cancel: Arc<AtomicBool>,
    next: Option<Frame>,
    pub current: Option<Arc<Pixmap>>,
}
impl Animation {
    pub fn advance(&mut self, elapsed: Duration) {
        loop {
            if self.next.is_none() {
                self.next = self.rx.try_recv().ok();
            }
            if self.next.as_ref().is_none_or(|f| f.at > elapsed) {
                break;
            }
            self.current = self.next.take().map(|f| f.pixels);
        }
    }
}
impl Drop for Animation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
struct Job {
    path: PathBuf,
    max: u32,
    looped: bool,
    tx: SyncSender<Frame>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct Queue {
    stop: bool,
    jobs: VecDeque<Job>,
}
pub struct DecoderPool {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    threads: Vec<JoinHandle<()>>,
}
impl DecoderPool {
    pub fn new() -> Self {
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let threads = (0..2)
            .map(|_| {
                let shared = shared.clone();
                std::thread::spawn(move || loop {
                    let job = {
                        let (lock, wake) = &*shared;
                        let mut q = lock.lock().unwrap();
                        while !q.stop && q.jobs.is_empty() {
                            q = wake.wait(q).unwrap();
                        }
                        if q.stop {
                            break;
                        }
                        q.jobs.pop_front().unwrap()
                    };
                    if !job.cancel.load(Ordering::Relaxed) {
                        if let Err(e) = decode(&job) {
                            eprintln!("Animation: {e}");
                        }
                    }
                })
            })
            .collect();
        Self { shared, threads }
    }
    pub fn play(&self, path: PathBuf, max: u32, looped: bool) -> Animation {
        let (tx, rx) = mpsc::sync_channel(2);
        let cancel = Arc::new(AtomicBool::new(false));
        let (lock, wake) = &*self.shared;
        let mut q = lock.lock().unwrap();
        q.jobs.retain(|j| !j.cancel.load(Ordering::Relaxed));
        // At most two owners (self/preview) are active. Bound pending requests even during floods.
        while q.jobs.len() >= 2 {
            q.jobs.pop_front();
        }
        q.jobs.push_back(Job {
            path,
            max: max.clamp(1, 768),
            looped,
            tx,
            cancel: cancel.clone(),
        });
        wake.notify_one();
        Animation {
            rx,
            cancel,
            next: None,
            current: None,
        }
    }
}
impl Drop for DecoderPool {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        {
            let mut q = lock.lock().unwrap();
            q.stop = true;
            q.jobs.clear();
        }
        wake.notify_all();
        // Scenes (and frame receivers) must be dropped before the pool, unblocking send().
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
fn decode(job: &Job) -> Result<(), String> {
    let bytes = fs::read(&job.path).map_err(|e| e.to_string())?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("animationTooLarge".into());
    }
    let format = image::guess_format(&bytes).map_err(|e| e.to_string())?;
    let mut time = Duration::ZERO;
    loop {
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1024);
        limits.max_image_height = Some(1024);
        limits.max_alloc = Some(32 * 1024 * 1024);
        let reader = BufReader::new(Cursor::new(&bytes));
        let frames = match format {
            ImageFormat::WebP => {
                let mut d =
                    image::codecs::webp::WebPDecoder::new(reader).map_err(|e| e.to_string())?;
                d.set_limits(limits).map_err(|e| e.to_string())?;
                d.into_frames()
            }
            ImageFormat::Gif => {
                let mut d =
                    image::codecs::gif::GifDecoder::new(reader).map_err(|e| e.to_string())?;
                d.set_limits(limits).map_err(|e| e.to_string())?;
                d.into_frames()
            }
            _ => return Err("unsupportedAnimation".into()),
        };
        let start = time;
        for frame in frames {
            if job.cancel.load(Ordering::Relaxed) || time >= Duration::from_millis(3150) {
                return Ok(());
            }
            let frame = frame.map_err(|e| e.to_string())?;
            let (n, d) = frame.delay().numer_denom_ms();
            let delay = Duration::from_secs_f64((n as f64 / d.max(1) as f64 / 1000.).max(0.01));
            let pixels = Arc::new(pixmap(frame.into_buffer(), job.max).ok_or("invalidFrame")?);
            if job.tx.send(Frame { pixels, at: time }).is_err() {
                return Ok(());
            }
            time += delay;
        }
        if !job.looped || time == start {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_animated_emotes_stream_in_order_with_bounded_frames() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/emotes");
        let pool = DecoderPool::new();
        for id in crate::settings::core_emotes() {
            let animation = pool.play(root.join(format!("{id}.webp")), 384, false);
            let mut count = 0;
            let mut last = Duration::ZERO;
            while let Ok(frame) = animation.rx.recv_timeout(Duration::from_secs(5)) {
                assert!(frame.at >= last);
                assert!(frame.pixels.data().len() <= 384 * 384 * 4);
                last = frame.at;
                count += 1;
            }
            assert!(count > 2, "{id} must retain its original animation");
        }
    }
    #[test]
    fn cancelled_streams_do_not_block_replacement_or_shutdown() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/emotes/facepalm.webp");
        let pool = DecoderPool::new();
        for _ in 0..40 {
            let stream = pool.play(path.clone(), 320, false);
            drop(stream);
        }
        let stream = pool.play(path, 320, false);
        assert!(stream.rx.recv_timeout(Duration::from_secs(5)).is_ok());
        drop(stream);
        drop(pool);
    }
    #[test]
    fn premultiplication_and_resizing_preserve_alpha() {
        let rgba = image::RgbaImage::from_pixel(8, 4, image::Rgba([200, 100, 50, 128]));
        let p = pixmap(rgba, 4).unwrap();
        assert_eq!((p.width(), p.height()), (4, 2));
        assert_eq!(&p.data()[..4], &[100, 50, 25, 128]);
    }
    #[test]
    fn custom_gifs_stream_and_loop_without_storing_the_entire_animation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.gif");
        let file = fs::File::create(&path).unwrap();
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(32, 16, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                ))
                .unwrap();
        }
        drop(encoder);
        let pool = DecoderPool::new();
        let stream = pool.play(path, 64, true);
        for i in 0..6 {
            let frame = stream.rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(frame.at, Duration::from_millis(i * 100));
            assert_eq!(
                frame.pixels.pixel(0, 0).unwrap().red(),
                if i % 2 == 0 { 255 } else { 0 }
            );
        }
        drop(stream);
    }
}
