//! The look of the picker and the modes' options, read from a TOML file. Every field is required except `window.image`.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use windows::Win32::UI::Shell::{FOLDERID_Profile, FOLDERID_RoamingAppData};

use crate::error::Error;
use crate::platform::known_folder;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub font: Font,
    pub window: Window,
    pub input: Input,
    pub list: List,
    pub element: Element,
    pub apps: Apps,
    pub files: Files,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Font {
    /// A DirectWrite family name, as the Fonts settings page lists it.
    pub family: String,
    /// One `em` is this size.
    pub size: Points,
}

/// A font size from 4 to 72 points, which bounds an em to a few hundred px on any monitor.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "f32")]
pub struct Points(f32);

impl Points {
    #[must_use]
    pub fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Points {
    type Error = String;

    fn try_from(points: f32) -> Result<Self, String> {
        if (4.0..=72.0).contains(&points) {
            Ok(Points(points))
        } else {
            Err(format!("{points} is not a font size from 4 to 72 points"))
        }
    }
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
    /// Drawn in `prompt_font` and `placeholder_color` at the bar's right end in file search: a click on it, or Tab,
    /// switches to the apps.
    pub apps_icon: String,
    /// The same in apps, switching to file search.
    pub files_icon: String,
}

/// The grid of matches, filled column by column.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub background: Color,
    pub padding: Length,
    pub spacing: Length,
    pub columns: Count,
    pub lines: Count,
}

impl List {
    /// The cells on one page.
    #[must_use]
    pub fn page(&self) -> NonZeroUsize {
        // Both are at most `Count::MAX`, so the product never saturates.
        self.columns.get().saturating_mul(self.lines.get())
    }
}

/// A count of grid columns or lines, from 1 to `Count::MAX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "usize")]
pub struct Count(NonZeroUsize);

impl Count {
    pub const MAX: usize = 64;

    #[must_use]
    pub fn get(self) -> NonZeroUsize {
        self.0
    }
}

impl TryFrom<usize> for Count {
    type Error = String;

    fn try_from(count: usize) -> Result<Self, String> {
        NonZeroUsize::new(count)
            .filter(|count| count.get() <= Count::MAX)
            .map(Count)
            .ok_or(format!("{count} is not a count from 1 to {}", Count::MAX))
    }
}

/// One match in the grid: an icon, when the mode has one, then the label.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    pub padding: Length,
    pub radius: Length,
    pub color: Color,
    pub selected: Color,
    pub highlight: Color,
    pub icon: Side,
    pub gap: Length,
}

/// An icon's side, 1 to 256 px or 0.25 to 4 em, so it rounds to 1 to a few thousand device px.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub struct Side(Length);

impl Side {
    #[must_use]
    pub fn px(self, em: f32, scale: f32) -> f32 {
        self.0.px(em, scale)
    }
}

impl TryFrom<String> for Side {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let length = Length::try_from(text.clone())?;
        let bounded = match length {
            Length::Px(px) => (1.0..=256.0).contains(&px),
            Length::Em(ems) => (0.25..=4.0).contains(&ems),
        };
        if bounded {
            Ok(Side(length))
        } else {
            Err(format!(
                "{text:?} is not an icon side from 1px to 256px or 0.25em to 4em"
            ))
        }
    }
}

/// The Start menu mode.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Apps {
    /// Opens on the apps found last time and lists them again after the picker closes: the shell takes hundreds of
    /// ms to list them.
    pub cache: bool,
}

/// The file search mode.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Files {
    /// The folders searched, with everything below them.
    pub roots: Vec<PathBuf>,
    /// Opens on the entries found last time and lists them again after the picker closes.
    pub cache: bool,
    /// Started as the Run dialog would by Ctrl+Enter, in the selected folder or the folder of the selected file.
    pub terminal: String,
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
    #[must_use]
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
        let [red, green, blue, alpha] = rgba.to_be_bytes().map(|byte| f32::from(byte) / 255.0);
        Ok(Color {
            red,
            green,
            blue,
            alpha,
        })
    }
}

/// The config at `path`, with a leading `~` in `window.image` and `files.roots` meaning the user's profile folder.
pub fn load(path: &Path) -> Result<Config, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::ConfigRead {
        path: path.to_owned(),
        source,
    })?;
    let config: Config = toml::from_str(&text).map_err(|source| Error::ConfigParse {
        path: path.to_owned(),
        source: Box::new(source),
    })?;
    rooted(config, path, &known_folder(&FOLDERID_Profile)?)
}

