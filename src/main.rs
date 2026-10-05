//! `carronade dmenu` prints the stdin line picked, `carronade apps` launches the Start menu app picked, and
//! `carronade files` opens the file or folder picked from below `files.roots`. apps and files switch to each other in
//! the same window. Each exits 1 on cancel and 2 on error. `--config <path>` replaces
//! `%APPDATA%\carronade\config.toml`.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use carronade::apps::{self, App};
use carronade::config::{self, Config};
use carronade::error::Error;
use carronade::files;
use carronade::history::{self, Use};
use carronade::menu::Choice;
use carronade::picker::{Action, Picture, Row, Step, browse, pick};
use carronade::shell;
use carronade::store::{self, UnixSeconds};
use carronade::system::{self, Command};
use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
use windows::core::{HSTRING, w};

fn main() -> ExitCode {
    match parse(std::env::args().skip(1).collect()) {
        Ok(Request::Help) => print("help", &help()),
        Ok(Request::Version) => print("version", VERSION),
        Ok(Request::Run { config, mode }) => match run(config.as_deref(), mode) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(error) => fail(&error),
        },
        Err(usage) => fail(&usage),
    }
}

fn fail(error: &dyn std::error::Error) -> ExitCode {
    report(error);
    ExitCode::from(2)
}

/// Writes `text`, the `what` asked for, to stdout.
fn print(what: &'static str, text: &str) -> ExitCode {
    match writeln!(std::io::stdout(), "{text}") {
        Ok(()) => ExitCode::SUCCESS,
        Err(source) => fail(&Error::Print { what, source }),
    }
}

const VERSION: &str = concat!("carronade ", env!("CARGO_PKG_VERSION"));

/// The help below its version and description lines, which a missing mode also prints.
const USAGE: &str = "\
Usage: carronade [--config <path>] <mode>

Modes:
  apps   Launch a Start menu app, or lock, sign out, hibernate, restart or shut down
  files  Open a file or folder found below files.roots
  dmenu  Print the line picked from the lines read on stdin

Tab switches between apps and files in the same window.

Options:
  --config <path>  Read the config from <path> instead of %APPDATA%\\carronade\\config.toml
  -h, --help       Print this help
  -V, --version    Print the version

Exits 0 when something is picked, 1 on cancel and 2 on error.";

