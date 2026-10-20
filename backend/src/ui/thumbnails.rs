//! Bounded background decoding for settings thumbnails. The UI only uploads
//! small RGBA images; opening a tab or resizing must not decode image files.
use base64::Engine;
use eframe::egui::{self, ColorImage, TextureHandle};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Cursor,
    sync::mpsc,
};

const CACHE_SIZE: usize = 48;
const IN_FLIGHT: usize = 8;
const UPLOADS_PER_PASS: usize = 2;
const MAX_ENCODED_BYTES: usize = 2 * 1024 * 1024;

pub(super) enum Source {
    Embedded(&'static [u8]),
    DataUri(String),
}

struct Job {
    id: String,
    source: Source,
}

struct Decoded {
    id: String,
    pixels: Option<ColorImage>,
}

struct Worker {
    sender: mpsc::SyncSender<Job>,
    results: mpsc::Receiver<Decoded>,
    #[cfg(test)]
    alive: std::sync::Weak<()>,
}

#[derive(Default)]
pub(super) struct Cache {
    // None remembers a failed decode, avoiding retries on every resize frame.
    entries: HashMap<String, Option<TextureHandle>>,
    recent: VecDeque<String>,
    pending: HashSet<String>,
    worker: Option<Worker>,
    #[cfg(test)]
    pub synchronous: bool,
}

impl Cache {
    fn touch(&mut self, id: &str) {
        if let Some(index) = self.recent.iter().position(|item| item == id) {
            self.recent.remove(index);
        }
        self.recent.push_back(id.to_owned());
    }

    pub fn get(&mut self, id: &str) -> Option<TextureHandle> {
        let texture = self.entries.get(id)?.clone();
        self.touch(id);
        texture
    }

    // Catalog enumeration must not make off-screen items look recently used.
    pub fn peek(&self, id: &str) -> Option<&TextureHandle> {
        self.entries.get(id)?.as_ref()
    }

    pub fn request(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        source: impl FnOnce() -> Source,
    ) -> Option<TextureHandle> {
        if self.entries.contains_key(id) {
            return self.get(id);
        }
        if self.pending.contains(id) || self.pending.len() >= IN_FLIGHT {
            return None;
        }
        #[cfg(test)]
        if self.synchronous {
            let texture = decode(source())
                .map(|pixels| ctx.load_texture(id, pixels, egui::TextureOptions::LINEAR));
            self.remember(id.to_owned(), texture);
            return self.get(id);
        }
        let worker = self
            .worker
            .get_or_insert_with(|| Worker::start(ctx.clone()));
        if worker
            .sender
            .try_send(Job {
                id: id.to_owned(),
                source: source(),
            })
            .is_ok()
        {
            self.pending.insert(id.to_owned());
        }
        None
    }

    fn remember(&mut self, id: String, texture: Option<TextureHandle>) {
        while self.entries.len() >= CACHE_SIZE && !self.entries.contains_key(&id) {
            if let Some(old) = self.recent.pop_front() {
                self.entries.remove(&old);
            } else {
                break;
            }
        }
        self.touch(&id);
        self.entries.insert(id, texture);
    }

