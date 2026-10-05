//! The picker's shell icons, fetched from the shell, scaled once and kept in `%LOCALAPPDATA%\carronade\icons.bin`,
//! since the shell takes tens of ms per icon and the cache a fraction of one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HBITMAP, HPALETTE};
use windows::Win32::Graphics::Imaging::{
    IWICImagingFactory, WICBitmapInterpolationModeHighQualityCubic, WICBitmapUsePremultipliedAlpha,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
};
use windows::core::HSTRING;

use crate::error::{self, Error};
use crate::platform::com;
use crate::store::{self, UnixSeconds};

/// A `side` px square icon as rows of 32-bit premultiplied BGRA. `bgra` always holds `side * side * 4` bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    side: u32,
    bgra: Vec<u8>,
}

impl Pixels {
    /// `bgra` as a `side` px square, or `None` when its length is not `side * side * 4`.
    #[must_use]
    pub fn new(side: u32, bgra: Vec<u8>) -> Option<Self> {
        (bgra.len() == byte_count(side)?).then_some(Self { side, bgra })
    }

    #[must_use]
    pub fn side(&self) -> u32 {
        self.side
    }

    #[must_use]
    pub fn bgra(&self) -> &[u8] {
        &self.bgra
    }
}

/// The bytes of a `side` px square of 32-bit pixels, or `None` past `usize`.
fn byte_count(side: u32) -> Option<usize> {
    let edge = usize::try_from(side).ok()?;
    edge.checked_mul(edge)?.checked_mul(4)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cached {
    /// What `display_icon` was asked for.
    pub target: String,
    /// When the shell gave it.
    pub fetched: UnixSeconds,
    pub pixels: Pixels,
}

/// The most icons `merged` keeps, about 4 MB at 32 px.
pub const LIMIT: usize = 1000;

/// How long a cached icon stands in for the shell's, so a changed icon shows within a week.
pub const MAX_AGE_SECONDS: u64 = 7 * 24 * 60 * 60;

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

/// The cached icon for `target` at `side` px, unless it is older than `MAX_AGE_SECONDS` at `now`.
#[must_use]
pub fn find<'a>(
    icons: &'a [Cached],
    target: &str,
    side: u32,
    now: UnixSeconds,
) -> Option<&'a Pixels> {
    icons
        .iter()
        .find(|icon| icon.target == target)
        .filter(|icon| icon.pixels.side() == side && now.since(icon.fetched) < MAX_AGE_SECONDS)
        .map(|icon| &icon.pixels)
}

/// `fetched` ahead of `previous`, one icon per target, at most `LIMIT`.
#[must_use]
pub fn merged(fetched: Vec<Cached>, previous: Vec<Cached>) -> Vec<Cached> {
    let mut seen = HashSet::new();
    fetched
        .into_iter()
        .chain(previous)
        .filter(|icon| seen.insert(icon.target.clone()))
        .take(LIMIT)
        .collect()
}

/// The shell's icon for `target`, scaled to `side` px. Every error names `target`.
pub fn fetch(wic: &IWICImagingFactory, target: &str, side: u32) -> Result<Pixels, Error> {
    // An icon side is tens of px.
    let icon = display_icon(target, side as i32)?;
    let pixels = scaled(wic, target, icon, side);
    // SAFETY: `icon` came from `display_icon`, this function owns it, and `scaled` copied its pixels already.
    let deleted = unsafe { DeleteObject(icon.into()) }
        .ok()
        .map_err(error::icon(target, "DeleteObject"));
    // The scaling error is the cause, so it wins over a failed delete.
    let pixels = pixels?;
    deleted?;
    Ok(pixels)
}

fn scaled(
    wic: &IWICImagingFactory,
    target: &str,
    icon: HBITMAP,
    side: u32,
) -> Result<Pixels, Error> {
    // SAFETY: `icon` is a live bitmap the caller owns until this returns, and a 32-bit one needs no palette.
    let bitmap = unsafe {
        wic.CreateBitmapFromHBITMAP(icon, HPALETTE::default(), WICBitmapUsePremultipliedAlpha)
    }
    .map_err(error::icon(target, "CreateBitmapFromHBITMAP"))?;
    // SAFETY: `wic` is a live factory on this thread.
    let scaler =
        unsafe { wic.CreateBitmapScaler() }.map_err(error::icon(target, "CreateBitmapScaler"))?;
    // SAFETY: `bitmap` and `scaler` are live WIC objects from the same factory.
    unsafe {
        scaler.Initialize(
            &bitmap,
            side,
            side,
            WICBitmapInterpolationModeHighQualityCubic,
        )
    }
    .map_err(error::icon(target, "IWICBitmapScaler::Initialize"))?;
    // u32 to usize widens on the only target, 64-bit Windows.
    let mut bgra = vec![0; side as usize * side as usize * 4];
    // SAFETY: a null rect copies the whole `side` square, which fills `bgra` exactly at a stride of `side * 4`.
    unsafe { scaler.CopyPixels(std::ptr::null(), side * 4, &mut bgra) }
        .map_err(error::icon(target, "IWICBitmapScaler::CopyPixels"))?;
    Ok(Pixels { side, bgra })
}

