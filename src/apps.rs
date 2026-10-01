//! The Start menu's apps, the list `Get-StartApps` prints, and launching them.

use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, IBindCtx,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT,
    SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW, SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

use crate::error::{Error, win32};

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

impl AsRef<str> for App {
    fn as_ref(&self) -> &str {
        &self.name
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

/// The shell needs COM on the calling thread. A second call on the same thread is a no-op.
fn com() -> Result<(), Error> {
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
