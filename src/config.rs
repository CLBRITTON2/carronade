//! The look of the picker and the modes' options, read from a TOML file. Every field is required except `window.image`.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::GUID;

use crate::error::{Error, win32};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub font: Font,
    pub window: Window,
    pub input: Input,
    pub list: List,
    pub element: Element,
    pub drun: Drun,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Font {
    /// A DirectWrite family name, as the Fonts settings page lists it.
    pub family: String,
    /// In points. One `em` is this size.
    pub size: f32,
}

/// The popup's frame. Its height follows from the input bar and the list.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub width: Length,
    pub background: Color,
    pub border: Length,
    pub border_color: Color,
    pub radius: Length,
    /// Drawn behind the input bar and the list, scaled to cover the window.
    pub image: Option<PathBuf>,
}

/// The bar with the prompt and the typed text.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Space between the bar and the window's edges and the list.
    pub margin: Length,
    pub padding: Length,
    pub radius: Length,
    pub background: Color,
    pub color: Color,
    pub prompt: String,
    pub prompt_font: String,
    pub prompt_gap: Length,
    pub placeholder: String,
    pub placeholder_color: Color,
}

/// The grid of matches, filled column by column.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub background: Color,
    pub padding: Length,
    pub spacing: Length,
    pub columns: NonZeroUsize,
    pub lines: NonZeroUsize,
}

/// One match in the grid: an icon, when the mode has one, then the label.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    pub padding: Length,
    pub radius: Length,
    pub color: Color,
    pub selected: Color,
    pub icon: Length,
    pub gap: Length,
}

/// The Start menu mode.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drun {
    /// Opens on the apps found last time and lists them again after the picker closes: the shell takes hundreds of
    /// ms to list them.
    pub cache: bool,
}

/// `"8px"`, scaled with the monitor's DPI, or `"1.5em"`, a multiple of the font size.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub enum Length {
    Px(f32),
    Em(f32),
}

impl Length {
    /// Device pixels, given the font size in device pixels and the monitor's scale (1.0 at 96 DPI).
    pub fn px(self, em: f32, scale: f32) -> f32 {
        match self {
            Length::Px(px) => px * scale,
            Length::Em(ems) => ems * em,
        }
    }
}

impl TryFrom<String> for Length {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let number = |digits: &str| {
            digits
                .parse::<f32>()
                .ok()
                .filter(|n| n.is_finite() && *n >= 0.0)
        };
        let length = match (text.strip_suffix("px"), text.strip_suffix("em")) {
            (Some(digits), _) => number(digits).map(Length::Px),
            (_, Some(digits)) => number(digits).map(Length::Em),
            _ => None,
        };
        length.ok_or(format!(
            "{text:?} is not a length like \"8px\" or \"1.5em\""
        ))
    }
}

/// `"#rrggbb"` or `"#rrggbbaa"`, each channel from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl TryFrom<String> for Color {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let invalid = || format!("{text:?} is not a color like \"#1c1c22\" or \"#1c1c22eb\"");
        let hex = text
            .strip_prefix('#')
            .filter(|hex| {
                matches!(hex.len(), 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .ok_or_else(invalid)?;
        let value = u32::from_str_radix(hex, 16).map_err(|_| invalid())?;
        let rgba = match hex.len() {
            6 => value << 8 | 0xff,
            _ => value,
        };
        let channel = |shift: u32| ((rgba >> shift) & 0xff) as f32 / 255.0;
        Ok(Color {
            red: channel(24),
            green: channel(16),
            blue: channel(8),
            alpha: channel(0),
        })
    }
}

pub fn load(path: &Path) -> Result<Config, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::ConfigRead {
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| Error::ConfigParse {
        path: path.to_owned(),
        source: Box::new(source),
    })
}

/// `%APPDATA%\carronade\config.toml`, the config read when no `--config` is given.
pub fn path() -> Result<PathBuf, Error> {
    Ok(known_folder(&FOLDERID_RoamingAppData)?
        .join("carronade")
        .join("config.toml"))
}

pub(crate) fn known_folder(id: &GUID) -> Result<PathBuf, Error> {
    let folder = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }
        .map_err(win32("SHGetKnownFolderPath"))?;
    let text = unsafe { folder.to_string() };
    unsafe { CoTaskMemFree(Some(folder.0 as _)) };
    Ok(PathBuf::from(text?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn length(text: &str) -> Result<Length, String> {
        Length::try_from(text.to_owned())
    }

    fn color(text: &str) -> Result<Color, String> {
        Color::try_from(text.to_owned())
    }

    #[test]
    fn lengths_take_px_or_em() {
        assert_eq!(length("3px"), Ok(Length::Px(3.0)));
        assert_eq!(length("1.5em"), Ok(Length::Em(1.5)));
        assert_eq!(length("1.5em").map(|l| l.px(10.0, 2.0)), Ok(15.0));
        assert_eq!(length("3px").map(|l| l.px(10.0, 2.0)), Ok(6.0));
    }

    #[test]
    fn lengths_need_a_unit_and_a_positive_number() {
        for text in ["3", "3pt", "-1px", "em", "NaNpx", "infem"] {
            assert!(length(text).is_err(), "{text:?} parsed");
        }
    }

    #[test]
    fn colors_take_an_optional_alpha() {
        assert_eq!(
            color("#ff0033"),
            Ok(Color {
                red: 1.0,
                green: 0.0,
                blue: 0.2,
                alpha: 1.0
            })
        );
        assert_eq!(color("#00000000").map(|c| c.alpha), Ok(0.0));
    }

    #[test]
    fn colors_need_six_or_eight_hex_digits() {
        for text in ["ff0033", "#f03", "#ff00331", "#+f0033", "#gg0033"] {
            assert!(color(text).is_err(), "{text:?} parsed");
        }
    }

    #[test]
    fn the_shipped_config_parses() -> Result<(), toml::de::Error> {
        let config: Config = toml::from_str(include_str!("../config.toml"))?;
        assert_eq!(config.window.image, None);
        Ok(())
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let text = include_str!("../config.toml").replace("[list]", "[list]\ncycle = true");
        let error = toml::from_str::<Config>(&text)
            .err()
            .map(|error| error.to_string());
        assert!(error.is_some_and(|error| error.contains("cycle")));
    }
}
