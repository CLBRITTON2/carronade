//! The picker's shell icons, fetched from the shell, scaled once and kept in `%LOCALAPPDATA%\carronade\icons.bin`,
//! since the shell takes tens of ms per icon and the cache a fraction of one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, SIZE};
use windows::Win32::Graphics::Gdi::{BITMAP, GetObjectW, HBITMAP, HPALETTE};
use windows::Win32::Graphics::Imaging::{
    IWICImagingFactory, WICBitmapInterpolationModeHighQualityCubic, WICBitmapUsePremultipliedAlpha,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
};
use windows::core::{HRESULT, HSTRING, Owned};

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
    let bytes = encode(icons).ok_or_else(|| Error::IconEncode {
        path: path.to_owned(),
        count: icons.len(),
    })?;
    store::write(path, &bytes)
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
    // `config::Side` keeps a side to a few thousand px.
    let icon = display_icon(target, side as i32)?;
    scaled(wic, target, &icon, side)
}

fn scaled(
    wic: &IWICImagingFactory,
    target: &str,
    icon: &Owned<HBITMAP>,
    side: u32,
) -> Result<Pixels, Error> {
    // SAFETY: `icon` is a live bitmap borrowed for the call, and a 32-bit one needs no palette.
    let bitmap = unsafe {
        wic.CreateBitmapFromHBITMAP(**icon, HPALETTE::default(), WICBitmapUsePremultipliedAlpha)
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
    let bytes = byte_count(side).ok_or_else(|| Error::IconSide {
        target: target.to_owned(),
        side,
    })?;
    let mut bgra = vec![0; bytes];
    // SAFETY: a null rect copies the whole `side` square, which fills `bgra` exactly at a stride of `side * 4`.
    unsafe { scaler.CopyPixels(std::ptr::null(), side * 4, &mut bgra) }
        .map_err(error::icon(target, "IWICBitmapScaler::CopyPixels"))?;
    Ok(Pixels { side, bgra })
}

/// How `SHCreateItemFromParsingName` says a target is gone, as a cached file or app can be since it was listed.
const GONE: [HRESULT; 2] = [
    ERROR_FILE_NOT_FOUND.to_hresult(),
    ERROR_PATH_NOT_FOUND.to_hresult(),
];

/// The icon the shell shows for `target`, at most `size` px square, as a 32-bit bitmap with premultiplied alpha.
/// A target the shell cannot find is `Error::NoItem`.
pub fn icon(target: &str, size: i32) -> Result<Owned<HBITMAP>, Error> {
    com()?;
    // SAFETY: COM is initialized on this thread and the name is a temporary HSTRING that outlives the call.
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(target), None) }.map_err(|source| {
            if GONE.contains(&source.code()) {
                Error::NoItem {
                    target: target.to_owned(),
                    source,
                }
            } else {
                error::icon(target, "SHCreateItemFromParsingName")(source)
            }
        })?;
    // SAFETY: `factory` is a live COM object.
    let bitmap = unsafe { factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY) }
        .map_err(error::icon(target, "IShellItemImageFactory::GetImage"))?;
    // SAFETY: GetImage hands the caller a fresh bitmap nothing else deletes.
    Ok(unsafe { Owned::new(bitmap) })
}

/// The shell's largest icon size. It scales smaller requests up from coarse assets, so fetch this and scale down.
const LARGEST: i32 = 256;
/// The smallest opaque side of a real `LARGEST` icon. A u32 widens to usize on 64-bit Windows.
const HALF_LARGEST: usize = LARGEST.unsigned_abs() as usize / 2;

/// The icon to show for `target` at `size` px. An app with no large image comes back from a `LARGEST` request as its
/// small one, unscaled, in the middle of a translucent frame, so that one is fetched again at `size`.
pub fn display_icon(target: &str, size: i32) -> Result<Owned<HBITMAP>, Error> {
    let large = icon(target, LARGEST)?;
    if opaque_side(target, &large)? >= HALF_LARGEST {
        return Ok(large);
    }
    icon(target, size)
}

