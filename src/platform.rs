//! Windows setup the other modules share: COM on the calling thread and the known folders.

use std::path::PathBuf;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::GUID;

use crate::error::{Error, win32};

/// The shell needs COM on the calling thread. A second call on the same thread is a no-op.
pub(crate) fn com() -> Result<(), Error> {
    // SAFETY: the reserved argument is null, and the apartment model matches every other call on this thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(win32("CoInitializeEx"))
}

pub(crate) fn known_folder(id: &GUID) -> Result<PathBuf, Error> {
    // SAFETY: `id` points at a live GUID and no access token is passed, so the folder is the caller's.
    let folder = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }
        .map_err(win32("SHGetKnownFolderPath"))?;
    // SAFETY: the shell returned `folder` NUL-terminated and it is still allocated.
    let text = unsafe { folder.to_string() };
    // SAFETY: the shell allocated `folder` with the COM allocator and nothing reads it after this.
    unsafe { CoTaskMemFree(Some(folder.0 as _)) };
    Ok(PathBuf::from(text?))
}
