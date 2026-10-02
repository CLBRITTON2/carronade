//! The picker's shell icons, scaled once and kept in `%LOCALAPPDATA%\carronade\icons.bin`, since the shell takes tens
//! of ms per icon and the cache a fraction of one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP, HPALETTE};
use windows::Win32::Graphics::Imaging::{
    IWICImagingFactory, WICBitmapInterpolationModeHighQualityCubic, WICBitmapUsePremultipliedAlpha,
};

use crate::apps;
use crate::error::{Error, win32};
use crate::store;

/// A `side` px square icon as rows of 32-bit premultiplied BGRA.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    pub side: u32,
    pub bgra: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cached {
    /// What `apps::display_icon` was asked for.
    pub target: String,
    /// When the shell gave it, in seconds since the Unix epoch.
    pub fetched: u64,
    pub pixels: Pixels,
}

/// The most icons `merged` keeps, about 4 MB at 32 px.
pub const LIMIT: usize = 1000;

/// How long a cached icon stands in for the shell's, so a changed icon shows within a week.
pub const MAX_AGE: u64 = 7 * 24 * 60 * 60;

/// Changes whenever the layout below does.
const MAGIC: &[u8; 8] = b"carric01";

/// `%LOCALAPPDATA%\carronade\icons.bin`.
pub fn path() -> Result<PathBuf, Error> {
    store::path("icons.bin")
}

/// The icons `save` wrote to `path`, most recently fetched first. Empty before the first save.
pub fn load(path: &Path) -> Result<Vec<Cached>, Error> {
    let Some(bytes) = store::read(path)? else {
        return Ok(Vec::new());
    };
    decode(&bytes).ok_or_else(|| Error::IconCache {
        path: path.to_owned(),
    })
}

pub fn save(path: &Path, icons: &[Cached]) -> Result<(), Error> {
    store::write(path, &encode(icons))
}

/// The cached icon for `target` at `side` px, unless it is older than `MAX_AGE` at `now`.
pub fn find<'a>(icons: &'a [Cached], target: &str, side: u32, now: u64) -> Option<&'a Pixels> {
    icons
        .iter()
        .find(|icon| icon.target == target)
        .filter(|icon| icon.pixels.side == side && now.saturating_sub(icon.fetched) < MAX_AGE)
        .map(|icon| &icon.pixels)
}

/// `fetched` ahead of `previous`, one icon per target, at most `LIMIT`.
pub fn merged(fetched: Vec<Cached>, previous: Vec<Cached>) -> Vec<Cached> {
    let mut seen = HashSet::new();
    fetched
        .into_iter()
        .chain(previous)
        .filter(|icon| seen.insert(icon.target.clone()))
        .take(LIMIT)
        .collect()
}

/// The shell's icon for `target`, scaled to `side` px.
pub fn fetch(wic: &IWICImagingFactory, target: &str, side: u32) -> Result<Pixels, Error> {
    let icon = apps::display_icon(target, side as i32)?;
    let pixels = scaled(wic, icon, side);
    unsafe { DeleteObject(icon.into()) }
        .ok()
        .map_err(win32("DeleteObject"))?;
    pixels
}

fn scaled(wic: &IWICImagingFactory, icon: HBITMAP, side: u32) -> Result<Pixels, Error> {
    let bitmap = unsafe {
        wic.CreateBitmapFromHBITMAP(icon, HPALETTE::default(), WICBitmapUsePremultipliedAlpha)
    }
    .map_err(win32("CreateBitmapFromHBITMAP"))?;
    let scaler = unsafe { wic.CreateBitmapScaler() }.map_err(win32("CreateBitmapScaler"))?;
    unsafe {
        scaler.Initialize(
            &bitmap,
            side,
            side,
            WICBitmapInterpolationModeHighQualityCubic,
        )
    }
    .map_err(win32("IWICBitmapScaler::Initialize"))?;
    let mut bgra = vec![0; side as usize * side as usize * 4];
    unsafe { scaler.CopyPixels(std::ptr::null(), side * 4, &mut bgra) }
        .map_err(win32("IWICBitmapScaler::CopyPixels"))?;
    Ok(Pixels { side, bgra })
}