    pub fn upload_ready(&mut self, ctx: &egui::Context) {
        for _ in 0..UPLOADS_PER_PASS {
            let Some(decoded) = self
                .worker
                .as_ref()
                .and_then(|worker| worker.results.try_recv().ok())
            else {
                break;
            };
            self.pending.remove(&decoded.id);
            let texture = decoded
                .pixels
                .map(|pixels| ctx.load_texture(&decoded.id, pixels, egui::TextureOptions::LINEAR));
            self.remember(decoded.id, texture);
            // The draw that requested an image might already be complete.
            // Ensure remaining ready uploads and newly-visible tiles progress.
            ctx.request_repaint();
        }
    }
}

impl Worker {
    fn start(wake: egui::Context) -> Self {
        let (sender, requests) = mpsc::sync_channel::<Job>(IN_FLIGHT);
        let (output, results) = mpsc::sync_channel(IN_FLIGHT);
        #[cfg(test)]
        let keep_alive = std::sync::Arc::new(());
        #[cfg(test)]
        let alive = std::sync::Arc::downgrade(&keep_alive);
        std::thread::spawn(move || {
            #[cfg(test)]
            let _keep_alive = keep_alive;
            for job in requests {
                let pixels = decode(job.source);
                if output.send(Decoded { id: job.id, pixels }).is_err() {
                    break;
                }
                wake.request_repaint();
            }
        });
        Self {
            sender,
            results,
            #[cfg(test)]
            alive,
        }
    }
}

fn decode(source: Source) -> Option<ColorImage> {
    let owned;
    let bytes = match source {
        Source::Embedded(bytes) => bytes,
        Source::DataUri(source) => {
            let (_, data) = source.split_once(',')?;
            if data.len() > MAX_ENCODED_BYTES {
                return None;
            }
            owned = base64::engine::general_purpose::STANDARD
                .decode(data)
                .ok()?;
            &owned
        }
    };
    if bytes.len() > MAX_ENCODED_BYTES {
        return None;
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.thumbnail(128, 128).to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        &rgba,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const PNG: &[u8] = include_bytes!("../../../resources/emotes/bee_happy.png");

    fn finish(cache: &mut Cache, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cache.pending.is_empty() && Instant::now() < deadline {
            cache.upload_ready(ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(cache.pending.is_empty(), "decode worker did not finish");
    }

    #[test]
    fn repeated_resize_requests_decode_once_and_reuse_the_texture() {
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        assert!(cache
            .request(&ctx, "visible", || Source::Embedded(PNG))
            .is_none());
        for _ in 0..100 {
            assert!(cache
                .request(&ctx, "visible", || panic!("duplicate decode"))
                .is_none());
        }
        finish(&mut cache, &ctx);
        let texture = cache.get("visible").unwrap();
        for _ in 0..100 {
            assert_eq!(
                cache
                    .request(&ctx, "visible", || panic!("cached decode"))
                    .unwrap()
                    .id(),
                texture.id()
            );
        }
        assert!(texture.size().iter().all(|side| *side <= 128));
    }

    #[test]
    fn decode_queue_and_cache_remain_bounded_and_visible_images_survive_eviction() {
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        for index in 0..100 {
            cache.request(&ctx, &format!("item-{index}"), || Source::Embedded(PNG));
        }
        assert_eq!(cache.pending.len(), IN_FLIGHT);
        finish(&mut cache, &ctx);
        let visible = cache.get("item-0").unwrap().id();
        for index in IN_FLIGHT..(CACHE_SIZE * 2) {
            cache.request(&ctx, &format!("item-{index}"), || Source::Embedded(PNG));
            finish(&mut cache, &ctx);
            assert_eq!(cache.get("item-0").unwrap().id(), visible);
            assert!(cache.entries.len() <= CACHE_SIZE);
        }
        assert!(!cache.entries.contains_key("item-1"));
        assert_eq!(cache.entries.len(), CACHE_SIZE);
    }

    #[test]
    fn bad_thumbnail_is_not_retried_on_every_frame() {
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        cache.request(&ctx, "bad", || {
            Source::DataUri("data:image/png;base64,broken".into())
        });
        finish(&mut cache, &ctx);
        for _ in 0..100 {
            assert!(cache
                .request(&ctx, "bad", || panic!("retry storm"))
                .is_none());
        }
        assert!(decode(Source::DataUri(format!(
            "data:image/png;base64,{}",
            "x".repeat(MAX_ENCODED_BYTES + 1)
        )))
        .is_none());
    }

    #[test]
    fn closing_settings_drops_pending_work_without_blocking_the_ui() {
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        for index in 0..IN_FLIGHT {
            cache.request(&ctx, &format!("item-{index}"), || Source::Embedded(PNG));
        }
        // Dropping the result receiver releases a worker blocked on delivery.
        let alive = cache.worker.as_ref().unwrap().alive.clone();
        drop(cache);
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive.strong_count() != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            alive.strong_count(),
            0,
            "decode thread survived settings close"
        );
    }
}
