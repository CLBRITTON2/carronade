//! `carronade dmenu` prints the stdin line picked, `carronade drun` launches the Start menu app picked. Either exits 1
//! on cancel and 2 on error. `--config <path>` replaces `%APPDATA%\carronade\config.toml`.

// No console window flashes up when GlazeWM starts it. Piped stdin and stdout still reach it.
#![windows_subsystem = "windows"]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use carronade::apps;
use carronade::config::{self, Config};
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

enum Mode {
    Dmenu,
    Drun,
}

/// Runs the mode `args` names, returning whether something was picked.
fn run(args: Vec<String>) -> Result<bool, Error> {
    let (path, mode) = match args.as_slice() {
        [mode] => (None, mode),
        [flag, path, mode] if flag == "--config" => (Some(PathBuf::from(path)), mode),
        _ => return Err(Error::Usage(args)),
    };
    let mode = match mode.as_str() {
        "dmenu" => Mode::Dmenu,
        "drun" => Mode::Drun,
        _ => return Err(Error::Usage(args)),
    };
    let config = config::load(&path.map_or_else(config::path, Ok)?)?;
    match mode {
        Mode::Dmenu => dmenu(config),
        Mode::Drun => drun(config),
    }
}

fn dmenu(config: Config) -> Result<bool, Error> {
    let lines: Vec<String> = std::io::stdin()
        .lines()
        .collect::<Result<_, _>>()
        .map_err(Error::Stdin)?;
    let line = match pick(config, lines)? {
        Choice::Item(line) | Choice::Text(line) => line,
        Choice::Cancel => return Ok(false),
    };
    writeln!(std::io::stdout(), "{line}").map_err(Error::Stdout)?;
    Ok(true)
}

fn drun(config: Config) -> Result<bool, Error> {
    if !config.drun.cache {
        return launch(pick(config, apps::list()?)?);
    }
    let path = apps::cache_path()?;
    let Some(cached) = apps::load(&path)? else {
        let found = apps::list()?;
        apps::save(&path, &found)?;
        return launch(pick(config, found)?);
    };
    let launched = launch(pick(config, cached)?)?;
    // After the picker closes, since listing beside it slowed its startup by tens of ms.
    apps::save(&path, &apps::list()?)?;
    Ok(launched)
}

/// Launches what `choice` names, returning whether there was one.
fn launch(choice: Choice<apps::App>) -> Result<bool, Error> {
    match choice {
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