/// `config`, read from `path`, with its paths under `home` where they start with `~`, each of them absolute.
fn rooted(config: Config, path: &Path, home: &Path) -> Result<Config, Error> {
    // A relative path would resolve against the launcher's working folder.
    let absolute = |field: &'static str, value: &Path| {
        let expanded = under_home(value, home);
        if expanded.is_absolute() {
            Ok(expanded)
        } else {
            Err(Error::RelativePath {
                config: path.to_owned(),
                field,
                path: expanded,
            })
        }
    };
    let image = config
        .window
        .image
        .as_deref()
        .map(|image| absolute("window.image", image))
        .transpose()?;
    let roots = config
        .files
        .roots
        .iter()
        .map(|root| absolute("files.roots", root))
        .collect::<Result<Vec<PathBuf>, Error>>()?;
    Ok(Config {
        window: Window {
            image,
            ..config.window
        },
        files: Files {
            roots,
            ..config.files
        },
        ..config
    })
}

/// `path` with a leading `~` component replaced by `home`.
fn under_home(path: &Path, home: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) if rest.as_os_str().is_empty() => home.to_owned(),
        Ok(rest) => home.join(rest),
        Err(_) => path.to_owned(),
    }
}

/// `%APPDATA%\carronade\config.toml`, the config read when no `--config` is given.
pub fn path() -> Result<PathBuf, Error> {
    Ok(known_folder(&FOLDERID_RoamingAppData)?
        .join("carronade")
        .join("config.toml"))
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
    fn lengths_need_a_unit_and_a_finite_number_not_below_0() {
        assert_eq!(length("0px"), Ok(Length::Px(0.0)));
        for text in ["3", "3pt", "-1px", "em", "NaNpx", "infem"] {
            assert!(length(text).is_err(), "{text:?} parsed");
        }
    }

    #[test]
    fn font_sizes_run_from_4_to_72_points() {
        for points in [4.0, 10.5, 72.0] {
            assert_eq!(Points::try_from(points).map(Points::get), Ok(points));
        }
        for points in [0.0, -1.0, 3.9, 72.5, f32::NAN, f32::INFINITY] {
            assert!(Points::try_from(points).is_err(), "{points} parsed");
        }
    }

    #[test]
    fn counts_run_from_1_to_the_max() {
        for count in [1, Count::MAX] {
            assert_eq!(Count::try_from(count).map(|c| c.get().get()), Ok(count));
        }
        for count in [0, Count::MAX + 1, usize::MAX] {
            assert!(Count::try_from(count).is_err(), "{count} parsed");
        }
    }

    #[test]
    fn icon_sides_are_bounded_in_px_and_em() {
        for text in ["1px", "256px", "0.25em", "4em"] {
            assert!(Side::try_from(text.to_owned()).is_ok(), "{text:?} failed");
        }
        for text in ["0px", "0.5px", "257px", "0em", "0.2em", "4.5em", "2"] {
            assert!(Side::try_from(text.to_owned()).is_err(), "{text:?} parsed");
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
    fn a_leading_tilde_is_the_home_folder() {
        let home = Path::new("C:\\Users\\you");
        let expanded = ["~", "~\\dev", "~/dev/x", "~dev", "C:\\~\\dev", "dev"]
            .map(|path| under_home(Path::new(path), home));
        assert_eq!(
            expanded.each_ref().map(|path| path.to_str()),
            [
                Some("C:\\Users\\you"),
                Some("C:\\Users\\you\\dev"),
                Some("C:\\Users\\you\\dev/x"),
                Some("~dev"),
                Some("C:\\~\\dev"),
                Some("dev"),
            ]
        );
    }

    fn rooted_roots(roots: &str) -> Result<Config, Error> {
        let text = include_str!("../config.toml").replace("roots = ['~\\dev']", roots);
        let config: Config = toml::from_str(&text).map_err(|source| Error::ConfigParse {
            path: PathBuf::from("test.toml"),
            source: Box::new(source),
        })?;
        rooted(config, Path::new("test.toml"), Path::new("C:\\Users\\you"))
    }

    #[test]
    fn absolute_and_home_paths_are_kept() -> Result<(), Error> {
        let config = rooted_roots("roots = ['~\\dev', 'D:\\work']")?;
        assert_eq!(
            config.files.roots,
            [
                PathBuf::from("C:\\Users\\you\\dev"),
                PathBuf::from("D:\\work")
            ]
        );
        Ok(())
    }

    #[test]
    fn a_relative_path_is_rejected() {
        for root in ["dev", "\\dev", "D:dev"] {
            let error = rooted_roots(&format!("roots = ['{root}']"));
            assert!(
                matches!(
                    &error,
                    Err(Error::RelativePath { field: "files.roots", path, .. }) if path == Path::new(root)
                ),
                "{root:?} gave {error:?}"
            );
        }
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let text = include_str!("../config.toml").replace("[list]", "[list]\ncycle = true");
        let error = toml::from_str::<Config>(&text)
            .err()
            .map(|error| error.to_string());
        assert!(
            error
                .as_ref()
                .is_some_and(|error| error.contains("unknown field `cycle`")),
            "got {error:?}"
        );
    }
}
