#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("usage: carronade <dmenu|drun>, got {0:?}")]
    Usage(Vec<String>),
    #[error("reading items from stdin failed: {0}")]
    Stdin(#[source] std::io::Error),
    #[error("writing the selection to stdout failed: {0}")]
    Stdout(#[source] std::io::Error),
    #[error("{call} failed: {source}")]
    Win32 {
        call: &'static str,
        #[source]
        source: windows::core::Error,
    },
    #[error("text is not valid UTF-16: {0}")]
    Utf16(#[from] std::string::FromUtf16Error),
    #[error("launching {target:?} failed: {source}")]
    Launch {
        target: String,
        #[source]
        source: windows::core::Error,
    },
    #[error("Windows refused to bring the picker to the foreground")]
    Foreground,
    #[error("the picker window got a message before its state was set")]
    NoState,
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
