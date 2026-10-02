//! The apps drun launched and the entries files opened, most recent first, so they lead their lists.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::store;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    /// `App::id`s in the apps history, `Entry::path`s in the files one.
    launched: Vec<String>,
}

/// `%LOCALAPPDATA%\carronade\history.toml`, the apps drun launched.
pub fn apps_path() -> Result<PathBuf, Error> {
    store::path("history.toml")
}

/// `%LOCALAPPDATA%\carronade\files-history.toml`, the entries files opened.
pub fn files_path() -> Result<PathBuf, Error> {
    store::path("files-history.toml")
}

/// The keys `save` wrote to `path`, most recent first. Empty before the first launch.
pub fn load(path: &Path) -> Result<Vec<String>, Error> {
    Ok(store::load::<History>(path)?.map_or_else(Vec::new, |history| history.launched))
}

pub fn save(path: &Path, launched: Vec<String>) -> Result<(), Error> {
    store::save(path, &History { launched })
}

/// `recent` with `key` moved to the front.
pub fn launched(recent: &[String], key: &str) -> Vec<String> {
    std::iter::once(key.to_owned())
        .chain(recent.iter().filter(|other| *other != key).cloned())
        .collect()
}

/// `items` with the ones whose `key` is in `recent` first, in its order, and the rest after in their own.
pub fn by_recent<T>(items: Vec<T>, recent: &[String], key: fn(&T) -> &str) -> Vec<T> {
    let mut items = items;
    items.sort_by_cached_key(|item| {
        recent
            .iter()
            .position(|other| other == key(item))
            .unwrap_or(recent.len())
    });
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(items: &[String]) -> Vec<&str> {
        items.iter().map(String::as_str).collect()
    }

    #[test]
    fn recent_items_lead_in_launch_order_and_the_rest_keep_theirs() {
        let items = ["a", "b", "c", "d", "e"].map(String::from).into();
        let recent = ["d", "gone", "b"].map(String::from);
        let sorted = by_recent(items, &recent, String::as_str);
        assert_eq!(keys(&sorted), ["d", "b", "a", "c", "e"]);
    }

    #[test]
    fn a_launch_moves_the_key_to_the_front_once() {
        let recent = ["a", "b", "c"].map(String::from);
        assert_eq!(launched(&recent, "b"), ["b", "a", "c"]);
        assert_eq!(launched(&recent, "new"), ["new", "a", "b", "c"]);
    }
}
