//! The TOML files drun keeps between runs, in `%LOCALAPPDATA%\carronade`.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

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

/// What `save` wrote to `path`, or `None` before the first save.
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::StoreRead {
                path: path.to_owned(),
                source,
            });
        }
    };
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
    let temporary = path.with_extension("toml.tmp");
    let write = |source| Error::StoreWrite {
        path: path.to_owned(),
        source,
    };
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(write)?;
    }
    std::fs::write(&temporary, text).map_err(write)?;
    std::fs::rename(&temporary, path).map_err(write)
}
