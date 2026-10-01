//! The Start menu's apps, the list `Get-StartApps` prints, and launching them.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, IBindCtx,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_LocalAppData, IEnumShellItems, IShellItem,
    IShellItemImageFactory, KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SHCreateItemFromParsingName,
    SHELLEXECUTEINFOW, SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

use crate::config::known_folder;
use crate::error::{Error, win32};
use crate::picker::Row;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
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

    fn icon(&self) -> Option<String> {
        Some(self.target())
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

/// `%LOCALAPPDATA%\carronade\apps.toml`, the apps drun found last time.
pub fn cache_path() -> Result<PathBuf, Error> {
    Ok(known_folder(&FOLDERID_LocalAppData)?
        .join("carronade")
        .join("apps.toml"))
}

/// The apps `save` wrote to `path`, or `None` before the first save.
pub fn load(path: &Path) -> Result<Option<Vec<App>>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::CacheRead {
                path: path.to_owned(),
                source,
            });
        }
    };
    let cache: Cache = toml::from_str(&text).map_err(|source| Error::CacheParse {
        path: path.to_owned(),
        source: Box::new(source),
    })?;
    Ok(Some(cache.app))
}

/// Writes `apps` for `load`, through a temporary file so a reader never sees half a list.
pub fn save(path: &Path, apps: &[App]) -> Result<(), Error> {
    let text = toml::to_string(&CacheRef { app: apps }).map_err(Error::CacheSerialize)?;
    let temporary = path.with_extension("toml.tmp");
    let write = |source| Error::CacheWrite {
        path: path.to_owned(),
        source,
    };
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(write)?;
    }
    std::fs::write(&temporary, text).map_err(write)?;
    std::fs::rename(&temporary, path).map_err(write)
}

/// Opens `target` as the Run dialog would: an `App::target`, a program on PATH, a path or a URL.
pub fn launch(target: &str) -> Result<(), Error> {
    com()?;
    let file: Vec<u16> = target.encode_utf16().chain([0]).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|source| Error::Launch {
        target: target.to_owned(),
        source,
    })
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
