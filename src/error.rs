use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("reading the config {path:?} failed")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the config {path:?} is invalid")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("the font family {0:?} is not installed")]
    Font(String),
    #[error("loading the image {path:?} failed")]
    Image {
        path: PathBuf,
        #[source]
        source: windows::core::Error,
    },
    #[error("reading items from stdin failed")]
    Stdin(#[source] std::io::Error),
    #[error("writing the selection to stdout failed")]
    Stdout(#[source] std::io::Error),
    #[error("{call} failed")]
    Win32 {
        call: &'static str,
        #[source]
        source: windows::core::Error,
    },
    #[error("text is not valid UTF-16")]
    Utf16(#[from] std::string::FromUtf16Error),
    #[error("launching {target:?} failed")]
    Launch {
        target: String,
        #[source]
        source: windows::core::Error,
    },
    #[error("launching {program:?} in {folder:?} failed")]
    LaunchIn {
        program: String,
        folder: PathBuf,
        #[source]
        source: windows::core::Error,
    },
    #[error("reading the attributes of {path:?} failed")]
    Attributes {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path:?} is a file with no folder above it")]
    NoParent { path: PathBuf },
    #[error("loading the icon of {target:?} failed")]
    Icon {
        target: String,
        #[source]
        source: windows::core::Error,
    },
    #[error("the shell's icon of {target:?} is not a 32-bit bitmap")]
    IconBitmap { target: String },
    #[error("reading {path:?} failed")]
    StoreRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path:?} is invalid, delete it to start it over")]
    StoreParse {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error(
        "{path:?} has version {found}, this carronade reads version {expected}, delete it to start it over"
    )]
    StoreVersion {
        path: PathBuf,
        found: String,
        expected: i64,
    },
    #[error("the system clock is before 1970")]
    Clock(#[source] std::time::SystemTimeError),
    #[error("{path:?} is not an icon cache carronade wrote, delete it to start it over")]
    IconCache { path: PathBuf },
    #[error("writing {path:?} failed")]
    StoreWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("encoding {path:?} failed")]
    StoreSerialize {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },
    #[error("listing the files below {root:?} failed")]
    Walk {
        root: PathBuf,
        #[source]
        source: ignore::Error,
    },
    #[error("listing {root:?} found {path:?}, which is not below it")]
    OutsideRoot { root: PathBuf, path: PathBuf },
    #[error("the name of {path:?} is not valid Unicode")]
    FileName { path: PathBuf },
    #[error("Windows refused to bring the picker to the foreground")]
    Foreground,
    #[error("the picker window got a message before its state was set")]
    NoState,
    #[error("the picker window got a message while handling another")]
    Reentered,
    #[error("the picker's message loop ended without a choice")]
    NoChoice,
    #[error("the picker chose row {row} of {len} items")]
    Row { row: usize, len: usize },
}

/// Maps a failed Win32 call to `Error::Win32`, for `map_err`.
pub fn win32(call: &'static str) -> impl FnOnce(windows::core::Error) -> Error {
    move |source| Error::Win32 { call, source }
}

/// The calling thread's last Win32 error, for calls that report failure only through their return value.
pub fn last(call: &'static str) -> Error {
    win32(call)(windows::core::Error::from_thread())
}
