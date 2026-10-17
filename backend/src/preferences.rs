use serde_json::{json, Value};
use std::{fs, path::Path};

pub fn load(data: &Path) -> Value {
    let raw: Value = fs::read(data.join("app.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    json!({"theme":if ["system","light","dark"].contains(&raw["theme"].as_str().unwrap_or("")){raw["theme"].clone()}else{json!("dark")},"autostart":raw["autostart"]==true,"onboarded":raw["onboarded"]==true})
}
pub fn update(data: &Path, current: &Value, patch: &Value) -> Result<Value, String> {
    let mut next = current.clone();
    if let Some(theme) = patch.get("theme") {
        if !["system", "light", "dark"].contains(&theme.as_str().unwrap_or("")) {
            return Err("invalidTheme".into());
        }
        next["theme"] = theme.clone();
    }
    let mut autostart_changed = false;
    if let Some(value) = patch.get("autostart") {
        let on = value.as_bool().ok_or("invalidAutostart")?;
        if current["autostart"] != on {
            configure_autostart(on)?;
            autostart_changed = true;
        }
        next["autostart"] = json!(on);
    }
    if let Err(error) = crate::settings::atomic_json(&data.join("app.json"), &next) {
        if autostart_changed {
            let _ = configure_autostart(current["autostart"] == true);
        }
        return Err(error);
    }
    Ok(next)
}
fn configure_autostart(on: bool) -> Result<(), String> {
    let path = std::env::var_os("APPIMAGE")
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_exe().map_err(|e| e.to_string())?);
    let mut builder = auto_launch::AutoLaunchBuilder::new();
    builder
        .set_app_name("LOLPing Lite")
        .set_app_path(&path.to_string_lossy())
        .set_args(&["--background"]);
    #[cfg(target_os = "windows")]
    builder.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
    #[cfg(target_os = "macos")]
    builder.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
    let auto = builder.build().map_err(|e| e.to_string())?;
    if on { auto.enable() } else { auto.disable() }.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_install_defaults_to_dark_without_enabling_autostart() {
        let data = tempfile::tempdir().unwrap();
        assert_eq!(
            load(data.path()),
            json!({"theme": "dark", "autostart": false, "onboarded": false})
        );
        assert!(!data.path().join("app.json").exists());
    }

    #[test]
    fn existing_theme_is_preserved_independently_of_onboarding() {
        let data = tempfile::tempdir().unwrap();
        for theme in ["system", "light", "dark"] {
            for onboarded in [false, true] {
                let saved = json!({"theme": theme, "autostart": true, "onboarded": onboarded});
                let bytes = serde_json::to_vec(&saved).unwrap();
                fs::write(data.path().join("app.json"), &bytes).unwrap();
                assert_eq!(load(data.path()), saved);
                assert_eq!(fs::read(data.path().join("app.json")).unwrap(), bytes);
            }
        }
    }

    #[test]
    fn missing_or_invalid_theme_uses_dark_without_resetting_other_preferences() {
        let data = tempfile::tempdir().unwrap();
        for saved in [
            json!({"autostart": true, "onboarded": true}),
            json!({"theme": null, "autostart": true, "onboarded": true}),
            json!({"theme": "invalid", "autostart": true, "onboarded": true}),
            json!({"theme": 23, "autostart": true, "onboarded": true}),
        ] {
            fs::write(
                data.path().join("app.json"),
                serde_json::to_vec(&saved).unwrap(),
            )
            .unwrap();
            assert_eq!(
                load(data.path()),
                json!({"theme": "dark", "autostart": true, "onboarded": true})
            );
        }
    }

    #[test]
    fn invalid_preference_does_not_replace_saved_settings() {
        let d = tempfile::tempdir().unwrap();
        let initial = load(d.path());
        let v = update(d.path(), &initial, &json!({"theme":"dark"})).unwrap();
        assert_eq!(v["autostart"], false);
        assert!(update(d.path(), &v, &json!({"theme":"bad"})).is_err());
        assert_eq!(load(d.path())["theme"], "dark");
    }
}
