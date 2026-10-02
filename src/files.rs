//! The files and folders the files mode searches.

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::picker::{Picture, Row};
use crate::store;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The path below its root, which the query matches against.
    pub label: String,
    pub path: String,
    depth: usize,
}

impl Row for Entry {
    fn label(&self) -> &str {
        &self.label
    }

    fn icon(&self) -> Option<Picture> {
        Some(Picture::Shell(self.path.clone()))
    }
}

/// Every file and folder below `roots`, shallowest first, then by label. It leaves out what git ignores and hidden
/// entries, as `rg --files` does.
pub fn list(roots: &[PathBuf]) -> Result<Vec<Entry>, Error> {
    let mut entries = Vec::new();
    for root in roots {
        entries.extend(below(root)?);
    }
    entries.sort_by_key(|entry| (entry.depth, entry.label.to_lowercase()));
    Ok(entries)
}

/// `path` when it is a folder, else the folder holding it.
pub fn folder(path: &Path) -> Result<PathBuf, Error> {
    let metadata = std::fs::metadata(path).map_err(|source| Error::Attributes {
        path: path.to_owned(),
        source,
    })?;
    match metadata.is_dir() {
        true => Ok(path.to_owned()),
        false => path
            .parent()
            .map(Path::to_owned)
            .ok_or_else(|| Error::NoParent {
                path: path.to_owned(),
            }),
    }
}

/// TOML needs a table at the top, so the entries sit in an `[[entry]]` array.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    entry: Vec<Entry>,
}

#[derive(Serialize)]
struct CacheRef<'a> {
    entry: &'a [Entry],
}

/// `%LOCALAPPDATA%\carronade\files.toml`, the entries the files mode found last time.
pub fn cache_path() -> Result<PathBuf, Error> {
    store::path("files.toml")
}

/// The entries `save` wrote to `path`, or `None` before the first save.
pub fn load(path: &Path) -> Result<Option<Vec<Entry>>, Error> {
    Ok(store::load::<Cache>(path)?.map(|cache| cache.entry))
}

pub fn save(path: &Path, entries: &[Entry]) -> Result<(), Error> {
    store::save(path, &CacheRef { entry: entries })
}

fn below(root: &Path) -> Result<Vec<Entry>, Error> {
    let mut entries = Vec::new();
    for found in WalkBuilder::new(root).build() {
        let found = found.map_err(|source| Error::Walk {
            root: root.to_owned(),
            source,
        })?;
        if found.depth() == 0 {
            continue;
        }
        let path = found.path();
        let unicode = |part: &Path| {
            part.to_str()
                .map(str::to_owned)
                .ok_or_else(|| Error::FileName {
                    path: path.to_owned(),
                })
        };
        let relative = path.strip_prefix(root).map_err(|_| Error::OutsideRoot {
            root: root.to_owned(),
            path: path.to_owned(),
        })?;
        entries.push(Entry {
            label: unicode(relative)?,
            path: unicode(path)?,
            depth: found.depth(),
        });
    }
    Ok(entries)
}
