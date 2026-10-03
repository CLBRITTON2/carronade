//! Commands that lock, end the session or power the machine down, listed after the apps.

use windows::Win32::Foundation::{ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES,
};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Shutdown::{
    EWX_LOGOFF, EWX_POWEROFF, EWX_REBOOT, EXIT_WINDOWS_FLAGS, ExitWindowsEx, LockWorkStation,
    SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::Owned;

use crate::error::{Error, last, win32};
use crate::picker::{Picture, Row};

// No Sleep: SetSuspendState has no way into Modern Standby (S0 low power idle), the only sleep many laptops have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Lock,
    SignOut,
    Hibernate,
    Restart,
    ShutDown,
}

impl Command {
    pub const ALL: [Command; 5] = [
        Command::Lock,
        Command::SignOut,
        Command::Hibernate,
        Command::Restart,
        Command::ShutDown,
    ];
}

impl Row for Command {
    fn label(&self) -> &str {
        match self {
            Command::Lock => "Lock",
            Command::SignOut => "Sign out",
            Command::Hibernate => "Hibernate",
            Command::Restart => "Restart",
            Command::ShutDown => "Shut down",
        }
    }

    fn alias(&self) -> Option<&str> {
        None
    }

    /// Segoe MDL2 Assets glyphs: Lock, LeaveChat, QuietHours, UpdateRestore and PowerButton.
    fn icon(&self) -> Option<Picture> {
        Some(Picture::Glyph(match self {
            Command::Lock => '\u{e72e}',
            Command::SignOut => '\u{e89b}',
            Command::Hibernate => '\u{e708}',
            Command::Restart => '\u{e777}',
            Command::ShutDown => '\u{e7e8}',
        }))
    }

    fn boost(&self) -> i32 {
        0
    }
}

pub fn run(command: Command) -> Result<(), Error> {
    match command {
        Command::Lock => unsafe { LockWorkStation() }.map_err(win32("LockWorkStation")),
        Command::SignOut => exit_windows(EWX_LOGOFF),
        Command::Hibernate => match unsafe { SetSuspendState(true, false, false) } {
            true => Ok(()),
            false => Err(last("SetSuspendState")),
        },
        Command::Restart => {
            enable_shutdown()?;
            exit_windows(EWX_REBOOT)
        }
        Command::ShutDown => {
            enable_shutdown()?;
            exit_windows(EWX_POWEROFF)
        }
    }
}

fn exit_windows(flags: EXIT_WINDOWS_FLAGS) -> Result<(), Error> {
    unsafe { ExitWindowsEx(flags, SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_FLAG_PLANNED) }
        .map_err(win32("ExitWindowsEx"))
}

/// Turns on the shutdown privilege, which every account holds but has off, as restart and shut down need.
fn enable_shutdown() -> Result<(), Error> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut token) }
        .map_err(win32("OpenProcessToken"))?;
    let token = unsafe { Owned::new(token) };
    enable(*token, SE_SHUTDOWN_NAME)
}

fn enable(token: HANDLE, privilege: windows::core::PCWSTR) -> Result<(), Error> {
    let mut luid = LUID::default();
    unsafe { LookupPrivilegeValueW(None, privilege, &mut luid) }
        .map_err(win32("LookupPrivilegeValueW"))?;
    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    unsafe { AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None) }
        .map_err(win32("AdjustTokenPrivileges"))?;
    // It succeeds without enabling a privilege the account lacks, and says so only in the last error.
    match unsafe { GetLastError() } {
        ERROR_NOT_ALL_ASSIGNED => Err(last("AdjustTokenPrivileges")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shutdown_privilege_turns_on() -> Result<(), Error> {
        enable_shutdown()
    }
}
