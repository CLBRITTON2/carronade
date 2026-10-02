//! `carronade dmenu` prints the stdin line picked, `carronade apps` launches the Start menu app picked, and
//! `carronade files` opens the file or folder picked from below `files.roots`. apps and files switch to each other in
//! the same window. Each exits 1 on cancel and 2 on error. `--config <path>` replaces
//! `%APPDATA%\carronade\config.toml`.

// No console window flashes up when GlazeWM starts it. Piped stdin and stdout still reach it.
#![windows_subsystem = "windows"]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use carronade::apps::{self, App};
use carronade::config::{self, Config};
use carronade::error::Error;
use carronade::files;
use carronade::history;
use carronade::menu::Choice;
use carronade::picker::{Action, Picture, Row, Step, browse, pick};
use carronade::system::{self, Command};
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
    Apps,
    Files,
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
        "apps" => Mode::Apps,
        "files" => Mode::Files,
        _ => return Err(Error::Usage(args)),
    };
    let config = config::load(&path.map_or_else(config::path, Ok)?)?;
    match mode {
        Mode::Dmenu => dmenu(config),
        Mode::Apps => search(config, Kind::Apps),
        Mode::Files => search(config, Kind::Files),
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

/// The two lists apps and files switch between in one window.
#[derive(Clone, Copy)]
enum Kind {
    Apps,
    Files,
}

impl Kind {
    fn other(self) -> Kind {
        match self {
            Kind::Apps => Kind::Files,
            Kind::Files => Kind::Apps,
        }
    }
}

#[derive(Clone)]
enum Item {
    App(App),
    Command(Command),
    Entry(files::Entry),
}

impl Row for Item {
    fn label(&self) -> &str {
        match self {
            Item::App(app) => app.label(),
            Item::Command(command) => command.label(),
            Item::Entry(entry) => entry.label(),
        }
    }

    fn icon(&self) -> Option<Picture> {
        match self {
            Item::App(app) => app.icon(),
            Item::Command(command) => command.icon(),
            Item::Entry(entry) => entry.icon(),
        }
    }
}

/// What a search needs from the config, which the picker takes.
struct Settings {
    apps_cache: bool,
    files_cache: bool,
    roots: Vec<PathBuf>,
    terminal: String,
    apps_icon: String,
    files_icon: String,
    recent_apps: Vec<String>,
    recent_files: Vec<String>,
}

impl Settings {
    /// The icon that switches away from `kind`.
    fn switch(&self, kind: Kind) -> String {
        match kind {
            Kind::Apps => self.files_icon.clone(),
            Kind::Files => self.apps_icon.clone(),
        }
    }
}

/// Each list, found the first time the picker shows it.
struct Found {
    apps: Option<Vec<App>>,
    files: Option<Vec<files::Entry>>,
}

impl Found {
    fn items(&mut self, kind: Kind, settings: &Settings) -> Result<Vec<Item>, Error> {
        Ok(match kind {
            Kind::Apps => {
                if self.apps.is_none() {
                    let found = cached(
                        settings.apps_cache,
                        apps::cache_path,
                        apps::load,
                        apps::list,
                    )?;
                    self.apps = Some(history::by_recent(found, &settings.recent_apps, app_id));
                }
                let apps = self.apps.iter().flatten().cloned().map(Item::App);
                apps.chain(Command::ALL.map(Item::Command)).collect()
            }
            Kind::Files => {
                if self.files.is_none() {
                    let list = || files::list(&settings.roots);
                    let found = cached(settings.files_cache, files::cache_path, files::load, list)?;
                    let recent = &settings.recent_files;
                    self.files = Some(history::by_recent(found, recent, entry_path));
                }
                self.files
                    .iter()
                    .flatten()
                    .cloned()
                    .map(Item::Entry)
                    .collect()
            }
        })
    }
}

fn app_id(app: &App) -> &str {
    &app.id
}

fn entry_path(entry: &files::Entry) -> &str {
    &entry.path
}

/// What `list` found last time when `cache` is on and there was a last time, else what it finds now.
fn cached<T>(
    cache: bool,
    path: fn() -> Result<PathBuf, Error>,
    load: fn(&Path) -> Result<Option<Vec<T>>, Error>,
    list: impl Fn() -> Result<Vec<T>, Error>,
) -> Result<Vec<T>, Error> {
    let last = match cache {
        true => load(&path()?)?,
        false => None,
    };
    last.map_or_else(list, Ok)
}

/// What a search ends with.
enum Picked {
    Open(Choice<Item>),
    /// Ctrl+Enter on a file or folder.
    Terminal(files::Entry),
    /// Ctrl+Shift+Enter on an app.
    Admin(App),
}

/// Opens the pick from apps and files, starting on `start`: launches an app, elevated or not, runs a system command,
/// opens a file or folder, recording opened apps and entries as recent, starts the terminal in an entry's folder, or runs the typed text as
/// the Run dialog would. Returns whether there was a pick, which a declined UAC prompt is not.
fn search(config: Config, start: Kind) -> Result<bool, Error> {
    let apps_history = history::apps_path()?;
    let files_history = history::files_path()?;
    let settings = Settings {
        apps_cache: config.apps.cache,
        files_cache: config.files.cache,
        roots: config.files.roots.clone(),
        terminal: config.files.terminal.clone(),
        apps_icon: config.input.apps_icon.clone(),
        files_icon: config.input.files_icon.clone(),
        recent_apps: history::load(&apps_history)?,
        recent_files: history::load(&files_history)?,
    };
    let mut found = Found {
        apps: None,
        files: None,
    };
    let mut kind = start;
    let first = found.items(kind, &settings)?;
    let choice = browse(config, first, Some(settings.switch(kind)), |action| {
        Ok(match action {
            Action::Pick(choice) => Step::Done(Picked::Open(choice)),
            Action::Switch => {
                kind = kind.other();
                Step::Show {
                    items: found.items(kind, &settings)?,
                    switch: settings.switch(kind),
                }
            }
            Action::Terminal(Item::Entry(entry)) => Step::Done(Picked::Terminal(entry)),
            Action::Terminal(Item::App(_) | Item::Command(_)) => Step::Stay,
            Action::Admin(Item::App(app)) => Step::Done(Picked::Admin(app)),
            Action::Admin(Item::Command(_) | Item::Entry(_)) => Step::Stay,
        })
    })?;
    let picked = match choice {
        Picked::Open(Choice::Item(Item::App(app))) => {
            apps::launch(&app.target())?;
            history::save(
                &apps_history,
                history::launched(&settings.recent_apps, &app.id),
            )?;
            true
        }
        Picked::Admin(app) => {
            let launched = apps::launch_as_admin(&app.target())?;
            if launched {
                history::save(
                    &apps_history,
                    history::launched(&settings.recent_apps, &app.id),
                )?;
            }
            launched
        }
        Picked::Open(Choice::Item(Item::Command(command))) => {
            system::run(command)?;
            true
        }
        Picked::Open(Choice::Item(Item::Entry(entry))) => {
            apps::launch(&entry.path)?;
            history::save(
                &files_history,
                history::launched(&settings.recent_files, &entry.path),
            )?;
            true
        }
        Picked::Open(Choice::Text(text)) => {
            apps::launch(&text)?;
            true
        }
        Picked::Open(Choice::Cancel) => false,
        Picked::Terminal(entry) => {
            apps::launch_in(&settings.terminal, &files::folder(Path::new(&entry.path))?)?;
            true
        }
    };
    // After the picker closes, since listing beside it slowed its startup by tens of ms.
    if settings.apps_cache && found.apps.is_some() {
        apps::save(&apps::cache_path()?, &apps::list()?)?;
    }
    if settings.files_cache && found.files.is_some() {
        files::save(&files::cache_path()?, &files::list(&settings.roots)?)?;
    }
    Ok(picked)
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