/// `MAGIC`, the count, then per icon: the target's length and UTF-8, `fetched`, `side`, and the pixels. Little-endian.
pub(crate) fn encode(icons: &[Cached]) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend((icons.len() as u32).to_le_bytes());
    for icon in icons {
        bytes.extend((icon.target.len() as u32).to_le_bytes());
        bytes.extend(icon.target.as_bytes());
        bytes.extend(icon.fetched.to_le_bytes());
        bytes.extend(icon.pixels.side.to_le_bytes());
        bytes.extend(&icon.pixels.bgra);
    }
    bytes
}

/// What `encode` wrote, or `None` for anything else.
pub(crate) fn decode(bytes: &[u8]) -> Option<Vec<Cached>> {
    let (count, mut rest) = u32_at(bytes.strip_prefix(MAGIC)?)?;
    let mut icons = Vec::new();
    for _ in 0..count {
        let (length, after) = u32_at(rest)?;
        let (target, after) = after.split_at_checked(length as usize)?;
        let (fetched, after) = after.split_first_chunk::<8>()?;
        let (side, after) = u32_at(after)?;
        let size = (side as usize).checked_mul(side as usize)?.checked_mul(4)?;
        let (bgra, after) = after.split_at_checked(size)?;
        icons.push(Cached {
            target: String::from_utf8(target.to_vec()).ok()?,
            fetched: u64::from_le_bytes(*fetched),
            pixels: Pixels {
                side,
                bgra: bgra.to_vec(),
            },
        });
        rest = after;
    }
    rest.is_empty().then_some(icons)
}

fn u32_at(bytes: &[u8]) -> Option<(u32, &[u8])> {
    let (head, rest) = bytes.split_first_chunk::<4>()?;
    Some((u32::from_le_bytes(*head), rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(target: &str, fetched: u64, side: u32) -> Cached {
        Cached {
            target: target.to_owned(),
            fetched,
            pixels: Pixels {
                side,
                bgra: vec![7; side as usize * side as usize * 4],
            },
        }
    }

    fn targets(icons: &[Cached]) -> Vec<&str> {
        icons.iter().map(|icon| icon.target.as_str()).collect()
    }

    #[test]
    fn icons_decode_to_what_was_encoded() {
        let icons = vec![icon("a", 1, 2), icon("ü", 3, 0)];
        assert_eq!(decode(&encode(&icons)), Some(icons));
    }

    #[test]
    fn a_truncated_or_padded_cache_does_not_decode() {
        let bytes = encode(&[icon("a", 1, 2)]);
        assert_eq!(bytes.split_last().and_then(|(_, rest)| decode(rest)), None);
        assert_eq!(decode(&[bytes.as_slice(), &[0]].concat()), None);
        assert_eq!(decode(b"carric00\0\0\0\0"), None);
    }

    #[test]
    fn find_skips_icons_of_another_size_or_too_old() {
        let icons = [icon("a", 100, 2)];
        assert!(find(&icons, "a", 2, 100 + MAX_AGE - 1).is_some());
        assert!(find(&icons, "a", 2, 100 + MAX_AGE).is_none());
        assert!(find(&icons, "a", 3, 100).is_none());
        assert!(find(&icons, "b", 2, 100).is_none());
    }

    #[test]
    fn fetched_icons_replace_and_lead_the_previous_ones() {
        let merged = merged(
            vec![icon("b", 9, 1), icon("c", 9, 1)],
            vec![icon("a", 1, 1), icon("b", 1, 1)],
        );
        assert_eq!(targets(&merged), ["b", "c", "a"]);
        assert_eq!(merged.first().map(|icon| icon.fetched), Some(9));
    }

    #[test]
    fn merging_keeps_at_most_the_limit() {
        let previous = (0..LIMIT + 5).map(|n| icon(&n.to_string(), 1, 0)).collect();
        assert_eq!(merged(vec![], previous).len(), LIMIT);
    }
}
