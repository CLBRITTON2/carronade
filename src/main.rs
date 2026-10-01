//! `carronade dmenu` prints the stdin line picked, `carronade drun` launches the Start menu app picked. Either exits 1
//! on cancel and 2 on error.

// No console window flashes up when GlazeWM starts it. Piped stdin and stdout still reach it.
#![windows_subsystem = "windows"]

use std::io::Write;
use std::process::ExitCode;

use carronade::apps;
use carronade::error::Error;
use carronade::menu::Choice;
use carronade::picker::pick;
use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
use windows::core::{HSTRING, w};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            report(&error);
            ExitCode::from(2)
        }
    }
}

/// Runs the mode `args` names, returning whether something was picked.
fn run(args: Vec<String>) -> Result<bool, Error> {
    match args.as_slice() {
        [mode] if mode == "dmenu" => dmenu(),
        [mode] if mode == "drun" => drun(),
        _ => Err(Error::Usage(args)),
    }
}

fn dmenu() -> Result<bool, Error> {
    let lines: Vec<String> = std::io::stdin()
        .lines()
        .collect::<Result<_, _>>()
        .map_err(Error::Stdin)?;
    let line = match pick(lines)? {
        Choice::Item(line) | Choice::Text(line) => line,
        Choice::Cancel => return Ok(false),
    };
    writeln!(std::io::stdout(), "{line}").map_err(Error::Stdout)?;
    Ok(true)
}

fn drun() -> Result<bool, Error> {
    match pick(apps::list()?)? {
        Choice::Item(app) => apps::launch(&app.target())?,
        Choice::Text(command) => apps::launch(&command)?,
        Choice::Cancel => return Ok(false),
    }
    Ok(true)
}

/// Writes to stderr when the caller gave one, else shows a message box: started from a hotkey, nothing reads stderr.
fn report(error: &Error) {
    match unsafe { GetStdHandle(STD_ERROR_HANDLE) } {
        Ok(handle) if !handle.is_invalid() => eprintln!("carronade: {error}"),
        _ => {
            unsafe {
                MessageBoxW(
                    None,
                    &HSTRING::from(error.to_string()),
                    w!("carronade"),
                    MB_ICONERROR,
                )
            };
        }
    }
}