fn help() -> String {
    format!("{VERSION}\n{}.\n\n{USAGE}", env!("CARGO_PKG_DESCRIPTION"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Dmenu,
    Apps,
    Files,
}

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    Help,
    Version,
    Run { config: Option<PathBuf>, mode: Mode },
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum Usage {
    #[error("a mode is required\n\n{}", USAGE)]
    Missing,
    #[error("--config needs a path\n\nRun carronade --help for usage.")]
    NoConfigPath,
    #[error("{0:?} is not a mode, use apps, files or dmenu\n\nRun carronade --help for usage.")]
    NotAMode(String),
    #[error("unexpected arguments {0:?}\n\nRun carronade --help for usage.")]
    Unexpected(Vec<String>),
}

fn parse(args: Vec<String>) -> Result<Request, Usage> {
    match args.as_slice() {
        [] => Err(Usage::Missing),
        [flag] if flag == "-h" || flag == "--help" => Ok(Request::Help),
        [flag] if flag == "-V" || flag == "--version" => Ok(Request::Version),
        [flag] if flag == "--config" => Err(Usage::NoConfigPath),
        [flag, _] if flag == "--config" => Err(Usage::Missing),
        [mode] => Ok(Request::Run {
            config: None,
            mode: mode_named(mode)?,
        }),
        [flag, path, mode] if flag == "--config" => Ok(Request::Run {
            config: Some(PathBuf::from(path)),
            mode: mode_named(mode)?,
        }),
        _ => Err(Usage::Unexpected(args)),
    }
}

fn mode_named(word: &str) -> Result<Mode, Usage> {
    match word {
        "apps" => Ok(Mode::Apps),
        "files" => Ok(Mode::Files),
        "dmenu" => Ok(Mode::Dmenu),
        _ => Err(Usage::NotAMode(word.to_owned())),
    }
}

/// Runs `mode` with the config at `path`, or the default one, returning whether something was picked.
fn run(path: Option<&Path>, mode: Mode) -> Result<bool, Error> {
    let config = config::load(&path.map_or_else(config::path, |path| Ok(path.to_owned()))?)?;
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
#[derive(Clone, Copy, Debug)]
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

/// An app or entry with the boost its history gives it, or a system command, which has none.
#[derive(Clone, Debug)]
enum Item {
    App(App, i32),
    Command(Command),
    Entry(files::Entry, i32),
}

impl Row for Item {
    fn label(&self) -> &str {
        match self {
            Item::App(app, _) => app.label(),
            Item::Command(command) => command.label(),
            Item::Entry(entry, _) => entry.label(),
        }
    }

    fn alias(&self) -> Option<&str> {
        match self {
            Item::App(app, _) => app.alias(),
            Item::Command(command) => command.alias(),
            Item::Entry(entry, _) => entry.alias(),
        }
    }

    fn icon(&self) -> Option<Picture> {
        match self {
            Item::App(app, _) => app.icon(),
            Item::Command(command) => command.icon(),
            Item::Entry(entry, _) => entry.icon(),
        }
    }

    fn boost(&self) -> i32 {
        match self {
            Item::App(_, boost) | Item::Entry(_, boost) => *boost,
            Item::Command(command) => command.boost(),
        }
    }
}

/// What a search needs from the config, which the picker takes, and from the histories: their uses and boosts.
#[derive(Debug)]
struct Settings {
    apps_cache: bool,
    files_cache: bool,
    roots: Vec<PathBuf>,
    terminal: String,
    apps_icon: String,
    files_icon: String,
    now: UnixSeconds,
    recent_apps: Vec<Use>,
    recent_files: Vec<Use>,
    app_boosts: HashMap<String, i32>,
    file_boosts: HashMap<String, i32>,
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
#[derive(Debug)]
struct Found {
    apps: Option<Listing<App>>,
    files: Option<Listing<files::Entry>>,
}

/// A list and where it came from.
#[derive(Debug)]
enum Listing<T> {
    /// Read from its cache, so possibly stale.
    Cached(Vec<T>),
    Listed(Vec<T>),
}

impl<T> Listing<T> {
    fn new(
        cached: Option<Vec<T>>,
        list: impl FnOnce() -> Result<Vec<T>, Error>,
    ) -> Result<Self, Error> {
        Ok(match cached {
            Some(items) => Listing::Cached(items),
            None => Listing::Listed(list()?),
        })
    }

    fn items(&self) -> &[T] {
        match self {
            Listing::Cached(items) | Listing::Listed(items) => items,
        }
    }
}

/// Saves a list the picker showed: listed again when it came from the cache, as it is when it was just listed.
fn refresh<T>(
    shown: Option<&Listing<T>>,
    list: impl FnOnce() -> Result<Vec<T>, Error>,
    save: impl FnOnce(&[T]) -> Result<(), Error>,
) -> Result<(), Error> {
    match shown {
        Some(Listing::Cached(_)) => save(&list()?),
        Some(Listing::Listed(items)) => save(items),
        None => Ok(()),
    }
}

impl Found {
    fn items(&mut self, kind: Kind, settings: &Settings) -> Result<Vec<Item>, Error> {
        Ok(match kind {
            Kind::Apps => {
                if self.apps.is_none() {
                    let last = if settings.apps_cache {
                        apps::load(&apps::cache_path()?)?
                    } else {
                        None
                    };
                    self.apps = Some(Listing::new(last, apps::list)?);
                }
                let apps = self
                    .apps
                    .iter()
                    .flat_map(Listing::items)
                    .map(|app| Item::App(app.clone(), boost(&settings.app_boosts, &app.id)));
                apps.chain(Command::ALL.map(Item::Command)).collect()
            }
            Kind::Files => {
                if self.files.is_none() {
                    let last = if settings.files_cache {
                        files::load(&files::cache_path()?)?
                    } else {
                        None
                    };
                    self.files = Some(Listing::new(last, || files::list(&settings.roots))?);
                }
                self.files
                    .iter()
                    .flat_map(Listing::items)
                    .map(|entry| {
                        Item::Entry(entry.clone(), boost(&settings.file_boosts, &entry.path))
                    })
                    .collect()
            }
        })
    }
}

/// The boost of `key` in `boosts`, none for a key never used.
fn boost(boosts: &HashMap<String, i32>, key: &str) -> i32 {
    boosts.get(key).copied().unwrap_or(0)
}

/// What a search ends with.
#[derive(Debug)]
enum Picked {
    Open(Choice<Item>),
    /// Ctrl+Enter on a file or folder.
    Terminal(files::Entry),
    /// Ctrl+Shift+Enter on an app.
    Admin(App),
}

/// Opens the pick from apps and files, starting on `start`: launches an app, elevated or not, runs a system command,
/// opens a file or folder, recording each app and entry opened in its history, starts the terminal in an entry's
/// folder, or runs the typed text as the Run dialog would. Returns whether there was a pick, which a declined UAC
/// prompt is not.
fn search(config: Config, start: Kind) -> Result<bool, Error> {
    let apps_history = history::apps_path()?;
    let files_history = history::files_path()?;
    let now = store::now()?;
    let recent_apps = history::load(&apps_history, now)?;
    let recent_files = history::load(&files_history, now)?;
    let settings = Settings {
        apps_cache: config.apps.cache,
        files_cache: config.files.cache,
        roots: config.files.roots.clone(),
        terminal: config.files.terminal.clone(),
        apps_icon: config.input.apps_icon.clone(),
        files_icon: config.input.files_icon.clone(),
        now,
        app_boosts: history::boosts(&recent_apps, now),
        file_boosts: history::boosts(&recent_files, now),
        recent_apps,
        recent_files,
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
            Action::Terminal(Item::Entry(entry, _)) => Step::Done(Picked::Terminal(entry)),
            Action::Admin(Item::App(app, _)) => Step::Done(Picked::Admin(app)),
            Action::Terminal(Item::App(..) | Item::Command(_))
            | Action::Admin(Item::Command(_) | Item::Entry(..)) => Step::Stay,
        })
    })?;
    let picked = match choice {
        Picked::Open(Choice::Item(Item::App(app, _))) => {
            shell::launch(&app.target())?;
            record_app(&apps_history, &settings, &app)?;
            true
        }
        Picked::Admin(app) => {
            let launched = shell::launch_as_admin(&app.target())?;
            if launched {
                record_app(&apps_history, &settings, &app)?;
            }
            launched
        }
        Picked::Open(Choice::Item(Item::Command(command))) => {
            system::run(command)?;
            true
        }
        Picked::Open(Choice::Item(Item::Entry(entry, _))) => {
            shell::launch(&entry.path)?;
            record_entry(&files_history, &settings, &entry)?;
            true
        }
        Picked::Open(Choice::Text(text)) => {
            shell::launch(&text)?;
            true
        }
        Picked::Open(Choice::Cancel) => false,
        Picked::Terminal(entry) => {
            shell::launch_in(&settings.terminal, &files::folder(Path::new(&entry.path))?)?;
            record_entry(&files_history, &settings, &entry)?;
            true
        }
    };
    // After the picker closes, since listing beside it slowed its startup by tens of ms.
    if settings.apps_cache {
        refresh(found.apps.as_ref(), apps::list, |apps| {
            apps::save(&apps::cache_path()?, apps)
        })?;
    }
    if settings.files_cache {
        refresh(
            found.files.as_ref(),
            || files::list(&settings.roots),
            |entries| files::save(&files::cache_path()?, entries),
        )?;
    }
    Ok(picked)
}

/// Saves the apps history at `path` with one more launch of `app`.
fn record_app(path: &Path, settings: &Settings, app: &App) -> Result<(), Error> {
    history::save(
        path,
        &history::used(&settings.recent_apps, &app.id, settings.now),
    )
}

/// Saves the files history at `path` with one more use of `entry`.
fn record_entry(path: &Path, settings: &Settings, entry: &files::Entry) -> Result<(), Error> {
    history::save(
        path,
        &history::used(&settings.recent_files, &entry.path, settings.now),
    )
}

/// Writes to stderr when the caller gave one, else shows a message box: started from a hotkey, nothing reads stderr.
/// A stderr that fails the write gets the message box too.
fn report(error: &dyn std::error::Error) {
    let text = chain(error);
    // SAFETY: reads this process's standard handle and takes no ownership of it.
    let handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    let has_stderr = handle.is_ok_and(|handle| !handle.is_invalid());
    if has_stderr && writeln!(std::io::stderr(), "carronade: {text}").is_ok() {
        return;
    }
    // SAFETY: the text is a temporary HSTRING and the caption a static wide string, both outliving the call.
    unsafe { MessageBoxW(None, &HSTRING::from(text), w!("carronade"), MB_ICONERROR) };
}

/// `error` and each cause below it, outermost first, joined by `: `.
fn chain(error: &dyn std::error::Error) -> String {
    std::iter::successors(Some(error), |error| error.source())
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Request, Usage> {
        parse(args.iter().map(|&arg| arg.to_owned()).collect())
    }

    #[test]
    fn a_mode_runs_with_or_without_a_config() {
        let run = |config: Option<&str>, mode| Request::Run {
            config: config.map(PathBuf::from),
            mode,
        };
        assert_eq!(parsed(&["apps"]), Ok(run(None, Mode::Apps)));
        assert_eq!(
            parsed(&["--config", "c.toml", "dmenu"]),
            Ok(run(Some("c.toml"), Mode::Dmenu))
        );
    }

    #[test]
    fn help_and_version_take_a_short_and_a_long_flag() {
        for flag in ["-h", "--help"] {
            assert_eq!(parsed(&[flag]), Ok(Request::Help));
        }
        for flag in ["-V", "--version"] {
            assert_eq!(parsed(&[flag]), Ok(Request::Version));
        }
    }

    #[test]
    fn a_bad_command_line_says_what_is_wrong() {
        assert_eq!(parsed(&[]), Err(Usage::Missing));
        assert_eq!(parsed(&["--config", "c.toml"]), Err(Usage::Missing));
        assert_eq!(parsed(&["--config"]), Err(Usage::NoConfigPath));
        assert_eq!(parsed(&["show"]), Err(Usage::NotAMode("show".to_owned())));
        assert_eq!(
            parsed(&["apps", "files"]),
            Err(Usage::Unexpected(vec![
                "apps".to_owned(),
                "files".to_owned()
            ]))
        );
    }
}
