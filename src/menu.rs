//! The picker's logic, free of any window.

use std::cmp::Reverse;
use std::num::NonZeroUsize;

#[derive(Debug, PartialEq, Eq)]
pub enum Choice<T> {
    Item(T),
    /// Typed text, picked when nothing matched or forced with Shift+Enter.
    Text(String),
    Cancel,
}

/// Indices of the items holding the letters of every whitespace-separated word of `query` in order, gaps allowed,
/// ignoring case, best matches first. Equal matches keep their input order, which callers use for recency or depth.
pub fn filter(items: &[String], query: &str) -> Vec<usize> {
    let words: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|word| word.chars().map(lower).collect())
        .collect();
    let mut ranked: Vec<(Reverse<i32>, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let letters = letters(item);
            let fits: Option<Vec<Fit>> = words.iter().map(|word| fit(&letters, word)).collect();
            fits.map(|fits| (Reverse(fits.iter().map(|fit| fit.score).sum()), index))
        })
        .collect();
    ranked.sort_by_key(|&(rank, _)| rank);
    ranked.into_iter().map(|(_, index)| index).collect()
}

/// The char positions in `item` of the letters the words of `query` matched in `filter`, ascending.
pub fn matched(item: &str, query: &str) -> Vec<usize> {
    let letters = letters(item);
    let mut positions: Vec<usize> = query
        .split_whitespace()
        .filter_map(|word| fit(&letters, &word.chars().map(lower).collect::<Vec<char>>()))
        .flat_map(|fit| fit.positions)
        .collect();
    positions.sort_unstable();
    positions.dedup();
    positions
}

const MATCH: i32 = 16;
/// Per matched letter at a word start: the item's start, after a non-alphanumeric, or a capital after a lowercase.
const BOUNDARY: i32 = 8;
/// Per matched letter in the name, the part after the last path separator, so name matches beat folder matches.
const NAME: i32 = 8;
const CONSECUTIVE: i32 = 4;
const GAP_START: i32 = 3;
const GAP_EXTENSION: i32 = 1;

/// One letter of an item, lowercase, with the bonuses a match on it earns.
#[derive(Clone, Copy)]
struct Letter {
    letter: char,
    boundary: i32,
    name: i32,
}

fn letters(item: &str) -> Vec<Letter> {
    let name = item.rfind(['\\', '/']).map_or(0, |separator| separator + 1);
    let mut previous: Option<char> = None;
    item.char_indices()
        .map(|(at, letter)| {
            let boundary = previous.is_none_or(|previous| {
                !previous.is_alphanumeric() || (previous.is_lowercase() && letter.is_uppercase())
            });
            previous = Some(letter);
            Letter {
                letter: lower(letter),
                boundary: if boundary { BOUNDARY } else { 0 },
                name: if at >= name { NAME } else { 0 },
            }
        })
        .collect()
}

/// The first char of `letter`'s lowercase, so item and query letters line up one to one.
fn lower(letter: char) -> char {
    letter.to_lowercase().next().unwrap_or(letter)
}

/// The best way found to match a word so far with its last letter at one position.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Cell {
    score: i32,
    /// The boundary bonus of the first letter of this run of consecutive matches, which every letter of the run earns,
    /// so `src` matched whole at a word start beats its letters scattered over other word starts.
    run: i32,
    /// Where the word's previous letter sits in this match.
    from: Option<usize>,
}

/// A word's best match in an item.
struct Fit {
    score: i32,
    /// The char positions of the word's letters in the item, in order.
    positions: Vec<usize>,
}

