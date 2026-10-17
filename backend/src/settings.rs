use serde_json::{json, Value};
use std::{collections::HashSet, fs, io::Write, path::Path};

pub fn defaults() -> Value {
    serde_json::from_str(include_str!("../../resources/default-settings.json")).unwrap()
}
pub fn catalog() -> Value {
    serde_json::from_str(include_str!("../../resources/catalog.json")).unwrap()
}
pub fn core_emotes() -> HashSet<String> {
    let d = defaults();
    d["emoteWheel"]
        .as_array()
        .unwrap()
        .iter()
        .chain(std::iter::once(&d["clickEmoteId"]))
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
pub const PINGS: &[&str] = &[
    "danger",
    "push",
    "omw",
    "allin",
    "assist",
    "needvision",
    "missing",
    "enemyvision",
    "generic",
    "bait",
    "visioncleared",
    "missingflip",
];

fn valid_trigger(v: &Value, mac: bool, off: bool) -> bool {
    if let Some(s) = v.as_str() {
        return ["alt", "ctrl", "shift", "win", "mouse4", "mouse5"].contains(&s)
            || (!mac && s == "capslock")
            || (off && s == "off");
    }
    v["vk"].as_u64().is_some_and(|k| {
        (1..=254).contains(&k)
            && ![16, 17, 18, 91, 92, 160, 161, 162, 163, 164, 165].contains(&k)
            && (!mac || k != 20)
    })
}
fn trigger_vk(v: &Value) -> Option<u64> {
    if v == "capslock" {
        Some(20)
    } else {
        v["vk"].as_u64()
    }
}

pub fn normalize(raw: &Value, available: &HashSet<String>, mac: bool) -> Value {
    let mut s = defaults();
    for (key, default) in s.as_object_mut().unwrap().iter_mut() {
        if let Some(v) = raw.get(key) {
            if (v.is_boolean() && default.is_boolean())
                || (v.is_number() && default.is_number())
                || (v.is_string() && default.is_string())
                || (v.is_array() && default.is_array())
                || (v.is_object() && default.is_object())
                || (key == "trigger" || key == "emoteTrigger")
            {
                *default = v.clone();
            }
        }
    }
    s["version"] = json!(1);
    for (key, min, max, step) in [
        ("dragThresholdPx", 2., 40., 1.),
        ("wheelSizePx", 300., 900., 10.),
        ("pingSizePx", 60., 220., 1.),
        ("pingDurationS", 1., 8., 0.1),
        ("volume", 0., 100., 1.),
        ("emoteSizePx", 80., 300., 1.),
    ] {
        let n = s[key].as_f64().unwrap_or(defaults()[key].as_f64().unwrap());
        let n = ((n.clamp(min, max) / step).round() * step * 10.).round() / 10.;
        s[key] = if step >= 1. {
            json!(n as u64)
        } else {
            json!(n)
        };
    }
    if !["auto", "en", "zh-CN"].contains(&s["language"].as_str().unwrap_or("")) {
        s["language"] = json!("auto");
    }
    if !valid_trigger(&s["trigger"], mac, false) {
        s["trigger"] = json!("alt");
    }
    if !valid_trigger(&s["emoteTrigger"], mac, true) {
        s["emoteTrigger"] = json!("ctrl");
    }
    let hotkey = &s["toggleHotkey"];
    if !hotkey["mods"]
        .as_u64()
        .is_some_and(|n| (1..=15).contains(&n) && n & 11 != 0)
        || !hotkey["vk"]
            .as_u64()
            .is_some_and(|n| (1..=254).contains(&n) && ![16, 17, 18, 91, 92].contains(&n))
    {
        s["toggleHotkey"] = defaults()["toggleHotkey"].clone();
    }
    if trigger_vk(&s["trigger"]) == s["toggleHotkey"]["vk"].as_u64() {
        s["trigger"] = json!("alt");
    }
    if s["trigger"] == s["emoteTrigger"]
        || (trigger_vk(&s["trigger"]).is_some()
            && trigger_vk(&s["trigger"]) == trigger_vk(&s["emoteTrigger"]))
        || trigger_vk(&s["emoteTrigger"]) == s["toggleHotkey"]["vk"].as_u64()
    {
        s["emoteTrigger"] = json!("off");
    }
    let valid_wheel = |v: &Value, known: &dyn Fn(&str) -> bool| {
        v.as_array().is_some_and(|a| {
            a.len() == 8
                && a.iter().all(|v| v.as_str().is_some_and(known))
                && a.iter()
                    .filter_map(Value::as_str)
                    .collect::<HashSet<_>>()
                    .len()
                    == 8
        })
    };
    if !valid_wheel(&s["wheel"], &|p| PINGS.contains(&p)) {
        s["wheel"] = defaults()["wheel"].clone();
    }
    if !PINGS.contains(&s["clickPingId"].as_str().unwrap_or("")) {
        s["clickPingId"] = json!("generic");
    }
    if !valid_wheel(&s["emoteWheel"], &|p| available.contains(p)) {
        s["emoteWheel"] = defaults()["emoteWheel"].clone();
    }
    if !available.contains(s["clickEmoteId"].as_str().unwrap_or("")) {
        s["clickEmoteId"] = json!("facepalm");
    }
    s
}

pub fn helper_config(s: &Value, enabled: bool) -> Value {
    json!({"type":"config", "trigger":s["trigger"].as_str().unwrap_or("vk"), "triggerVk":s["trigger"]["vk"].as_u64().unwrap_or(0),
        "emoteTrigger":s["emoteTrigger"].as_str().unwrap_or("vk"), "emoteTriggerVk":s["emoteTrigger"]["vk"].as_u64().unwrap_or(0),
        "clickPing":s["clickPing"], "emoteClick":s["emoteClick"], "gameCheck":s["gameCheck"],
        "dragThresholdPx":s["dragThresholdPx"], "toggleMods":s["toggleHotkey"]["mods"], "toggleVk":s["toggleHotkey"]["vk"], "enabled":enabled})
}
pub fn atomic_json(path: &Path, data: &Value) -> Result<(), String> {
    let temp = path.with_extension("tmp");
    let mut f = fs::File::create(&temp).map_err(|e| e.to_string())?;
    f.write_all(&serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    fs::rename(temp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_and_legacy_settings_are_normalized() {
        let s = normalize(
            &json!({"volume":900,"wheel":["danger"],"roomCode":"secret","language":"unknown"}),
            &core_emotes(),
            false,
        );
        assert_eq!(s["volume"], 100);
        assert_eq!(s["wheel"], defaults()["wheel"]);
        assert!(s.get("roomCode").is_none());
        assert_eq!(s["language"], "auto");
    }
    #[test]
    fn triggers_and_shift_only_shortcuts_are_safe() {
        let s = normalize(
            &json!({"trigger":"capslock", "emoteTrigger":"alt", "toggleHotkey":{"mods":4,"vk":65}}),
            &core_emotes(),
            true,
        );
        assert_eq!(s["trigger"], "alt");
        assert_eq!(s["emoteTrigger"], "off");
        assert_eq!(s["toggleHotkey"], defaults()["toggleHotkey"]);
    }
    #[test]
    fn config_is_flat_and_cannot_start_itself() {
        let c = helper_config(&defaults(), false);
        assert_eq!(c["enabled"], false);
        assert!(c
            .as_object()
            .unwrap()
            .values()
            .all(|v| !v.is_object() && !v.is_array()));
    }
    #[test]
    fn missing_emotes_and_duplicate_slots_fall_back() {
        let s = normalize(
            &json!({"clickEmoteId":"not-installed", "emoteWheel":vec!["facepalm";8]}),
            &core_emotes(),
            false,
        );
        assert_eq!(s["clickEmoteId"], "facepalm");
        assert_eq!(s["emoteWheel"], defaults()["emoteWheel"]);
    }
}
