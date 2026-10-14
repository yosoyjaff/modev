use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Name of the free-size profile (responsive mode).
pub const RESPONSIVE: &str = "Responsive";

// WKWebView (Capacitor) UA: no "Version/x Safari".
const IOS_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
const ANDROID_UA: &str = "Mozilla/5.0 (Linux; Android 15; Pixel 9 Build/AP3A.241005.015; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/153.0.0.0 Mobile Safari/537.36";

/// Profile of a real device. All measures in CSS points (logical px)
/// unless noted otherwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub name: String,
    pub os_label: String,
    /// CSS viewport in portrait.
    pub width: u32,
    pub height: u32,
    /// devicePixelRatio. Physical pixels = width * dpr.
    pub dpr: f64,
    /// Physical panel density (for "physical size" mode). 0 = unknown.
    #[serde(default)]
    pub ppi: f64,
    pub safe_top: u32,
    pub safe_bottom: u32,
    pub user_agent: String,
    pub platform: String,
}

impl DeviceProfile {
    pub fn is_ios(&self) -> bool {
        matches!(self.platform.as_str(), "iPhone" | "iPad")
    }

    /// Changes OS, UA and platform (Ionic picks ios/md mode from the UA).
    pub fn set_ios(&mut self, ios: bool) {
        let (os, ua, platform) = if ios {
            ("iOS 18.0", IOS_UA, "iPhone")
        } else {
            ("Android 15", ANDROID_UA, "Android")
        };
        self.os_label = os.into();
        self.user_agent = ua.into();
        self.platform = platform.into();
    }

    /// Free-size profile based on another one (inherits UA, DPR and safe areas).
    pub fn responsive_from(base: &DeviceProfile) -> Self {
        Self {
            name: RESPONSIVE.into(),
            ppi: 0.0,
            ..base.clone()
        }
    }
}

pub fn presets() -> Vec<DeviceProfile> {
    FIREFOX
        .iter()
        .map(
            |&(name, width, height, dpr, ppi, safe_top, safe_bottom, ua)| {
                let (os_label, platform) = os_from_ua(ua);
                DeviceProfile {
                    name: name.into(),
                    os_label,
                    width,
                    height,
                    dpr,
                    ppi,
                    safe_top,
                    safe_bottom,
                    user_agent: ua.into(),
                    platform: platform.into(),
                }
            },
        )
        .collect()
}

/// OS label and `navigator.platform` derived from the UA.
fn os_from_ua(ua: &str) -> (String, &'static str) {
    let after = |key: &str| {
        ua.split(key)
            .nth(1)
            .and_then(|r| r.split([' ', ';', ')']).next())
    };
    if let Some(v) = after("OS ").filter(|_| ua.contains("like Mac OS X")) {
        let platform = if ua.contains("iPad") {
            "iPad"
        } else {
            "iPhone"
        };
        return (format!("iOS {}", v.replace('_', ".")), platform);
    }
    (
        format!("Android {}", after("Android ").unwrap_or("")),
        "Android",
    )
}

/// `$XDG_CONFIG_HOME/modev/<name>` (or `~/.config/modev/<name>`).
pub fn config_file(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("modev").join(name))
}

fn custom_path() -> Option<PathBuf> {
    config_file("devices.json")
}

