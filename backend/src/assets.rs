use crate::settings::{atomic_json, catalog, core_emotes};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{ImageFormat, ImageReader};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
};

pub const REVISION: &str = "b947aab8d379015687117544191160bd82141954";
pub fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn safe_relative(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.contains(['\\', ':', '\0', '%'])
        && !raw.starts_with('/')
        && Path::new(raw)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && raw
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}
pub fn custom_id(id: &str) -> Option<&str> {
    id.strip_prefix("c:").filter(|s| {
        s.len() == 32
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
pub fn pack_dir(data: &Path) -> PathBuf {
    data.join("packs").join(REVISION)
}
pub fn resolve_asset(root: &Path, data: &Path, raw: &str) -> Option<PathBuf> {
    let raw = raw.trim_start_matches('/');
    if !safe_relative(raw) {
        return None;
    }
    let candidate = if let Some(file) = raw.strip_prefix("custom/") {
        let stem = file.split('.').next()?;
        if custom_id(&format!("c:{stem}")).is_none()
            || !["png", "anim", "sound"].contains(&file.rsplit('.').next()?)
        {
            return None;
        }
        data.join("custom").join(file)
    } else if raw.starts_with("emotes/") && pack_dir(data).join(raw).is_file() {
        pack_dir(data).join(raw)
    } else {
        root.join("ui").join(raw)
    };
    // Local symlinks must not let image/audio readers escape the approved asset roots.
    let canonical = candidate.canonicalize().ok()?;
    let allowed = [root.join("ui"), data.join("custom"), pack_dir(data)]
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .any(|base| canonical.starts_with(base));
    allowed.then_some(canonical)
}
pub fn library(root: &Path, data: &Path, custom: &Value) -> Vec<Value> {
    library_items(root, data, custom, true)
}
pub fn library_metadata(root: &Path, data: &Path, custom: &Value) -> Vec<Value> {
    library_items(root, data, custom, false)
}
fn library_items(root: &Path, data: &Path, custom: &Value, thumbnails: bool) -> Vec<Value> {
    let mut out = Vec::new();
    for e in catalog().as_array().unwrap() {
        let id = e["slug"].as_str().unwrap();
        if let Some(path) = resolve_asset(root, data, &format!("emotes/{id}.png")) {
            let mut item = e.clone();
            item["id"] = json!(id);
            item["custom"] = json!(false);
            if thumbnails {
                item["thumbnail"] = json!(format!(
                    "data:image/png;base64,{}",
                    STANDARD.encode(fs::read(path).unwrap_or_default())
                ));
            }
            out.push(item);
        }
    }
    for e in custom.as_array().into_iter().flatten().take(24) {
        let Some(hash) = e["id"].as_str().and_then(custom_id) else {
            continue;
        };
        let path = data.join("custom").join(format!("{hash}.png"));
        if path
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= 131072)
        {
            let mut item = e.clone();
            item["custom"] = json!(true);
            if thumbnails {
                item["thumbnail"] = json!(format!(
                    "data:image/png;base64,{}",
                    STANDARD.encode(fs::read(path).unwrap_or_default())
                ));
            }
            out.push(item);
        }
    }
    out
}

pub fn import_pack(bytes: &[u8], data: &Path) -> Result<(), String> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("packTooLarge".into());
    }
    let upstream: Value =
        serde_json::from_str(include_str!("../../resources/upstream.json")).unwrap();
    let core = core_emotes();
    let expected: HashSet<String> = upstream["files"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|p| {
            p.starts_with("emotes/")
                && !core.contains(Path::new(p).file_stem().unwrap().to_str().unwrap())
        })
        .cloned()
        .collect();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "invalidPack")?;
    if zip.len() != expected.len() + 1 {
        return Err("invalidPackEntries".into());
    }
    let mut seen = HashSet::new();
    let mut extracted = Vec::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|_| "invalidPack")?;
        let name = file.name().map_err(|_| "invalidPackPath")?.into_owned();
        if !safe_relative(&name)
            || !seen.insert(name.clone())
            || file.is_dir()
            || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err("unsafePackPath".into());
        }
        let max = if name == "pack.json" {
            65536
        } else {
            8 * 1024 * 1024
        };
        if file.size() > max {
            return Err("packTooLarge".into());
        }
        total += file.size();
        if total > 64 * 1024 * 1024 {
            return Err("packTooLarge".into());
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "invalidPack")?;
        if bytes.len() as u64 > max {
            return Err("packTooLarge".into());
        }
        if name == "pack.json" {
            let p: Value = serde_json::from_slice(&bytes).map_err(|_| "invalidPack")?;
            if p["version"] != 1 || p["id"] != "lolping-extra-emotes" || p["upstream"] != REVISION {
                return Err("incompatiblePack".into());
            }
            let actual: HashSet<_> = p["files"]
                .as_object()
                .ok_or("invalidPack")?
                .keys()
                .cloned()
                .collect();
            if actual != expected || actual.iter().any(|k| p["files"][k] != upstream["files"][k]) {
                return Err("packChecksum".into());
            }
        } else {
            if !expected.contains(&name)
                || upstream["files"][&name]["sha256"] != sha(&bytes)
                || upstream["files"][&name]["size"].as_u64() != Some(bytes.len() as u64)
            {
                return Err("packChecksum".into());
            }
            extracted.push((name, bytes));
        }
    }
    if !seen.contains("pack.json") {
        return Err("invalidPack".into());
    }
    let destination = pack_dir(data);
    if destination.is_dir() {
        // Reimports must verify every stored file rather than silently accepting corruption.
        if extracted
            .iter()
            .all(|(p, b)| fs::read(destination.join(p)).ok().as_deref() == Some(b.as_slice()))
        {
            return Ok(());
        }
        return Err("packNeedsRemoval".into());
    }
    let stage = data
        .join("packs")
        .join(format!(".import-{}", std::process::id()));
    fs::create_dir_all(stage.join("emotes")).map_err(|e| e.to_string())?;
    let result = (|| {
        for (path, bytes) in extracted {
            fs::write(stage.join(path), bytes).map_err(|e| e.to_string())?;
        }
        fs::rename(&stage, &destination).map_err(|e| e.to_string())
    })();
    if stage.exists() {
        let _ = fs::remove_dir_all(stage);
    }
    result
}

