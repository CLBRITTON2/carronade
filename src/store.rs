//! The TOML files carronade keeps between runs, in `%LOCALAPPDATA%\carronade`.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use windows::Win32::UI::Shell::FOLDERID_LocalAppData;

use crate::config::known_folder;
use crate::error::Error;

/// `%LOCALAPPDATA%\carronade\<name>`.
pub fn path(name: &str) -> Result<PathBuf, Error> {
    Ok(known_folder(&FOLDERID_LocalAppData)?
        .join("carronade")
        .join(name))
}

/// Seconds since the Unix epoch, the time stamps the stored files hold.
pub fn now() -> Result<u64, Error> {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    Ok(since.map_err(Error::Clock)?.as_secs())
}

/// What `save` wrote to `path`, or `None` before the first save.
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, Error> {
    let Some(bytes) = read(path)? else {
        return Ok(None);
    };
    let text = String::from_utf8(bytes).map_err(|source| Error::StoreRead {
        path: path.to_owned(),
        source: std::io::Error::new(ErrorKind::InvalidData, source),
    })?;
    toml::from_str(&text)
        .map(Some)
        .map_err(|source| Error::StoreParse {
            path: path.to_owned(),
            source: Box::new(source),
        })
}

/// Writes `value` for `load`, through a temporary file so a reader never sees half of it.
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let text = toml::to_string(value).map_err(|source| Error::StoreSerialize {
        path: path.to_owned(),
        source,
    })?;
    write(path, text.as_bytes())
}

/// The bytes `write` wrote to `path`, or `None` before the first write.
pub fn read(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::StoreRead {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Writes `bytes` to `path` through a temporary file so a reader never sees half of them.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let write = |source| Error::StoreWrite {
        path: path.to_owned(),
        source,
    };
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(write)?;
    }
    std::fs::write(&temporary, bytes).map_err(write)?;
    std::fs::rename(&temporary, path).map_err(write)
}
