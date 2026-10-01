//! The picker's logic, free of any window.

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

/// `text` after Ctrl+Backspace at its end: the trailing whitespace and the word before it removed.
pub fn delete_word(text: &str) -> &str {
    text.trim_end()
        .trim_end_matches(|c: char| !c.is_whitespace())
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

    #[test]
    fn delete_word_removes_last_word_and_its_trailing_space() {
        assert_eq!(delete_word("visual studio "), "visual ");
        assert_eq!(delete_word("visual studio"), "visual ");
        assert_eq!(delete_word("code"), "");
        assert_eq!(delete_word(""), "");
    }
}
