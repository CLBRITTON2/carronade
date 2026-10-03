//! How often and how lately each app was launched and each entry opened, which ranks them in their lists.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::store;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    used: Vec<Use>,
}

/// The history carronade 0.3.0 and earlier saved: keys, most recent first, without counts.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Order {
    launched: Vec<String>,
}

/// The uses of one app or entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Use {
    /// An `App::id` in the apps history, an `Entry::path` in the files one.
    key: String,
    count: u32,
    /// Seconds since the Unix epoch.
    last: u64,
}

/// `%LOCALAPPDATA%\carronade\history.toml`, the apps launched.
pub fn apps_path() -> Result<PathBuf, Error> {
    store::path("history.toml")
}

/// `%LOCALAPPDATA%\carronade\files-history.toml`, the entries files opened.
pub fn files_path() -> Result<PathBuf, Error> {
    store::path("files-history.toml")
}

/// The format version of the history files, raised when `Use` changes shape.
const VERSION: i64 = 1;

/// The uses `save` wrote to `path`, empty before the first. A history of 0.3.0 or earlier loads as one use per key, a
/// second apart before `now` in its order.
pub fn load(path: &Path, now: u64) -> Result<Vec<Use>, Error> {
    // 0.3.0's history and this format's first saves carry no version, so only the shape tells them apart.
    match store::load::<History>(path, VERSION) {
        Ok(history) => Ok(history.map_or_else(Vec::new, |history| history.used)),
        Err(error @ Error::StoreParse { .. }) => match store::load::<Order>(path, VERSION) {
            Ok(Some(order)) => Ok(counted(order, now)),
            Ok(None) | Err(_) => Err(error),
        },
        Err(error) => Err(error),
    }
}

fn counted(order: Order, now: u64) -> Vec<Use> {
    (0..)
        .zip(order.launched)
        .map(|(age, key)| Use {
            key,
            count: 1,
            last: now.saturating_sub(age),
        })
        .collect()
}

pub fn save(path: &Path, used: Vec<Use>) -> Result<(), Error> {
    store::save(path, VERSION, &History { used })
}

/// `uses` with one more use of `key` at `now`.
pub fn used(uses: &[Use], key: &str, now: u64) -> Vec<Use> {
    let count = uses
        .iter()
        .find(|other| other.key == key)
        .map_or(0, |other| other.count);
    let this = Use {
        key: key.to_owned(),
        count: count.saturating_add(1),
        last: now,
    };
    std::iter::once(this)
        .chain(uses.iter().filter(|other| other.key != key).cloned())
        .collect()
}

/// Score added to a match per doubling of its frecency, so a daily favorite gains about what a well-placed letter
/// earns and never outranks a much better match.
const BOOST: i32 = 4;

/// The score each key's uses add to its matches by `now`: `BOOST` per doubling of zoxide's frecency, the count
/// weighted by the age of the last use (https://github.com/ajeetdsouza/zoxide/wiki/Algorithm).
pub fn boosts(uses: &[Use], now: u64) -> HashMap<String, i32> {
    uses.iter()
        .map(|this| {
            let age = now.saturating_sub(this.last);
            let weight = match age {
                0..3_600 => 16,
                3_600..86_400 => 8,
                86_400..604_800 => 2,
                _ => 1,
            };
            let frecency = this.count.saturating_mul(weight);
            let doublings = frecency.saturating_add(1).ilog2() as i32;
            (this.key.clone(), BOOST * doublings)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 10_000_000;

    fn one(key: &str, count: u32, last: u64) -> Use {
        Use {
            key: key.to_owned(),
            count,
            last,
        }
    }

    #[test]
    fn a_use_counts_once_more_and_moves_to_the_front() {
        let uses = [one("a", 2, 10), one("b", 5, 20)];
        assert_eq!(used(&uses, "b", NOW), [one("b", 6, NOW), one("a", 2, 10)]);
        assert_eq!(
            used(&uses, "new", NOW),
            [one("new", 1, NOW), one("a", 2, 10), one("b", 5, 20)]
        );
    }

    #[test]
    fn frequent_and_recent_uses_boost_more() {
        let uses = [
            one("daily", 30, NOW - 60),
            one("once now", 1, NOW),
            one("once last month", 1, NOW - 2_592_000),
        ];
        let boosts = boosts(&uses, NOW);
        assert_eq!(boosts.get("daily"), Some(&(BOOST * 8)));
        assert_eq!(boosts.get("once now"), Some(&(BOOST * 4)));
        assert_eq!(boosts.get("once last month"), Some(&BOOST));
    }

    #[test]
    fn an_old_order_counts_one_use_each_in_its_order() {
        let order = Order {
            launched: ["a", "b"].map(String::from).into(),
        };
        assert_eq!(
            counted(order, NOW),
            [one("a", 1, NOW), one("b", 1, NOW - 1)]
        );
    }
}