pub fn import_image(
    bytes: &[u8],
    name: &str,
    data: &Path,
    settings: &mut Value,
) -> Result<String, String> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("imageTooLarge".into());
    }
    let format = image::guess_format(bytes).map_err(|_| "imageType")?;
    if ![
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::Gif,
        ImageFormat::WebP,
    ]
    .contains(&format)
    {
        return Err("imageType".into());
    }
    let (w, h) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| "imageType")?;
    if w == 0 || h == 0 || w.max(h) > 8192 || u64::from(w) * u64::from(h) > 16_777_216 {
        return Err("imageTooLarge".into());
    }
    let animated = format == ImageFormat::Gif
        || (format == ImageFormat::WebP && bytes.windows(4).any(|w| w == b"ANIM"));
    if animated && (w.max(h) > 1024 || bytes.len() > 5 * 1024 * 1024) {
        return Err("animationTooLarge".into());
    }
    let hash = &sha(bytes)[..32];
    let id = format!("c:{hash}");
    let custom = settings["customEmotes"]
        .as_array_mut()
        .ok_or("invalidSettings")?;
    if custom.iter().any(|c| c["id"] == id) {
        return Err("duplicateEmote".into());
    }
    if custom.len() >= 24 {
        return Err("emoteLimit".into());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| "imageType")?;
    let thumb = image.thumbnail(160, 160);
    let mut png = Cursor::new(Vec::new());
    thumb
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|_| "imageType")?;
    let dir = data.join("custom");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(format!("{hash}.png")), png.into_inner()).map_err(|e| e.to_string())?;
    if animated {
        fs::write(dir.join(format!("{hash}.anim")), bytes).map_err(|e| e.to_string())?;
    }
    let name: String = name.chars().filter(|c| !c.is_control()).take(24).collect();
    custom.push(json!({"id":id,"name":if name.trim().is_empty(){hash.to_string()}else{name},"animated":animated}));
    atomic_json(&data.join("settings.json"), settings)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_escape_paths() {
        for p in ["../a", "/a", "a\\b", "a/../b", "C:x", "a/%2e%2e/x", "a//b"] {
            assert!(!safe_relative(p), "{p}");
        }
        assert!(safe_relative("emotes/a.webp"));
    }
    #[test]
    fn imports_image_and_enforces_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = crate::settings::defaults();
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(20, 20)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let id = import_image(bytes.get_ref(), "hello", dir.path(), &mut s).unwrap();
        assert!(custom_id(&id).is_some());
        assert_eq!(
            import_image(bytes.get_ref(), "again", dir.path(), &mut s).unwrap_err(),
            "duplicateEmote"
        );
    }
    #[test]
    fn corrupt_pack_cannot_create_installed_directory() {
        let d = tempfile::tempdir().unwrap();
        assert!(import_pack(b"not a zip", d.path()).is_err());
        assert!(!pack_dir(d.path()).exists());
    }
}
