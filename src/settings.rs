//! App settings (`~/.config/modev/settings.json`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::device::config_file;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum AppTheme {
    /// Follows the desktop.
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: AppTheme,
    /// Folder for screenshots and recordings.
    pub capture_dir: String,
    pub remember_device: bool,
    /// Draw touches/clicks in recordings.
    pub record_touches: bool,
    pub last_device: Option<String>,
    /// Check for updates on launch (pending GitHub releases).
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: AppTheme::Auto,
            capture_dir: default_capture_dir().to_string_lossy().into_owned(),
            remember_device: true,
            record_touches: true,
            last_device: None,
            check_updates: true,
        }
    }
}

/// `~/Imágenes/modev` or `~/Pictures/modev` (whichever exists; otherwise the latter).
fn default_capture_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let pictures = ["Imágenes", "Pictures"]
        .into_iter()
        .map(|d| home.join(d))
        .find(|p| p.is_dir())
        .unwrap_or_else(|| home.join("Pictures"));
    pictures.join("modev")
}

impl Settings {
    pub fn load() -> Self {
        config_file("settings.json")
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = config_file("settings.json") else {
            return;
        };
        let res = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| {
                std::fs::write(&path, serde_json::to_vec_pretty(self).unwrap_or_default())
            });
        if let Err(e) = res {
            eprintln!("[modev] could not save settings.json: {e}");
        }
    }

    /// New path for a screenshot/recording: `modev-YYYYMMDD-HHMMSS.<ext>`.
    pub fn capture_path(&self, ext: &str) -> PathBuf {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        PathBuf::from(&self.capture_dir).join(format!("modev-{stamp}.{ext}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings {
            theme: AppTheme::Light,
            capture_dir: "/tmp/caps".into(),
            remember_device: false,
            record_touches: false,
            last_device: Some("Pixel 9".into()),
            check_updates: false,
        };
        let back: Settings = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(back.theme, AppTheme::Light);
        assert_eq!(back.capture_dir, "/tmp/caps");
        assert!(!back.remember_device && !back.record_touches && !back.check_updates);
        assert_eq!(back.last_device.as_deref(), Some("Pixel 9"));
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"theme":"Dark"}"#).unwrap();
        let d = Settings::default();
        assert_eq!(s.theme, AppTheme::Dark);
        assert_eq!(s.capture_dir, d.capture_dir);
        assert!(s.remember_device && s.record_touches && s.check_updates);
        assert_eq!(s.last_device, None);

        let empty: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.theme, AppTheme::Auto);
    }

    #[test]
    fn capture_path_uses_dir_and_extension() {
        let s = Settings {
            capture_dir: "/tmp/caps".into(),
            ..Settings::default()
        };
        let p = s.capture_path("png");
        assert_eq!(p.parent(), Some(std::path::Path::new("/tmp/caps")));
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("modev-") && name.ends_with(".png"));
        assert_eq!(name.len(), "modev-YYYYMMDD-HHMMSS.png".len());
    }
}
