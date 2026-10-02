//! The Start menu's apps, the list `Get-StartApps` prints, and launching them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{ERROR_CANCELLED, SIZE};
use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HBITMAP};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, IBindCtx,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItemImageFactory,
    KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SHCreateItemFromParsingName, SHELLEXECUTEINFOW,
    SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY,
    ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

use crate::error::{Error, win32};
use crate::picker::{Picture, Row};
use crate::store;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub name: String,
    /// The AppsFolder parsing name: an AUMID for packaged apps, a known-folder path for the rest.
    pub id: String,
}

impl App {
    pub fn target(&self) -> String {
        format!("shell:AppsFolder\\{}", self.id)
    }
}

impl Row for App {
    fn label(&self) -> &str {
        &self.name
    }

    fn icon(&self) -> Option<Picture> {
        Some(Picture::Shell(self.target()))
    }
}

/// Every app in the Start menu's All apps list, packaged ones included, sorted by name.
pub fn list() -> Result<Vec<App>, Error> {
    com()?;
    let folder: IShellItem =
        unsafe { SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) }
            .map_err(win32("SHGetKnownFolderItem"))?;
    let items: IEnumShellItems =
        unsafe { folder.BindToHandler(None::<&IBindCtx>, &BHID_EnumItems) }
            .map_err(win32("IShellItem::BindToHandler"))?;
    let mut apps = Vec::new();
    loop {
        let mut next = [None];
        unsafe { items.Next(&mut next, None) }.map_err(win32("IEnumShellItems::Next"))?;
        let [Some(item)] = next else { break };
        apps.push(App {
            name: display_name(&item, SIGDN_NORMALDISPLAY)?,
            id: display_name(&item, SIGDN_PARENTRELATIVEPARSING)?,
        });
    }
    apps.sort_by_key(|app| app.name.to_lowercase());
    Ok(apps)
}

/// TOML needs a table at the top, so the apps sit in an `[[app]]` array.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    app: Vec<App>,
}

#[derive(Serialize)]
struct CacheRef<'a> {
    app: &'a [App],
}

/// `%LOCALAPPDATA%\carronade\apps.toml`, the apps found last time.
pub fn cache_path() -> Result<PathBuf, Error> {
    store::path("apps.toml")
}

/// The apps `save` wrote to `path`, or `None` before the first save.
pub fn load(path: &Path) -> Result<Option<Vec<App>>, Error> {
    Ok(store::load::<Cache>(path)?.map(|cache| cache.app))
}

pub fn save(path: &Path, apps: &[App]) -> Result<(), Error> {
    store::save(path, &CacheRef { app: apps })
}

/// Opens `target` as the Run dialog would: an `App::target`, a program on PATH, a path or a URL.
pub fn launch(target: &str) -> Result<(), Error> {
    com()?;
    shell_execute(target, PCWSTR::null(), PCWSTR::null()).map_err(|source| Error::Launch {
        target: target.to_owned(),
        source,
    })
}

/// Opens `target` as `launch` does, elevated after the UAC prompt. Returns false when the prompt was declined.
pub fn launch_as_admin(target: &str) -> Result<bool, Error> {
    com()?;
    match shell_execute(target, w!("runas"), PCWSTR::null()) {
        Ok(()) => Ok(true),
        Err(error) if error.code() == ERROR_CANCELLED.to_hresult() => Ok(false),
        Err(source) => Err(Error::Launch {
            target: target.to_owned(),
            source,
        }),
    }
}

/// Starts `program` as the Run dialog would, with `folder` as its working folder.
pub fn launch_in(program: &str, folder: &Path) -> Result<(), Error> {
    com()?;
    let directory = HSTRING::from(folder);
    shell_execute(program, PCWSTR::null(), PCWSTR(directory.as_ptr())).map_err(|source| {
        Error::LaunchIn {
            program: program.to_owned(),
            folder: folder.to_owned(),
            source,
        }
    })
}

/// Needs `com` first. `verb` is null for the default one, `directory` null for the caller's working folder.
fn shell_execute(
    target: &str,
    verb: PCWSTR,
    directory: PCWSTR,
) -> Result<(), windows::core::Error> {
    let file: Vec<u16> = target.encode_utf16().chain([0]).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        lpVerb: verb,
        lpFile: PCWSTR(file.as_ptr()),
        lpDirectory: directory,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }
}

/// The icon the shell shows for `target`, at most `size` px square, as a 32-bit bitmap with premultiplied alpha.
pub fn icon(target: &str, size: i32) -> Result<HBITMAP, Error> {
    com()?;
    let icon = |source| Error::Icon {
        target: target.to_owned(),
        source,
    };
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(target), None) }.map_err(icon)?;
    unsafe { factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY) }.map_err(icon)
}

/// The shell's largest icon size. It scales smaller requests up from coarse assets, so fetch this and scale down.
const LARGEST: i32 = 256;

/// The icon to show for `target` at `size` px. An app with no large image comes back from a `LARGEST` request as its
/// small one, unscaled, in the middle of a translucent frame, so that one is fetched again at `size`.
pub fn display_icon(target: &str, size: i32) -> Result<HBITMAP, Error> {
    let large = icon(target, LARGEST)?;
    if opaque_side(target, large)? * 2 >= LARGEST.unsigned_abs() as usize {
        return Ok(large);
    }
    unsafe { DeleteObject(large.into()) }
        .ok()
        .map_err(win32("DeleteObject"))?;
    icon(target, size)
}

/// `opaque_extent` of the shell's icon `bitmap` for `target`.
fn opaque_side(target: &str, bitmap: HBITMAP) -> Result<usize, Error> {
    let mut info = BITMAP::default();
    let written = unsafe {
        GetObjectW(
            bitmap.into(),
            size_of::<BITMAP>() as i32,
            Some((&raw mut info).cast()),
        )
    };
    if written == 0 || info.bmBits.is_null() || info.bmBitsPixel != 32 {
        return Err(Error::IconBitmap {
            target: target.to_owned(),
        });
    }
    let stride = info.bmWidthBytes.unsigned_abs() as usize;
    let length = stride * info.bmHeight.unsigned_abs() as usize;
    let pixels = unsafe { std::slice::from_raw_parts(info.bmBits as *const u8, length) };
    Ok(opaque_extent(pixels, stride))
}

/// The longer side of the box around the pixels at least half opaque, in 32-bit BGRA rows of `stride` bytes.
pub(crate) fn opaque_extent(pixels: &[u8], stride: usize) -> usize {
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

/// The shell needs COM on the calling thread. A second call on the same thread is a no-op.
pub(crate) fn com() -> Result<(), Error> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(win32("CoInitializeEx"))
}

fn display_name(item: &IShellItem, form: SIGDN) -> Result<String, Error> {
    let name = unsafe { item.GetDisplayName(form) }.map_err(win32("IShellItem::GetDisplayName"))?;
    let text = unsafe { name.to_string() };
    unsafe { CoTaskMemFree(Some(name.0 as _)) };
    Ok(text?)
}

#[cfg(test)]
mod tests {
    use super::*;

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
