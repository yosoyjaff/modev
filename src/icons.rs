//! Own embedded icons and the `AssetSource` that serves them alongside gpui-kit's.
//!
//! Hugeicons (stroke rounded, free set), MIT — see `assets/icons/LICENSE-hugeicons.md`.
//! Converted to SVG from `@hugeicons/core-free-icons` (the one used by `hugeicons-react`).
//! Used with gpui-kit's `Icon`: `Icon::new(Ico::Close).small()` (inherits the text color).

use std::borrow::Cow;

use gpui_kit::assets::IconNamed;
use gpui_kit::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($(($name:ident, $file:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub enum Ico {
            $($name,)+
        }

        impl IconNamed for Ico {
            fn path(self) -> SharedString {
                match self {
                    $(Ico::$name => concat!("modev/icons/", $file, ".svg"),)+
                }
                .into()
            }
        }

        fn load_icon(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("modev/icons/", $file, ".svg") => {
                    Some(include_bytes!(concat!("../assets/icons/", $file, ".svg")).as_slice())
                })+
                _ => None,
            }
        }

        const ALL: &[&str] = &[$(concat!("modev/icons/", $file, ".svg")),+];
    };
}

icons![
    (Close, "close"),
    (Minimize, "window-minimize"),
    (Refresh, "refresh"),
    (Home, "home"),
    (Smartphone, "smartphone"),
    (Terminal, "terminal"),
    (Globe, "globe"),
    (Info, "info"),
    (Warning, "warning"),
    (Error, "error"),
    (Check, "check"),
    (ChevronLeft, "chevron-left"),
    (ChevronRight, "chevron-right"),
    (ChevronsUpDown, "chevrons-up-down"),
    (Network, "network"),
    (Wifi, "wifi"),
    (WifiOff, "wifi-off"),
    (Clear, "clear"),
    (Copy, "copy"),
    (Download, "download"),
    (WrapText, "wrap-text"),
    (Columns, "columns"),
    (ArrowUp, "arrow-up"),
    (ArrowDown, "arrow-down"),
    (Database, "database"),
    (Cookie, "cookie"),
    (Table, "table"),
    (HardDrive, "hard-drive"),
    (Package, "package"),
    (Delete, "delete"),
    (Key, "key"),
    (Settings, "settings"),
    (Sliders, "sliders"),
    (Camera, "camera"),
    (Timer, "timer"),
    (Sun, "sun"),
    (Moon, "moon"),
    (Record, "record"),
    (Stop, "stop"),
    (Folder, "folder"),
    (Search, "search"),
    (Code, "code"),
    (Android, "android"),
];

/// Own icons + gpui-kit's (its components, like `Input`, keep using theirs).
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = load_icon(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all: Vec<SharedString> = ALL
            .iter()
            .filter(|p| p.starts_with(path))
            .map(|p| SharedString::from(*p))
            .collect();
        all.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(all)
    }
}
