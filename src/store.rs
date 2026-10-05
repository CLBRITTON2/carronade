//! The TOML files carronade keeps between runs, in `%LOCALAPPDATA%\carronade`.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};
use windows::Win32::UI::Shell::FOLDERID_LocalAppData;

use crate::error::Error;
use crate::platform::known_folder;

/// `%LOCALAPPDATA%\carronade\<name>`.
pub(crate) fn path(name: &str) -> Result<PathBuf, Error> {
    Ok(known_folder(&FOLDERID_LocalAppData)?
        .join("carronade")
        .join(name))
}

/// Seconds since the Unix epoch, the time stamps the stored files hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnixSeconds(pub u64);

impl UnixSeconds {
    /// The seconds from `earlier` to this, none when `earlier` is later.
    #[must_use]
    pub fn since(self, earlier: UnixSeconds) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    /// `seconds` before this, the epoch at the earliest.
    #[must_use]
    pub fn before(self, seconds: u64) -> UnixSeconds {
        UnixSeconds(self.0.saturating_sub(seconds))
    }
}

pub fn now() -> Result<UnixSeconds, Error> {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    Ok(UnixSeconds(since.map_err(Error::Clock)?.as_secs()))
}

/// The top-level key holding a file's format version, which `T` in `load` and `save` never sees.
const VERSION: &str = "version";

/// What `save` wrote to `path` at `version`, or `None` before the first save. A file without a version, as 0.3.0 and
/// earlier wrote, loads as `version`.
pub(crate) fn load<T: DeserializeOwned>(path: &Path, version: i64) -> Result<Option<T>, Error> {
    let Some(bytes) = read(path)? else {
        return Ok(None);
    };
    let text = String::from_utf8(bytes).map_err(|source| Error::StoreRead {
        path: path.to_owned(),
        source: std::io::Error::new(ErrorKind::InvalidData, source),
    })?;
    let parse = |source| Error::StoreParse {
        path: path.to_owned(),
        source: Box::new(source),
    };
    let mut table: Table = toml::from_str(&text).map_err(parse)?;
    match table.remove(VERSION) {
        None => {}
        Some(Value::Integer(found)) if found == version => {}
        Some(found) => {
            return Err(Error::StoreVersion {
                path: path.to_owned(),
                found: found.to_string(),
                expected: version,
            });
        }
    }
    table.try_into().map(Some).map_err(parse)
}

/// Writes `value` at `version` for `load`, through a temporary file so a reader never sees half of it.
pub(crate) fn save<T: Serialize>(path: &Path, version: i64, value: &T) -> Result<(), Error> {
    let serialize = |source| Error::StoreSerialize {
        path: path.to_owned(),
        source,
    };
    let mut table = Table::try_from(value).map_err(serialize)?;
    table.insert(VERSION.to_owned(), Value::Integer(version));
    let text = toml::to_string(&table).map_err(serialize)?;
    write(path, text.as_bytes())
}

/// The bytes `write` wrote to `path`, or `None` before the first write.
pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, Error> {
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
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut temporary = path.as_os_str().to_owned();
    // Per process: two carronades saving at once would otherwise write, and rename, the same file.
    temporary.push(format!(".{}.tmp", std::process::id()));
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