/// The best match of `word`'s letters in order in `letters`, or `None` when they do not all occur. Each row holds, per
/// position, the best match of the word so far with its last letter there.
fn fit(letters: &[Letter], word: &[char]) -> Option<Fit> {
    let (first, rest) = word.split_first()?;
    let mut rows: Vec<Vec<Option<Cell>>> = vec![
        letters
            .iter()
            .map(|letter| {
                (letter.letter == *first).then_some(Cell {
                    score: MATCH + letter.boundary + letter.name,
                    run: letter.boundary,
                    from: None,
                })
            })
            .collect(),
    ];
    for wanted in rest {
        // The best score that reaches this position across a gap of at least one letter, and where it ended.
        let mut gapped: Option<(i32, usize)> = None;
        let mut before: Option<(Cell, usize)> = None;
        let row = letters
            .iter()
            .zip(rows.last()?)
            .enumerate()
            .map(|(at, (letter, &here))| {
                let extended = before.map(|(cell, from)| {
                    let run = cell.run.max(letter.boundary);
                    Cell {
                        score: cell.score + MATCH + CONSECUTIVE + run + letter.name,
                        run,
                        from: Some(from),
                    }
                });
                let jumped = gapped.map(|(score, from)| Cell {
                    score: score + MATCH + letter.boundary + letter.name,
                    run: letter.boundary,
                    from: Some(from),
                });
                let cell = extended.max(jumped).filter(|_| letter.letter == *wanted);
                gapped = gapped
                    .map(|(score, from)| (score - GAP_EXTENSION, from))
                    .max(before.map(|(cell, from)| (cell.score - GAP_START, from)));
                before = here.map(|cell| (cell, at));
                cell
            })
            .collect();
        rows.push(row);
    }
    let (end, best) = rows
        .last()?
        .iter()
        .enumerate()
        .filter_map(|(at, cell)| cell.map(|cell| (at, cell)))
        .max_by_key(|&(_, cell)| cell.score)?;
    let mut positions = vec![end];
    let mut from = best.from;
    for row in rows.iter().rev().skip(1) {
        let Some(at) = from else { break };
        positions.push(at);
        from = row.get(at).copied().flatten().and_then(|cell| cell.from);
    }
    positions.reverse();
    Some(Fit {
        score: best.score,
        positions,
    })
}

/// The row `by` rows from `cursor`, wrapping at both ends of `len` rows.
pub fn step(cursor: usize, len: usize, by: isize) -> usize {
    match isize::try_from(len) {
        Ok(len) if len > 0 => (cursor as isize + by).rem_euclid(len) as usize,
        _ => 0,
    }
}

/// The row `by` rows from `cursor`, stopping at both ends of `len` rows.
pub fn clamped(cursor: usize, len: usize, by: isize) -> usize {
    cursor.saturating_add_signed(by).min(len.saturating_sub(1))
}

/// One notch of a mouse wheel, `WHEEL_DELTA` in Win32.
const NOTCH: i32 = 120;

/// What a wheel turn does to the selection.
#[derive(Debug, PartialEq, Eq)]
pub struct Scroll {
    /// Positive moves down the list.
    pub rows: isize,
    /// The part of a notch left over, added to the next turn: touchpads send fractions of a notch.
    pub pending: i32,
}

/// A wheel turn of `delta`, positive away from the person, on top of the `pending` part of a notch.
pub fn scroll(pending: i32, delta: i32) -> Scroll {
    let total = pending.saturating_add(delta);
    Scroll {
        rows: -(total / NOTCH) as isize,
        pending: total % NOTCH,
    }
}

/// The row beside `cursor` in the previous column of a grid filled down columns of `lines`, else `cursor`.
pub fn column_left(cursor: usize, lines: NonZeroUsize) -> usize {
    cursor.checked_sub(lines.get()).unwrap_or(cursor)
}

/// The row beside `cursor` in the next column of `len` rows filled down columns of `lines`, or that column's last row
/// when it is shorter, else `cursor`.
pub fn column_right(cursor: usize, len: usize, lines: NonZeroUsize) -> usize {
    let last = len.saturating_sub(1);
    match cursor / lines < last / lines {
        true => (cursor + lines.get()).min(last),
        false => cursor,
    }
}

/// What Enter picks: the item under the cursor, else the typed query.
pub fn accept(shown: &[usize], cursor: usize, query: &str) -> Choice<usize> {
    match shown.get(cursor) {
        Some(&index) => Choice::Item(index),
        None => typed(query),
    }
}

/// What Shift+Enter picks: the typed query, or nothing when it is empty.
pub fn typed(query: &str) -> Choice<usize> {
    match query {
        "" => Choice::Cancel,
        _ => Choice::Text(query.to_owned()),
    }
}

/// The first match on the page that holds `cursor`, for a grid of `page` cells.
pub fn first(cursor: usize, page: NonZeroUsize) -> usize {
    cursor / page * page.get()
}

/// The query line, split at the caret.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub before: String,
    pub after: String,
}

impl Line {
    pub fn text(&self) -> String {
        format!("{}{}", self.before, self.after)
    }

    /// The caret's offset in UTF-16 units, as DirectWrite counts.
    pub fn caret(&self) -> usize {
        self.before.encode_utf16().count()
    }