/// User-created devices. A missing or invalid file means none.
pub fn load_custom() -> Vec<DeviceProfile> {
    custom_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn hidden_path() -> Option<PathBuf> {
    config_file("hidden-devices.json")
}

/// Names of the presets the user deleted.
pub fn load_hidden() -> Vec<String> {
    hidden_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_hidden(names: &[String]) -> std::io::Result<()> {
    write_config(hidden_path(), &serde_json::to_vec_pretty(names)?)
}

pub fn save_custom(devices: &[DeviceProfile]) -> std::io::Result<()> {
    write_config(custom_path(), &serde_json::to_vec_pretty(devices)?)
}

fn write_config(path: Option<PathBuf>, bytes: &[u8]) -> std::io::Result<()> {
    let path = path.ok_or_else(|| std::io::Error::other("no HOME"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes)
}

// Generated from Firefox's `devtools-devices` collection (phones and tablets).
// ppi and safe areas don't come from Firefox: approximated per model.
const UA0: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1";
const UA1: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 14_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/14.0.3 Mobile/15E148 Safari/604.1";
const UA2: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 12_0 like Mac OS X) AppleWebKit/604.1.38 (KHTML, like Gecko) Version/12.0 Mobile/15A372 Safari/604.1";
const UA3: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Mobile Safari/537.36";
const UA4: &str = "Mozilla/5.0 (Android 14; Mobile; rv:148.0) Gecko/148.0 Firefox/148.0";
const UA5: &str = "Mozilla/5.0 (Linux; Android 11; Pixel 5) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Mobile Safari/537.36";
const UA6: &str = "Mozilla/5.0 (Linux; Android 14; SM-S931B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Mobile Safari/537.36";
const UA7: &str = "Mozilla/5.0 (Linux; Android 14; SM-S936B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Mobile Safari/537.36";
const UA8: &str = "Mozilla/5.0 (Linux; Android 14; SM-S938B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Mobile Safari/537.36";
const UA9: &str = "Mozilla/5.0 (Linux; Android 11; SAMSUNG SM-G973U) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/14.2 Chrome/146.0.0.0 Mobile Safari/537.36";
const UA10: &str = "Mozilla/5.0 (Linux; Android 7.0; SM-G892A Build/NRD90M; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/109.0.0.0 Mobile Safari/537.36";
const UA11: &str = "Mozilla/5.0 (iPad; CPU OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1";
const UA12: &str = "Mozilla/5.0 (iPad; CPU OS 14_7_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/14.1.2 Mobile/15E148 Safari/604.1";
const UA13: &str = "Mozilla/5.0 (Linux; Android 14; SM-X710) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36";
const UA14: &str = "Mozilla/5.0 (Linux; Android 14; SM-X810) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36";
const UA15: &str = "Mozilla/5.0 (Linux; Android 14; SM-X910) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36";

/// (name, width, height, dpr, ppi, safe_top, safe_bottom, UA)
type Preset = (&'static str, u32, u32, f64, f64, u32, u32, &'static str);

const FIREFOX: &[Preset] = &[
    ("iPhone 17 / 17 Pro", 402, 874, 3.0, 460.0, 62, 34, UA0),
    ("iPhone 17 Pro Max", 440, 956, 3.0, 460.0, 62, 34, UA0),
    ("iPhone Air", 420, 921, 3.0, 460.0, 68, 34, UA0),
    ("iPhone 16 Pro Max", 440, 956, 3.0, 460.0, 62, 34, UA0),
    ("iPhone 15 / 16 Pro", 402, 874, 3.0, 460.0, 62, 34, UA0),
    ("iPhone 14 / 15 / 16", 393, 852, 3.0, 460.0, 59, 34, UA0),
    (
        "iPhone 14 / 15 / 16 Plus",
        430,
        932,
        3.0,
        460.0,
        59,
        34,
        UA0,
    ),
    ("iPhone 12/13 + Pro", 390, 844, 3.0, 460.0, 47, 34, UA1),
    ("iPhone 12/13 Pro Max", 428, 926, 3.0, 458.0, 47, 34, UA1),
    ("iPhone 12/13 mini", 375, 812, 3.0, 476.0, 50, 34, UA1),
    ("iPhone 11 Pro", 375, 812, 3.0, 458.0, 44, 34, UA1),
    ("iPhone 11 Pro Max", 414, 896, 3.0, 458.0, 44, 34, UA1),
    ("iPhone XR/11", 414, 896, 2.0, 326.0, 48, 34, UA2),
    ("iPhone XS Max", 414, 896, 3.0, 458.0, 44, 34, UA2),
    ("iPhone X/XS", 375, 812, 3.0, 458.0, 44, 34, UA2),
    ("iPhone SE", 375, 667, 2.0, 326.0, 20, 0, UA1),
    ("Pixel 8 / 9 (Chrome)", 412, 915, 2.625, 422.0, 52, 24, UA3),
    ("Pixel 8 / 9 (Firefox)", 412, 915, 2.625, 422.0, 52, 24, UA4),
    ("Pixel 5", 393, 851, 2.75, 432.0, 24, 0, UA5),
    ("Galaxy S25", 360, 780, 3.0, 416.0, 24, 0, UA6),
    ("Galaxy S25+", 384, 854, 3.0, 513.0, 24, 0, UA7),
    ("Galaxy S25 Ultra", 412, 915, 3.0, 498.0, 24, 0, UA8),
    ("Galaxy S20", 360, 800, 4.0, 563.0, 24, 0, UA9),
    ("Galaxy S20+", 384, 854, 3.75, 525.0, 24, 0, UA9),
    ("Galaxy S10/S10+", 360, 760, 4.0, 550.0, 24, 0, UA9),
    ("Galaxy S9/S9+", 360, 740, 4.0, 570.0, 24, 0, UA10),
    ("Galaxy Note 9", 414, 846, 3.5, 516.0, 24, 0, UA10),
    (
        "iPad Pro 13-inch (M4)",
        1032,
        1376,
        2.0,
        264.0,
        24,
        20,
        UA11,
    ),
    ("iPad Pro 11-inch (M4)", 834, 1210, 2.0, 264.0, 24, 20, UA11),
    (
        "iPad Pro 12.9-inch (old)",
        1024,
        1366,
        2.0,
        264.0,
        24,
        20,
        UA12,
    ),
    (
        "iPad Pro 11-inch (old)",
        834,
        1194,
        2.0,
        264.0,
        24,
        20,
        UA12,
    ),
    ("iPad Air", 820, 1180, 2.0, 264.0, 24, 20, UA12),
    (
        "iPad (10th / 11th gen)",
        820,
        1180,
        2.0,
        264.0,
        24,
        20,
        UA11,
    ),
    ("iPad Mini (6th gen)", 744, 1133, 2.0, 326.0, 24, 20, UA11),
    ("iPad Mini", 768, 1024, 2.0, 326.0, 20, 0, UA12),
    ("iPad", 810, 1080, 2.0, 264.0, 20, 0, UA12),
    ("Galaxy Tab S9", 712, 1138, 2.5, 274.0, 24, 0, UA13),
    ("Galaxy Tab S9+", 800, 1280, 2.5, 266.0, 24, 0, UA14),
    ("Galaxy Tab S9 Ultra", 960, 1848, 2.5, 239.0, 24, 0, UA15),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_sane() {
        let p = presets();
        assert!(!p.is_empty());
        for d in &p {
            assert!(!d.name.is_empty());
            assert!((200..=2000).contains(&d.width), "{}", d.name);
            assert!((400..=2000).contains(&d.height), "{}", d.name);
            assert!((1.0..=4.0).contains(&d.dpr), "{}", d.name);
            assert!(d.ppi > 0.0, "{}", d.name);
            assert!(d.safe_top < d.height / 4 && d.safe_bottom < d.height / 4);
            assert!(d.user_agent.starts_with("Mozilla/5.0"));
            assert_eq!(d.is_ios(), d.os_label.starts_with("iOS"), "{}", d.name);
        }
        assert!(p.iter().any(DeviceProfile::is_ios));
        assert!(p.iter().any(|d| !d.is_ios()));
    }

    #[test]
    fn preset_names_are_unique() {
        let p = presets();
        let mut names: Vec<_> = p.iter().map(|d| d.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), p.len());
    }

    #[test]
    fn os_from_ua_detects_platform() {
        assert_eq!(os_from_ua(UA0).1, "iPhone");
        assert_eq!(os_from_ua(UA12), ("iOS 14.7.1".to_string(), "iPad"));
        assert_eq!(os_from_ua(UA10), ("Android 7.0".to_string(), "Android"));
        assert_eq!(os_from_ua(UA13), ("Android 14".to_string(), "Android"));
        assert_eq!(os_from_ua(IOS_UA), ("iOS 18.0".to_string(), "iPhone"));
    }

    #[test]
    fn set_ios_switches_ua_and_platform() {
        let mut d = presets().into_iter().find(|d| !d.is_ios()).unwrap();
        d.set_ios(true);
        assert!(d.is_ios());
        assert_eq!(os_from_ua(&d.user_agent).1, "iPhone");
        d.set_ios(false);
        assert!(!d.is_ios());
        assert_eq!(d.user_agent, ANDROID_UA);
    }

    #[test]
    fn responsive_inherits_all_but_name_and_ppi() {
        let base = presets().remove(0);
        let r = DeviceProfile::responsive_from(&base);
        assert_eq!(r.name, RESPONSIVE);
        assert_eq!(r.ppi, 0.0);
        assert_eq!(
            DeviceProfile {
                name: base.name.clone(),
                ppi: base.ppi,
                ..r
            },
            base
        );
    }

    #[test]
    fn profile_without_ppi_deserializes() {
        let json = r#"{"name":"X","os_label":"Android 15","width":360,"height":800,
            "dpr":2.0,"safe_top":24,"safe_bottom":0,"user_agent":"ua","platform":"Android"}"#;
        let d: DeviceProfile = serde_json::from_str(json).unwrap();
        assert_eq!(d.ppi, 0.0);
        assert_eq!(d.width, 360);
    }
}
