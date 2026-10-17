use crate::assets::{custom_id, resolve_asset};
use crate::cache::Cache;
use rodio::{buffer::SamplesBuffer, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::{
    fs,
    io::Cursor,
    path::Path,
    time::{Duration, Instant},
};

enum Work {
    Play {
        epoch: u64,
        key: String,
        volume: f32,
    },
    Clear(std::sync::mpsc::SyncSender<()>),
    Invalidate,
    Quit,
}
/// Device startup and Opus decoding must not stall the native rendering thread.
pub struct AudioWorker {
    tx: std::sync::mpsc::SyncSender<Work>,
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    bytes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl AudioWorker {
    pub fn new(
        root: std::path::PathBuf,
        data: std::path::PathBuf,
        on_error: impl Fn(String) + Send + 'static,
    ) -> Self {
        use std::sync::{
            atomic::{AtomicU64, AtomicUsize, Ordering},
            mpsc, Arc,
        };
        let (tx, rx) = mpsc::sync_channel(32);
        let epoch = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicUsize::new(0));
        let generation = epoch.clone();
        let counter = bytes.clone();
        let thread = std::thread::spawn(move || {
            let mut audio = Audio::default();
            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(Work::Play { epoch, key, volume }) => {
                        if generation.load(Ordering::Acquire) == epoch {
                            if let Err(e) = audio.play(&root, &data, &key, volume) {
                                on_error(e);
                            }
                            if generation.load(Ordering::Acquire) != epoch {
                                audio.clear();
                            }
                        }
                    }
                    Ok(Work::Clear(ack)) => {
                        audio.clear();
                        counter.store(0, Ordering::Release);
                        let _ = ack.send(());
                    }
                    Ok(Work::Invalidate) => audio.invalidate_custom(),
                    Ok(Work::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                audio.maintain();
                counter.store(audio.cached_bytes(), Ordering::Release);
            }
        });
        Self {
            tx,
            epoch,
            bytes,
            thread: Some(thread),
        }
    }
    pub fn play(&self, _root: &Path, _data: &Path, key: &str, volume: f32) -> Result<(), String> {
        use std::sync::{atomic::Ordering, mpsc::TrySendError};
        if volume <= 0. {
            return Ok(());
        }
        match self.tx.try_send(Work::Play {
            epoch: self.epoch.load(Ordering::Acquire),
            key: key.into(),
            volume,
        }) {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()), // Audio floods never block input or painting.
            Err(TrySendError::Disconnected(_)) => Err("audioWorkerExited".into()),
        }
    }
    pub fn clear(&self) {
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        if self.tx.send(Work::Clear(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(1));
        }
    }
    pub fn invalidate_custom(&self) {
        let _ = self.tx.send(Work::Invalidate);
    }
    pub fn cached_bytes(&self) -> usize {
        self.bytes.load(std::sync::atomic::Ordering::Acquire)
    }
}
impl Drop for AudioWorker {
    fn drop(&mut self) {
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let _ = self.tx.send(Work::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    #[test]
    fn invalid_sound_reports_off_thread_and_clear_drains_pending_playback() {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = AudioWorker::new("missing-root".into(), "missing-data".into(), move |e| {
            let _ = tx.send(e);
        });
        worker
            .play(Path::new(""), Path::new(""), "invalid/path", 0.5)
            .unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            "invalidSound"
        );
        for _ in 0..100 {
            worker
                .play(Path::new(""), Path::new(""), "missing", 0.5)
                .unwrap();
        }
        worker.clear();
        assert_eq!(worker.cached_bytes(), 0);
        drop(worker);
    }
}

pub struct Audio {
    device: Option<MixerDeviceSink>,
    cache: Cache<SamplesBuffer>,
    voices: Vec<Player>,
}
impl Default for Audio {
    fn default() -> Self {
        Self {
            device: None,
            cache: Cache::new(8 * 1024 * 1024, Duration::from_secs(120)),
            voices: Vec::new(),
        }
    }
}
impl Audio {
    pub fn play(&mut self, root: &Path, data: &Path, key: &str, volume: f32) -> Result<(), String> {
        self.voices.retain(|p| !p.empty());
        if volume <= 0. || self.voices.len() >= 8 {
            return Ok(());
        }
        let relative = if let Some(id) = key.strip_prefix("emote:") {
            if let Some(hash) = custom_id(id) {
                format!("custom/{hash}.sound")
            } else if id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                format!("emotes/{id}.ogg")
            } else {
                return Err("invalidSound".into());
            }
        } else if key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            format!("sounds/{key}.wav")
        } else {
            return Err("invalidSound".into());
        };
        let now = Instant::now();
        let samples = if let Some(samples) = self.cache.get(key, now) {
            samples.clone()
        } else {
            let path = resolve_asset(root, data, &relative).ok_or("missingSound")?;
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            let samples = decode(&bytes, 30)?;
            self.cache.insert(
                key.into(),
                samples.clone(),
                samples.len() * std::mem::size_of::<f32>(),
                now,
            );
            samples
        };
        if self.device.is_none() {
            self.device = Some(DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?);
        }
        let voice = Player::connect_new(self.device.as_ref().unwrap().mixer());
        voice.set_volume(volume.clamp(0., 1.));
        voice.append(samples);
        self.voices.push(voice);
        Ok(())
    }
    pub fn clear(&mut self) {
        self.voices.clear();
        self.cache.clear();
        self.device = None;
    }
    pub fn maintain(&mut self) {
        self.voices.retain(|p| !p.empty());
        self.cache.trim(Instant::now());
    }
    pub fn invalidate_custom(&mut self) {
        self.cache.retain(|k| !k.starts_with("emote:c:"));
    }
    pub fn cached_bytes(&self) -> usize {
        self.cache.bytes()
    }
}
pub fn validate_sound(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > 1024 * 1024 {
        return Err("soundTooLarge".into());
    }
    decode(bytes, 5).map(|_| ())
}