    /// Typed or pasted text at the caret, without control characters such as a pasted line break.
    pub fn insert(&self, text: &str) -> Line {
        let typed: String = text.chars().filter(|c| !c.is_control()).collect();
        Line {
            before: format!("{}{typed}", self.before),
            after: self.after.clone(),
        }
    }

    pub fn backspace(&self) -> Line {
        let mut before = self.before.clone();
        before.pop();
        Line {
            before,
            after: self.after.clone(),
        }
    }

    pub fn delete(&self) -> Line {
        Line {
            before: self.before.clone(),
            after: self.after.chars().skip(1).collect(),
        }
    }

    /// Ctrl+Backspace: the whitespace before the caret and the word before that removed.
    pub fn delete_word(&self) -> Line {
        let before = self
            .before
            .trim_end()
            .trim_end_matches(|c: char| !c.is_whitespace());
        Line {
            before: before.to_owned(),
            after: self.after.clone(),
        }
    }

    pub fn left(&self) -> Line {
        let mut before = self.before.clone();
        let after = match before.pop() {
            Some(moved) => format!("{moved}{}", self.after),
            None => self.after.clone(),
        };
        Line { before, after }
    }

    pub fn right(&self) -> Line {
        let mut after = self.after.chars();
        let before = match after.next() {
            Some(moved) => format!("{}{moved}", self.before),
            None => self.before.clone(),
        };
        Line {
            before,
            after: after.collect(),
        }
    }

    pub fn home(&self) -> Line {
        Line {
            before: String::new(),
            after: self.text(),
        }
    }

    pub fn end(&self) -> Line {
        Line {
            before: self.text(),
            after: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_owned()).collect()
    }

    #[test]
    fn empty_query_keeps_every_item() {
        assert_eq!(filter(&items(&["a", "b"]), "  "), [0, 1]);
    }

    #[test]
    fn filter_ignores_case() {
        assert_eq!(
            filter(&items(&["Visual Studio Code", "Notepad"]), "code"),
            [0]
        );
    }

    #[test]
    fn filter_needs_every_word_in_any_order() {
        let apps = items(&["Windows Terminal", "Terminal Preview", "Windows Security"]);
        assert_eq!(filter(&apps, "term win"), [0]);
    }

    #[test]
    fn equal_matches_keep_input_order() {
        assert_eq!(filter(&items(&["zeta", "alpha", "beta"]), "eta"), [0, 2]);
    }

    #[test]
    fn a_word_start_beats_a_match_mid_word() {
        let apps = items(&["Notepad", "Paint", "Snipping Tool"]);
        assert_eq!(filter(&apps, "pa"), [1, 0]);
        assert_eq!(filter(&apps, "t"), [2, 0, 1]);
    }

    #[test]
    fn a_word_start_in_the_name_beats_one_in_the_folders_and_both_beat_one_mid_word() {
        let entries = items(&[
            "run\\notes.md",
            "zet\\tests\\integration\\main.go",
            "zet\\tests\\integration\\run.ps1",
            "zet\\prune.go",
        ]);
        assert_eq!(filter(&entries, "run"), [2, 0, 3]);
        assert_eq!(filter(&entries, "integ run"), [2]);
    }

    #[test]
    fn letters_match_in_order_with_gaps() {
        let apps = items(&["carronade", "nordic"]);
        assert_eq!(filter(&apps, "crnd"), [0]);
        assert_eq!(filter(&apps, "dnrc"), Vec::<usize>::new());
    }

    #[test]
    fn a_tight_match_beats_scattered_letters() {
        let apps = items(&["Network Tools Extra", "Notepad"]);
        assert_eq!(filter(&apps, "note"), [1, 0]);
    }

    #[test]
    fn a_capital_after_a_lowercase_starts_a_word() {
        let apps = items(&["Snowman", "SnowMan"]);
        assert_eq!(filter(&apps, "m"), [1, 0]);
    }

    #[test]
    fn every_word_adds_to_the_rank() {
        let entries = items(&["src\\domain.rs", "src\\main.rs"]);
        assert_eq!(filter(&entries, "src main"), [1, 0]);
    }

    #[test]
    fn matched_marks_the_letters_each_word_matched() {
        assert_eq!(matched("carronade", "cade"), [0, 1, 7, 8]);
        assert_eq!(matched("src\\main.rs", "main src"), [0, 1, 2, 4, 5, 6, 7]);
        assert_eq!(matched("SnowMan", "m"), [4]);
        assert_eq!(matched("notepad", "x"), Vec::<usize>::new());
    }