/// `opaque_extent` of the shell's icon `bitmap` for `target`.
fn opaque_side(target: &str, bitmap: &Owned<HBITMAP>) -> Result<usize, Error> {
    let mut info = BITMAP::default();
    // SAFETY: `info` is a writable BITMAP of exactly the size passed.
    let written = unsafe {
        GetObjectW(
            (**bitmap).into(),
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
/// `None` when the count or a target's length is past a u32.
pub(crate) fn encode(icons: &[Cached]) -> Option<Vec<u8>> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend(u32::try_from(icons.len()).ok()?.to_le_bytes());
    for icon in icons {
        bytes.extend(u32::try_from(icon.target.len()).ok()?.to_le_bytes());
        bytes.extend(icon.target.as_bytes());
        bytes.extend(icon.fetched.0.to_le_bytes());
        bytes.extend(icon.pixels.side().to_le_bytes());
        bytes.extend(icon.pixels.bgra());
    }
    Some(bytes)
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
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS,
    };

    use super::*;

    fn cached(target: &str, fetched: u64, side: u32) -> Cached {
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
        let icons = vec![cached("a", 1, 2), cached("ü", 3, 0)];
        assert_eq!(encode(&icons).and_then(|bytes| decode(&bytes)), Some(icons));
    }

    #[test]
    fn a_truncated_or_padded_cache_does_not_decode() -> Result<(), &'static str> {
        let bytes = encode(&[cached("a", 1, 2)]).ok_or("encoding failed")?;
        assert_eq!(bytes.split_last().and_then(|(_, rest)| decode(rest)), None);
        assert_eq!(decode(&[bytes.as_slice(), &[0]].concat()), None);
        assert_eq!(decode(b"carric00\0\0\0\0"), None);
        Ok(())
    }

    #[test]
    fn an_invalid_cache_names_its_path() -> Result<(), std::io::Error> {
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
        let icons = [cached("a", 100, 2)];
        let at = |seconds: u64| UnixSeconds(100 + seconds);
        assert!(find(&icons, "a", 2, at(MAX_AGE_SECONDS - 1)).is_some());
        assert!(find(&icons, "a", 2, at(MAX_AGE_SECONDS)).is_none());
        assert!(find(&icons, "a", 3, at(0)).is_none());
        assert!(find(&icons, "b", 2, at(0)).is_none());
    }

    #[test]
    fn fetched_icons_replace_and_lead_the_previous_ones() {
        let merged = merged(
            vec![cached("b", 9, 1), cached("c", 9, 1)],
            vec![cached("a", 1, 1), cached("b", 1, 1)],
        );
        assert_eq!(targets(&merged), ["b", "c", "a"]);
        assert_eq!(
            merged.first().map(|icon| icon.fetched),
            Some(UnixSeconds(9))
        );
    }

    #[test]
    fn merging_keeps_at_most_the_limit() {
        let previous = (0..LIMIT + 5)
            .map(|n| cached(&n.to_string(), 1, 0))
            .collect();
        assert_eq!(merged(vec![], previous).len(), LIMIT);
    }

    /// The `opaque_extent` of an 8 px square 32-bit image with `alpha` at each `(x, y)` and 0 elsewhere, or `None` for
    /// a point outside it.
    fn extent(alpha: u8, at: &[(usize, usize)]) -> Option<usize> {
        let mut pixels = vec![0; 8 * 8 * 4];
        for &(x, y) in at {
            if x >= 8 {
                return None;
            }
            *pixels.get_mut((y * 8 + x) * 4 + 3)? = alpha;
        }
        Some(opaque_extent(&pixels, 8 * 4))
    }

    #[test]
    fn a_bitmap_without_32_bit_pixels_names_its_target() -> windows::core::Result<()> {
        let header = BITMAPINFOHEADER {
            // A BITMAPINFOHEADER is 40 bytes.
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 1,
            biHeight: 1,
            biPlanes: 1,
            biBitCount: 8,
            biCompression: BI_RGB.0,
            // BITMAPINFO holds one color, so the table must not claim the 256 an 8-bit DIB defaults to.
            biClrUsed: 1,
            ..Default::default()
        };
        let info = BITMAPINFO {
            bmiHeader: header,
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: `info` describes an 8-bit DIB with a one-color table, and `bits` is a live local the call fills.
        let bitmap = unsafe {
            CreateDIBSection(
                None,
                &raw const info,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            )
        }?;
        // SAFETY: `bitmap` came from `CreateDIBSection` and nothing else holds it.
        let bitmap = unsafe { Owned::new(bitmap) };
        let result = opaque_side("8-bit", &bitmap);
        assert!(
            matches!(&result, Err(Error::IconBitmap { target }) if target == "8-bit"),
            "got {result:?}"
        );
        Ok(())
    }

    #[test]
    fn the_extent_is_the_longer_side_of_the_opaque_box() {
        assert_eq!(extent(255, &[(2, 3), (5, 4)]), Some(4));
    }

    #[test]
    fn pixels_below_half_opaque_are_not_counted() {
        assert_eq!(extent(127, &[(0, 0), (7, 7)]), Some(0));
    }

    #[test]
    fn a_single_opaque_pixel_spans_one() {
        assert_eq!(extent(128, &[(6, 1)]), Some(1));
    }
}
