//! The Start menu's apps, the list `Get-StartApps` prints.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Storage::EnhancedStorage::PKEY_Link_TargetParsingPath;
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItem2, KF_FLAG_DEFAULT,
    SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};
use windows::core::{Interface, PWSTR};

use crate::error::{Error, utf16, win32};
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
    #[must_use]
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

fn display_name(item: &IShellItem, form: SIGDN) -> Result<String, Error> {
    // SAFETY: `item` is a live shell item, and `taken` frees the returned string.
    let name = unsafe { item.GetDisplayName(form) }.map_err(win32("IShellItem::GetDisplayName"))?;
    taken(name, "an app's display name")
}

/// What the shortcut behind `item` opens, or `None` for an app with no shortcut, as packaged apps are.
fn link_target(item: &IShellItem) -> Result<Option<String>, Error> {
    let item: IShellItem2 = item
        .cast()
        .map_err(win32("IShellItem::cast::<IShellItem2>"))?;
    // SAFETY: `item` is a live shell item, the key is a static PROPERTYKEY, and `taken` frees the returned string.
    match unsafe { item.GetString(&PKEY_Link_TargetParsingPath) } {
        Ok(target) => taken(target, "a shortcut's target").map(Some),
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

/// `text`, which is `what`, as a `String`, freeing the shell's copy.
fn taken(text: PWSTR, what: &'static str) -> Result<String, Error> {
    // SAFETY: the shell returned `text` NUL-terminated and it is still allocated.
    let owned = unsafe { text.to_string() };
    // SAFETY: the shell allocated `text` with the COM allocator and nothing reads it after this.
    unsafe { CoTaskMemFree(Some(text.0 as _)) };
    owned.map_err(utf16(what))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