/// The icon the shell shows for `target`, at most `size` px square, as a 32-bit bitmap with premultiplied alpha.
pub fn icon(target: &str, size: i32) -> Result<HBITMAP, Error> {
    com()?;
    // SAFETY: COM is initialized on this thread and the name is a temporary HSTRING that outlives the call.
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(target), None) }
            .map_err(error::icon(target, "SHCreateItemFromParsingName"))?;
    // SAFETY: `factory` is a live COM object, and the caller owns the returned bitmap.
    unsafe { factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY) }
        .map_err(error::icon(target, "IShellItemImageFactory::GetImage"))
}

/// The shell's largest icon size. It scales smaller requests up from coarse assets, so fetch this and scale down.
const LARGEST: i32 = 256;
/// The smallest opaque side of a real `LARGEST` icon. A u32 widens to usize on 64-bit Windows.
const HALF_LARGEST: usize = LARGEST.unsigned_abs() as usize / 2;

/// The icon to show for `target` at `size` px. An app with no large image comes back from a `LARGEST` request as its
/// small one, unscaled, in the middle of a translucent frame, so that one is fetched again at `size`.
pub fn display_icon(target: &str, size: i32) -> Result<HBITMAP, Error> {
    let large = icon(target, LARGEST)?;
    let opaque = opaque_side(target, large);
    if let Ok(opaque) = opaque
        && opaque >= HALF_LARGEST
    {
        return Ok(large);
    }
    // SAFETY: `large` came from `icon`, this function owns it, and it is not used after.
    let deleted = unsafe { DeleteObject(large.into()) }
        .ok()
        .map_err(error::icon(target, "DeleteObject"));
    // The bitmap error is the cause, so it wins over a failed delete.
    opaque?;
    deleted?;
    icon(target, size)
}

/// `opaque_extent` of the shell's icon `bitmap` for `target`.
fn opaque_side(target: &str, bitmap: HBITMAP) -> Result<usize, Error> {
    let mut info = BITMAP::default();
    // SAFETY: `info` is a writable BITMAP of exactly the size passed.
    let written = unsafe {
        GetObjectW(
            bitmap.into(),
            // A BITMAP is 32 bytes.
            size_of::<BITMAP>() as i32,
            Some((&raw mut info).cast()),
        )
    };
    if written == 0 || info.bmBits.is_null() || info.bmBitsPixel != 32 {
        return Err(Error::IconBitmap {
            target: target.to_owned(),
        });
    }
    // u32 to usize widens on the only target, 64-bit Windows.
    let stride = info.bmWidthBytes.unsigned_abs() as usize;
    let length = stride * info.bmHeight.unsigned_abs() as usize;
    // SAFETY: a DIB section's bits are non-null (checked above), span stride times height bytes, and live as long as
    // `bitmap`, which the caller holds past this borrow.
    let pixels =
        unsafe { std::slice::from_raw_parts(info.bmBits.cast::<u8>().cast_const(), length) };
    Ok(opaque_extent(pixels, stride))
}

/// The longer side of the box around the pixels at least half opaque, in 32-bit BGRA rows of `stride` bytes.
fn opaque_extent(pixels: &[u8], stride: usize) -> usize {
    if stride == 0 {
        return 0;
    }
    let opaque = pixels
        .chunks_exact(stride)
        .enumerate()
        .flat_map(|(y, row)| {
            row.as_chunks::<4>()
                .0
                .iter()
                .enumerate()
                .filter(|(_, [_, _, _, alpha])| *alpha >= 128)
                .map(move |(x, _)| (x, y))
        });
    let bounds = opaque.fold(None, |bounds, (x, y)| match bounds {
        None => Some((x, y, x, y)),
        Some((left, top, right, bottom)) => {
            Some((left.min(x), top.min(y), right.max(x), bottom.max(y)))
        }
    });
    bounds.map_or(0, |(left, top, right, bottom)| {
        (right - left).max(bottom - top) + 1
    })
}

/// `MAGIC`, the count, then per icon: the target's length and UTF-8, `fetched`, `side`, and the pixels. Little-endian.
pub(crate) fn encode(icons: &[Cached]) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    // At most `LIMIT` icons, each target a shell path, so both lengths fit a u32.
    bytes.extend((icons.len() as u32).to_le_bytes());
    for icon in icons {
        bytes.extend((icon.target.len() as u32).to_le_bytes());
        bytes.extend(icon.target.as_bytes());
        bytes.extend(icon.fetched.0.to_le_bytes());
        bytes.extend(icon.pixels.side().to_le_bytes());
        bytes.extend(icon.pixels.bgra());
    }
    bytes
}

