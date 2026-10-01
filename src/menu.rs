//! The picker's logic, free of any window.

use std::num::NonZeroUsize;

#[derive(Debug, PartialEq, Eq)]
pub enum Choice<T> {
    Item(T),
    /// Typed text, picked when nothing matched or forced with Shift+Enter.
    Text(String),
    Cancel,
}

/// Indices of the items that contain every whitespace-separated word of `query`, ignoring case, in input order.
pub fn filter(items: &[String], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            let item = item.to_lowercase();
            words.iter().all(|word| item.contains(word.as_str()))
        })
        .map(|(index, _)| index)
        .collect()
}

/// The row `by` rows from `cursor`, wrapping at both ends of `len` rows.
pub fn step(cursor: usize, len: usize, by: isize) -> usize {
    match isize::try_from(len) {
        Ok(len) if len > 0 => (cursor as isize + by).rem_euclid(len) as usize,
        _ => 0,
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
    fn filter_keeps_input_order() {
        assert_eq!(filter(&items(&["zeta", "alpha", "beta"]), "eta"), [0, 2]);
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
