//! The Start menu's apps, the list `Get-StartApps` prints, and launching them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{ERROR_CANCELLED, ERROR_NOT_FOUND, SIZE};
use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HBITMAP};
use windows::Win32::Storage::EnhancedStorage::PKEY_Link_TargetParsingPath;
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItem2,
    IShellItemImageFactory, KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SHCreateItemFromParsingName,
    SHELLEXECUTEINFOW, SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, w};

use crate::error::{Error, win32};
use crate::picker::{Picture, Row};
use crate::platform::com;
use crate::store;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub name: String,
    /// The `AppsFolder` parsing name: an AUMID for packaged apps, a known-folder path for the rest.
    pub id: String,
    /// The file name, without `.exe`, of the program its shortcut starts. Packaged apps have none.
    pub exe: Option<String>,
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

    fn alias(&self) -> Option<&str> {
        self.exe.as_deref()
    }

    fn icon(&self) -> Option<Picture> {
        Some(Picture::Shell(self.target()))
    }

    fn boost(&self) -> i32 {
        0
    }
}

/// Every app in the Start menu's All apps list, packaged ones included, sorted by name.
pub fn list() -> Result<Vec<App>, Error> {
    com()?;
    // SAFETY: COM is initialized on this thread by `com` and the folder id is a static GUID.
    let folder: IShellItem =
        unsafe { SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) }
            .map_err(win32("SHGetKnownFolderItem"))?;
    // SAFETY: `folder` is a live shell item and no bind context is needed.
    let items: IEnumShellItems =
        unsafe { folder.BindToHandler(None::<&IBindCtx>, &BHID_EnumItems) }
            .map_err(win32("IShellItem::BindToHandler"))?;
    let mut apps = Vec::new();
    loop {
        let mut next = [None];
        // SAFETY: `next` holds one slot, and the fetched count may be omitted when asking for one item.
        unsafe { items.Next(&mut next, None) }.map_err(win32("IEnumShellItems::Next"))?;
        let [Some(item)] = next else { break };
        apps.push(App {
            name: display_name(&item, SIGDN_NORMALDISPLAY)?,
            id: display_name(&item, SIGDN_PARENTRELATIVEPARSING)?,
            exe: link_target(&item)?.as_deref().and_then(exe_name),
        });
    }
    apps.sort_by_key(|app| app.name.to_lowercase());
    Ok(apps)
}

/// The format version of `apps.toml`, raised when `App` changes shape.
const VERSION: i64 = 1;

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
    Ok(store::load::<Cache>(path, VERSION)?.map(|cache| cache.app))
}

pub fn save(path: &Path, apps: &[App]) -> Result<(), Error> {
    store::save(path, VERSION, &CacheRef { app: apps })
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
    // SAFETY: `info` is sized, and `file`, `verb` and `directory` outlive the call as NUL-terminated or null strings.
    unsafe { ShellExecuteExW(&mut info) }
}

/// The icon the shell shows for `target`, at most `size` px square, as a 32-bit bitmap with premultiplied alpha.
pub fn icon(target: &str, size: i32) -> Result<HBITMAP, Error> {
    com()?;
    let icon = |source| Error::Icon {
        target: target.to_owned(),
        source,
    };
    // SAFETY: COM is initialized on this thread and the name is a temporary HSTRING that outlives the call.
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(target), None) }.map_err(icon)?;
    // SAFETY: `factory` is a live COM object, and the caller owns the returned bitmap.
    unsafe { factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY) }.map_err(icon)
}

/// The shell's largest icon size. It scales smaller requests up from coarse assets, so fetch this and scale down.
const LARGEST: i32 = 256;

/// The icon to show for `target` at `size` px. An app with no large image comes back from a `LARGEST` request as its
/// small one, unscaled, in the middle of a translucent frame, so that one is fetched again at `size`.
pub fn display_icon(target: &str, size: i32) -> Result<HBITMAP, Error> {
    let large = icon(target, LARGEST)?;
    let side = opaque_side(target, large);
    if let Ok(side) = side
        && side * 2 >= LARGEST.unsigned_abs() as usize
    // A u32 widens on 64-bit Windows.
    {
        return Ok(large);
    }
    // SAFETY: `large` came from `icon`, this function owns it, and it is not used after.
    let deleted = unsafe { DeleteObject(large.into()) }
        .ok()
        .map_err(win32("DeleteObject"));
    // The bitmap error is the cause, so it wins over a failed delete.
    side?;
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

fn display_name(item: &IShellItem, form: SIGDN) -> Result<String, Error> {
    // SAFETY: `item` is a live shell item, and `taken` frees the returned string.
    let name = unsafe { item.GetDisplayName(form) }.map_err(win32("IShellItem::GetDisplayName"))?;
    taken(name)
}

/// What the shortcut behind `item` opens, or `None` for an app with no shortcut, as packaged apps are.
fn link_target(item: &IShellItem) -> Result<Option<String>, Error> {
    let item: IShellItem2 = item
        .cast()
        .map_err(win32("IShellItem::cast::<IShellItem2>"))?;
    // SAFETY: `item` is a live shell item, the key is a static PROPERTYKEY, and `taken` frees the returned string.
    match unsafe { item.GetString(&PKEY_Link_TargetParsingPath) } {
        Ok(target) => taken(target).map(Some),
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(None),
        Err(error) => Err(win32("IShellItem2::GetString(Link.TargetParsingPath)")(
            error,
        )),
    }
}

/// The file name of `target` without its extension, when it is an exe.
fn exe_name(target: &str) -> Option<String> {
    let path = Path::new(target);
    path.extension()
        .filter(|extension| extension.eq_ignore_ascii_case("exe"))?;
    path.file_stem()?.to_str().map(str::to_owned)
}

/// `text` as a `String`, freeing the shell's copy.
fn taken(text: PWSTR) -> Result<String, Error> {
    // SAFETY: the shell returned `text` NUL-terminated and it is still allocated.
    let owned = unsafe { text.to_string() };
    // SAFETY: the shell allocated `text` with the COM allocator and nothing reads it after this.
    unsafe { CoTaskMemFree(Some(text.0 as _)) };
    Ok(owned?)
}

#[cfg(test)]
mod tests {
    use windows::Win32::Graphics::Gdi::CreateBitmap;

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
    fn a_bitmap_without_32_bit_pixels_names_its_target() -> Result<(), Error> {
        // SAFETY: a 1 px monochrome bitmap with no initial bits, deleted below.
        let bitmap = unsafe { CreateBitmap(1, 1, 1, 1, None) };
        let result = opaque_side("mono", bitmap);
        // SAFETY: `bitmap` came from `CreateBitmap` and nothing else holds it.
        unsafe { DeleteObject(bitmap.into()) }
            .ok()
            .map_err(win32("DeleteObject"))?;
        assert!(
            matches!(&result, Err(Error::IconBitmap { target }) if target == "mono"),
            "got {result:?}"
        );
        Ok(())
    }

    #[test]
    fn only_an_exe_target_names_an_exe() {
        let names = [
            "C:\\Program Files\\Acme\\acme.exe",
            "C:\\Program Files\\Widget Suite\\WIDGET.EXE",
            "C:\\Windows\\system32\\gadgets.msc",
            "::{52205FD8-5DFB-447D-801A-D0B52F2E83E1}",
            "https://example.com/",
        ]
        .map(exe_name);
        assert_eq!(
            names,
            [
                Some("acme".to_owned()),
                Some("WIDGET".to_owned()),
                None,
                None,
                None
            ]
        );
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
