//! The apps drun launched, most recent first, so they lead its list.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::apps::App;
use crate::error::Error;
use crate::store;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    /// `App::id`s.
    launched: Vec<String>,
}

/// `%LOCALAPPDATA%\carronade\history.toml`.
pub fn path() -> Result<PathBuf, Error> {
    store::path("history.toml")
}

/// The app ids `save` wrote to `path`, most recent first. Empty before the first launch.
pub fn load(path: &Path) -> Result<Vec<String>, Error> {
    Ok(store::load::<History>(path)?.map_or_else(Vec::new, |history| history.launched))
}

pub fn save(path: &Path, launched: Vec<String>) -> Result<(), Error> {
    store::save(path, &History { launched })
}

/// `recent` with `id` moved to the front.
pub fn launched(recent: &[String], id: &str) -> Vec<String> {
    std::iter::once(id.to_owned())
        .chain(recent.iter().filter(|other| *other != id).cloned())
        .collect()
}

/// `apps` with the ones in `recent` first, in its order, and the rest after in their own.
pub fn by_recent(apps: Vec<App>, recent: &[String]) -> Vec<App> {
    let mut apps = apps;
    apps.sort_by_key(|app| {
        recent
            .iter()
            .position(|id| *id == app.id)
            .unwrap_or(recent.len())
    });
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str) -> App {
        App {
            name: id.to_uppercase(),
            id: id.to_owned(),
        }
    }

    fn ids(apps: &[App]) -> Vec<&str> {
        apps.iter().map(|app| app.id.as_str()).collect()
    }

    #[test]
    fn recent_apps_lead_in_launch_order_and_the_rest_keep_theirs() {
        let apps = ["a", "b", "c", "d", "e"].map(app).into();
        let recent = ["d", "gone", "b"].map(String::from);
        assert_eq!(ids(&by_recent(apps, &recent)), ["d", "b", "a", "c", "e"]);
    }

    #[test]
    fn a_launch_moves_the_app_to_the_front_once() {
        let recent = ["a", "b", "c"].map(String::from);
        assert_eq!(launched(&recent, "b"), ["b", "a", "c"]);
        assert_eq!(launched(&recent, "new"), ["new", "a", "b", "c"]);
    }
}
