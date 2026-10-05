//! Opening apps, files, folders, URLs and typed text through the shell, as the Run dialog does.

use std::path::Path;

use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::UI::Shell::{
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

use crate::error::Error;
use crate::platform::com;

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
        // A SHELLEXECUTEINFOW is 112 bytes.
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        // The process exits right after the call, which ShellExecuteEx documents as needing NOASYNC.
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpVerb: verb,
        lpFile: PCWSTR(file.as_ptr()),
        lpDirectory: directory,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is sized, and `file`, `verb` and `directory` outlive the call as NUL-terminated or null strings.
    unsafe { ShellExecuteExW(&raw mut info) }
}