fn decode(bytes: &[u8], seconds: usize) -> Result<SamplesBuffer, String> {
    // Riot's emotes use Ogg Opus; rodio's Ogg decoder handles Vorbis only.
    if bytes.starts_with(b"OggS") {
        let mut reader = ogg::PacketReader::new(Cursor::new(bytes));
        if let Some(head) = reader.read_packet().map_err(|_| "soundType")? {
            if head.data.starts_with(b"OpusHead") {
                return decode_opus(reader, head, seconds);
            }
        }
    }
    let decoder = Decoder::try_from(Cursor::new(bytes.to_vec())).map_err(|_| "soundType")?;
    let (channels, rate) = (decoder.channels(), decoder.sample_rate());
    if channels.get() > 8 || rate.get() > 192_000 {
        return Err("soundType".into());
    }
    let max = usize::from(channels.get()) * rate.get() as usize * seconds;
    let samples: Vec<_> = decoder.take(max + 1).collect();
    if samples.len() > max {
        return Err("soundTooLong".into());
    }
    if samples.is_empty() {
        return Err("soundType".into());
    }
    Ok(SamplesBuffer::new(channels, rate, samples))
}

fn decode_opus(
    mut reader: ogg::PacketReader<Cursor<&[u8]>>,
    head: ogg::Packet,
    seconds: usize,
) -> Result<SamplesBuffer, String> {
    let h = &head.data;
    if h.len() < 19 || h[8] > 15 || !(1..=2).contains(&h[9]) || h[18] != 0 {
        return Err("soundType".into());
    }
    let channels = h[9] as usize;
    let skip = u16::from_le_bytes([h[10], h[11]]) as usize;
    let gain = 10f32.powf(i16::from_le_bytes([h[16], h[17]]) as f32 / (256. * 20.));
    let serial = head.stream_serial();
    let mut decoder = opus::Decoder::new(
        48_000,
        if channels == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        },
    )
    .map_err(|_| "soundType")?;
    let tags = reader
        .read_packet()
        .map_err(|_| "soundType")?
        .ok_or("soundType")?;
    if tags.stream_serial() != serial || !tags.data.starts_with(b"OpusTags") {
        return Err("soundType".into());
    }
    let max = seconds * 48_000 * channels;
    let mut pcm = vec![0.; 5760 * channels]; // Opus packets contain at most 120 ms.
    let mut samples = Vec::new();
    let mut frames_decoded = 0usize;
    let mut ended = false;
    while let Some(packet) = reader.read_packet().map_err(|_| "soundType")? {
        if ended || packet.stream_serial() != serial {
            return Err("soundType".into());
        }
        let frames = decoder
            .decode_float(&packet.data, &mut pcm, false)
            .map_err(|_| "soundType")?;
        let start = skip.saturating_sub(frames_decoded).min(frames);
        let end = if packet.last_in_stream() {
            ended = true;
            usize::try_from(packet.absgp_page())
                .map_err(|_| "soundType")?
                .saturating_sub(frames_decoded)
                .min(frames)
        } else {
            frames
        };
        if end > start {
            if samples.len() + (end - start) * channels > max {
                return Err("soundTooLong".into());
            }
            samples.extend(
                pcm[start * channels..end * channels]
                    .iter()
                    .map(|s| s * gain),
            );
        }
        frames_decoded += frames;
        if frames_decoded > seconds * 48_000 + skip + 5760 {
            return Err("soundTooLong".into());
        }
    }
    if !ended || samples.is_empty() {
        return Err("soundType".into());
    }
    Ok(SamplesBuffer::new(
        (channels as u16).try_into().unwrap(),
        48_000u32.try_into().unwrap(),
        samples,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_original_audio_decodes_without_transcoding() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
        for folder in ["emotes", "sounds"] {
            for file in fs::read_dir(root.join(folder)).unwrap().flatten() {
                let path = file.path();
                if matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("ogg" | "wav")
                ) {
                    assert!(
                        decode(&fs::read(&path).unwrap(), 30).is_ok(),
                        "{}",
                        path.display()
                    );
                }
            }
        }
    }
    #[test]
    fn custom_audio_rejects_invalid_oversized_and_long_files() {
        assert_eq!(validate_sound(b"bad").unwrap_err(), "soundType");
        assert_eq!(
            validate_sound(&vec![0; 1024 * 1024 + 1]).unwrap_err(),
            "soundTooLarge"
        );
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/emotes/facepalm.ogg");
        let bytes = fs::read(file).unwrap();
        assert!(validate_sound(&bytes).is_ok());
        assert_eq!(decode(&bytes, 0).unwrap_err(), "soundTooLong");
        assert!(validate_sound(&bytes[..bytes.len() / 2]).is_err());
    }
}
