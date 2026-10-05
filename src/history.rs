//! How often and how lately each app was launched and each entry opened, which ranks them in their lists.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::store::{self, UnixSeconds};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    used: Vec<Use>,
}

#[derive(Serialize)]
struct HistoryRef<'a> {
    used: &'a [Use],
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
    last: UnixSeconds,
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
pub fn load(path: &Path, now: UnixSeconds) -> Result<Vec<Use>, Error> {
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

fn counted(order: Order, now: UnixSeconds) -> Vec<Use> {
    (0..)
        .zip(order.launched)
        .map(|(age, key)| Use {
            key,
            count: 1,
            last: now.before(age),
        })
        .collect()
}

pub fn save(path: &Path, used: &[Use]) -> Result<(), Error> {
    store::save(path, VERSION, &HistoryRef { used })
}

/// The most uses a history counts in all before `aged` halves them.
const MAX_TOTAL: u32 = 1_000;

/// `uses` with one more use of `key` at `now`, aged once their counts pass `MAX_TOTAL`.
#[must_use]
pub fn used(uses: &[Use], key: &str, now: UnixSeconds) -> Vec<Use> {
    let count = uses
        .iter()
        .find(|other| other.key == key)
        .map_or(0, |other| other.count);
    let this = Use {
        key: key.to_owned(),
        count: count.saturating_add(1),
        last: now,
    };
    aged(
        std::iter::once(this)
            .chain(uses.iter().filter(|other| other.key != key).cloned())
            .collect(),
    )
}

/// `uses` with every count halved and the keys left at none dropped when the counts total more than `MAX_TOTAL`, as
/// zoxide ages its database, so keys used rarely or long ago fall out and the history stays small.
fn aged(uses: Vec<Use>) -> Vec<Use> {
    let total: u64 = uses.iter().map(|this| u64::from(this.count)).sum();
    if total <= u64::from(MAX_TOTAL) {
        return uses;
    }
    uses.into_iter()
        .map(|this| Use {
            count: this.count / 2,
            ..this
        })
        .filter(|this| this.count > 0)
        .collect()
}

/// Score added to a match per doubling of its frecency, so a daily favorite gains about what a well-placed letter
/// earns and never outranks a much better match.
const BOOST: i32 = 4;

/// The score each key's uses add to its matches by `now`: `BOOST` per doubling of zoxide's frecency, the count
/// weighted by the age of the last use (<https://github.com/ajeetdsouza/zoxide/wiki/Algorithm>).
#[must_use]
pub fn boosts(uses: &[Use], now: UnixSeconds) -> HashMap<String, i32> {
    uses.iter()
        .map(|this| {
            let age = now.since(this.last);
            let weight = match age {
                0..3_600 => 16,
                3_600..86_400 => 8,
                86_400..604_800 => 2,
                _ => 1,
            };
            let frecency = this.count.saturating_mul(weight);
            // The log2 of a u32 is below 32.
            let doublings = frecency.saturating_add(1).ilog2() as i32;
            (this.key.clone(), BOOST * doublings)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: UnixSeconds = UnixSeconds(10_000_000);

    fn one(key: &str, count: u32, last: UnixSeconds) -> Use {
        Use {
            key: key.to_owned(),
            count,
            last,
        }
    }

    fn at(seconds: u64) -> UnixSeconds {
        UnixSeconds(seconds)
    }

    #[test]
    fn a_use_counts_once_more_and_moves_to_the_front() {
        let uses = [one("a", 2, at(10)), one("b", 5, at(20))];
        assert_eq!(
            used(&uses, "b", NOW),
            [one("b", 6, NOW), one("a", 2, at(10))]
        );
        assert_eq!(
            used(&uses, "new", NOW),
            [one("new", 1, NOW), one("a", 2, at(10)), one("b", 5, at(20))]
        );
    }

    #[test]
    fn a_use_past_the_total_halves_every_count_and_drops_the_single_uses() {
        let full = MAX_TOTAL - 1;
        let uses = [one("often", full, at(10)), one("once", 1, at(20))];
        assert_eq!(used(&uses, "new", NOW), [one("often", full / 2, at(10))]);
        assert_eq!(
            used(&[one("often", full, at(10))], "once", NOW),
            [one("once", 1, NOW), one("often", full, at(10))]
        );
    }

    #[test]
    fn frequent_and_recent_uses_boost_more() {
        let uses = [
            one("daily", 30, NOW.before(60)),
            one("once now", 1, NOW),
            one("once last month", 1, NOW.before(2_592_000)),
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
            [one("a", 1, NOW), one("b", 1, NOW.before(1))]
        );
    }
}