    #[test]
    fn step_wraps_both_ways() {
        assert_eq!(step(2, 3, 1), 0);
        assert_eq!(step(0, 3, -1), 2);
        assert_eq!(step(1, 3, 1), 2);
    }

    #[test]
    fn step_in_empty_list_stays_at_zero() {
        assert_eq!(step(0, 0, 1), 0);
    }

    #[test]
    fn clamped_stops_at_both_ends() {
        assert_eq!(clamped(1, 3, 5), 2);
        assert_eq!(clamped(1, 3, -5), 0);
        assert_eq!(clamped(0, 3, 1), 1);
        assert_eq!(clamped(0, 0, 1), 0);
    }

    #[test]
    fn a_wheel_moves_a_row_per_notch_and_keeps_the_rest() {
        let turn = |rows, pending| Scroll { rows, pending };
        assert_eq!(scroll(0, 120), turn(-1, 0));
        assert_eq!(scroll(0, -240), turn(2, 0));
        assert_eq!(scroll(0, 40), turn(0, 40));
        assert_eq!(scroll(80, 40), turn(-1, 0));
        assert_eq!(scroll(0, -130), turn(1, -10));
    }

    #[test]
    fn columns_move_by_a_column_and_stop_at_the_ends() -> Result<(), &'static str> {
        let lines = NonZeroUsize::new(3).ok_or("zero lines")?;
        assert_eq!(
            [0, 2, 3, 7].map(|cursor| column_left(cursor, lines)),
            [0, 2, 0, 4]
        );
        assert_eq!(
            [0, 1, 2, 3, 4].map(|cursor| column_right(cursor, 5, lines)),
            [3, 4, 4, 3, 4]
        );
        assert_eq!(column_right(0, 0, lines), 0);
        Ok(())
    }

    #[test]
    fn accept_picks_row_under_cursor() {
        assert_eq!(accept(&[4, 7], 1, "x"), Choice::Item(7));
    }

    #[test]
    fn accept_without_match_picks_query() {
        assert_eq!(
            accept(&[], 0, "notepad"),
            Choice::Text("notepad".to_owned())
        );
    }

    #[test]
    fn accept_with_nothing_cancels() {
        assert_eq!(accept(&[], 0, ""), Choice::Cancel);
    }

    fn line(before: &str, after: &str) -> Line {
        Line {
            before: before.to_owned(),
            after: after.to_owned(),
        }
    }

    #[test]
    fn first_is_the_start_of_the_cursors_page() -> Result<(), &'static str> {
        let page = NonZeroUsize::new(6).ok_or("zero page")?;
        assert_eq!(
            [0, 5, 6, 13].map(|cursor| first(cursor, page)),
            [0, 0, 6, 12]
        );
        Ok(())
    }

    #[test]
    fn delete_word_removes_last_word_and_its_trailing_space() {
        assert_eq!(
            line("visual studio ", "").delete_word(),
            line("visual ", "")
        );
        assert_eq!(
            line("visual studio", " x").delete_word(),
            line("visual ", " x")
        );
        assert_eq!(line("code", "").delete_word(), line("", ""));
        assert_eq!(line("", "code").delete_word(), line("", "code"));
    }

    #[test]
    fn insert_goes_at_the_caret_without_control_characters() {
        assert_eq!(line("ga", "ma").insert("m"), line("gam", "ma"));
        assert_eq!(line("", "").insert("one\r\ntwo\t"), line("onetwo", ""));
    }

    #[test]
    fn backspace_and_delete_remove_one_character_either_side() {
        assert_eq!(line("café", "☕").backspace(), line("caf", "☕"));
        assert_eq!(line("café", "☕!").delete(), line("café", "!"));
        assert_eq!(line("", "x").backspace(), line("", "x"));
        assert_eq!(line("x", "").delete(), line("x", ""));
    }

    #[test]
    fn the_caret_moves_by_characters_and_stops_at_the_ends() {
        assert_eq!(line("é", "☕").left(), line("", "é☕"));
        assert_eq!(line("é", "☕").right(), line("é☕", ""));
        assert_eq!(line("", "a").left(), line("", "a"));
        assert_eq!(line("a", "").right(), line("a", ""));
        assert_eq!(line("ab", "cd").home(), line("", "abcd"));
        assert_eq!(line("ab", "cd").end(), line("abcd", ""));
    }

    #[test]
    fn the_caret_counts_utf16_units() {
        assert_eq!(line("a😀", "b").caret(), 3);
        assert_eq!(line("a😀", "b").text(), "a😀b");
    }
}