/// What `encode` wrote, or `None` for anything else.
pub(crate) fn decode(bytes: &[u8]) -> Option<Vec<Cached>> {
    let (count, mut rest) = u32_at(bytes.strip_prefix(MAGIC)?)?;
    let mut icons = Vec::new();
    for _ in 0..count {
        let (length, after) = u32_at(rest)?;
        let (target, after) = after.split_at_checked(usize::try_from(length).ok()?)?;
        let (fetched, after) = after.split_first_chunk::<8>()?;
        let (side, after) = u32_at(after)?;
        let (bgra, after) = after.split_at_checked(byte_count(side)?)?;
        icons.push(Cached {
            target: String::from_utf8(target.to_vec()).ok()?,
            fetched: UnixSeconds(u64::from_le_bytes(*fetched)),
            pixels: Pixels::new(side, bgra.to_vec())?,
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
    use windows::Win32::Graphics::Gdi::CreateBitmap;

    use super::*;

    fn icon(target: &str, fetched: u64, side: u32) -> Cached {
        Cached {
            target: target.to_owned(),
            fetched: UnixSeconds(fetched),
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
    fn pixels_take_exactly_four_bytes_per_pixel() {
        assert!(Pixels::new(2, vec![0; 16]).is_some());
        assert!(Pixels::new(2, vec![0; 15]).is_none());
        assert!(Pixels::new(u32::MAX, Vec::new()).is_none());
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
    fn an_unreadable_cache_names_its_path() -> Result<(), std::io::Error> {
        let path = std::env::temp_dir().join(format!(
            "carronade-icons-invalid-{}.bin",
            std::process::id()
        ));
        std::fs::write(&path, b"not icons")?;
        let result = load(&path);
        std::fs::remove_file(&path)?;
        assert!(
            matches!(&result, Err(Error::IconCache { path: failed }) if *failed == path),
            "got {result:?}"
        );
        Ok(())
    }

    #[test]
    fn find_skips_icons_of_another_size_or_too_old() {
        let icons = [icon("a", 100, 2)];
        let at = |seconds: u64| UnixSeconds(100 + seconds);
        assert!(find(&icons, "a", 2, at(MAX_AGE_SECONDS - 1)).is_some());
        assert!(find(&icons, "a", 2, at(MAX_AGE_SECONDS)).is_none());
        assert!(find(&icons, "a", 3, at(0)).is_none());
        assert!(find(&icons, "b", 2, at(0)).is_none());
    }

    #[test]
    fn fetched_icons_replace_and_lead_the_previous_ones() {
        let merged = merged(
            vec![icon("b", 9, 1), icon("c", 9, 1)],
            vec![icon("a", 1, 1), icon("b", 1, 1)],
        );
        assert_eq!(targets(&merged), ["b", "c", "a"]);
        assert_eq!(
            merged.first().map(|icon| icon.fetched),
            Some(UnixSeconds(9))
        );
    }

    #[test]
    fn merging_keeps_at_most_the_limit() {
        let previous = (0..LIMIT + 5).map(|n| icon(&n.to_string(), 1, 0)).collect();
        assert_eq!(merged(vec![], previous).len(), LIMIT);
    }

    /// A `width` px wide 32-bit image with `alpha` at each `(x, y)` and 0 elsewhere.
    fn image(width: usize, height: usize, alpha: u8, at: &[(usize, usize)]) -> Vec<u8> {
        let mut pixels = vec![0; width * height * 4];
        for (x, y) in at {
            if let Some(byte) = pixels.get_mut((y * width + x) * 4 + 3) {
                *byte = alpha;
            }
        }
        pixels
    }

    #[test]
    fn a_bitmap_without_32_bit_pixels_names_its_target() -> Result<(), Error> {
        // SAFETY: a 1 px monochrome bitmap with no initial bits, deleted below.
        let bitmap = unsafe { CreateBitmap(1, 1, 1, 1, None) };
        let result = opaque_side("mono", bitmap);
        // SAFETY: `bitmap` came from `CreateBitmap` and nothing else holds it.
        unsafe { DeleteObject(bitmap.into()) }
            .ok()
            .map_err(error::win32("DeleteObject"))?;
        assert!(
            matches!(&result, Err(Error::IconBitmap { target }) if target == "mono"),
            "got {result:?}"
        );
        Ok(())
    }

    #[test]
    fn the_extent_is_the_longer_side_of_the_opaque_box() {
        let pixels = image(8, 8, 255, &[(2, 3), (5, 4)]);
        assert_eq!(opaque_extent(&pixels, 32), 4);
    }

    #[test]
    fn translucent_pixels_are_not_counted() {
        let pixels = image(8, 8, 66, &[(0, 0), (7, 7)]);
        assert_eq!(opaque_extent(&pixels, 32), 0);
    }

    #[test]
    fn a_single_opaque_pixel_spans_one() {
        let pixels = image(8, 8, 128, &[(6, 1)]);
        assert_eq!(opaque_extent(&pixels, 32), 1);
    }
}
