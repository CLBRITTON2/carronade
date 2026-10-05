//! The window procedure: keys, typed text and the mouse, turned into edits of the query, cursor moves and actions.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
    VK_LEFT, VK_N, VK_P, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, WA_INACTIVE, WM_ACTIVATE, WM_CHAR, WM_CLOSE, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_MOUSEWHEEL,
};

use super::clipboard::clipboard;
use super::{Action, State, candidates, finish, render, with};
use crate::error::Error;
use crate::menu::{self, Choice, Line};

pub(super) extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Win32 packs keys, characters and wheel turns into a word of `wparam`, so the casts below take that word.
    let result = match message {
        WM_KEYDOWN => match key(VIRTUAL_KEY(wparam.0 as u16)) {
            // SAFETY: passes on the message this procedure received, unchanged.
            Ok(false) => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
            handled => handled.map(|_| ()),
        },
        WM_CHAR => typed(wparam.0 as u16),
        WM_LBUTTONDOWN => {
            let (x, y) = client(lparam);
            clicked(f32::from(x), f32::from(y))
        }
        WM_MOUSEMOVE => hovered(client(lparam)),
        // The high word is the signed turn.
        WM_MOUSEWHEEL => scrolled((wparam.0 >> 16) as i16),
        WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_INACTIVE => {
            finish(Ok(Action::Pick(Choice::Cancel)))
        }
        // DefWindowProcW would destroy the window, which only `browse` may do.
        WM_CLOSE => finish(Ok(Action::Pick(Choice::Cancel))),
        // SAFETY: passes on the message this procedure received, unchanged.
        _ => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
    };
    if let Err(error) = result {
        // Only the wake-up can fail here, and the error is recorded before it, so `pump` still returns it.
        _ = finish(Err(error));
    }
    LRESULT(0)
}

/// Handles the keys that steer the picker or edit the query, returning false for the rest.
fn key(key: VIRTUAL_KEY) -> Result<bool, Error> {
    let (ctrl, shift) = (held(VK_CONTROL), held(VK_SHIFT));
    let context = with(|state| KeyContext {
        ctrl,
        shift,
        caret_at_start: state.line.before.is_empty(),
        caret_at_end: state.line.after.is_empty(),
        switch: state.switch.is_some(),
    })?;
    match key_action(key, &context) {
        Some(action) => act(action).map(|()| true),
        None => Ok(false),
    }
}

/// What a key asks of the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyAction {
    Cancel,
    /// Pick the selected match, or the query when nothing matches.
    Accept,
    /// Pick the query as typed, even when something matches.
    AcceptTyped,
    Terminal,
    Admin,
    Switch,
    Down,
    Up,
    ColumnLeft,
    ColumnRight,
    DeleteWord,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Paste,
}

/// What a key's meaning depends on besides the key.
struct KeyContext {
    ctrl: bool,
    shift: bool,
    caret_at_start: bool,
    caret_at_end: bool,
    /// The bar shows a switch icon.
    switch: bool,
}

fn key_action(key: VIRTUAL_KEY, context: &KeyContext) -> Option<KeyAction> {
    let &KeyContext {
        ctrl,
        shift,
        caret_at_start,
        caret_at_end,
        switch,
    } = context;
    Some(match key {
        VK_ESCAPE => KeyAction::Cancel,
        VK_RETURN if ctrl && shift => KeyAction::Admin,
        VK_RETURN if ctrl => KeyAction::Terminal,
        VK_RETURN if shift => KeyAction::AcceptTyped,
        VK_RETURN => KeyAction::Accept,
        VK_TAB if switch => KeyAction::Switch,
        VK_DOWN => KeyAction::Down,
        VK_UP => KeyAction::Up,
        VK_N if ctrl => KeyAction::Down,
        VK_P if ctrl => KeyAction::Up,
        VK_BACK if ctrl => KeyAction::DeleteWord,
        VK_BACK => KeyAction::Backspace,
        VK_DELETE => KeyAction::Delete,
        VK_LEFT if caret_at_start => KeyAction::ColumnLeft,
        VK_RIGHT if caret_at_end => KeyAction::ColumnRight,
        VK_LEFT => KeyAction::Left,
        VK_RIGHT => KeyAction::Right,
        VK_HOME => KeyAction::Home,
        VK_END => KeyAction::End,
        VK_V if ctrl => KeyAction::Paste,
        _ => return None,
    })
}

fn act(action: KeyAction) -> Result<(), Error> {
    match action {
        KeyAction::Cancel => finish(Ok(Action::Pick(Choice::Cancel))),
        KeyAction::Accept => {
            let choice =
                with(|state| menu::accept(&state.shown, state.cursor, &state.line.text()))?;
            finish(Ok(Action::Pick(choice)))
        }
        KeyAction::AcceptTyped => {
            let choice = with(|state| menu::typed(&state.line.text()))?;
            finish(Ok(Action::Pick(choice)))
        }
        KeyAction::Terminal => on_selected(Action::Terminal),
        KeyAction::Admin => on_selected(Action::Admin),
        KeyAction::Switch => finish(Ok(Action::Switch)),
        KeyAction::Down => move_by(1),
        KeyAction::Up => move_by(-1),
        KeyAction::ColumnLeft => {
            move_to(|state| menu::column_left(state.cursor, state.canvas.config.list.lines.get()))
        }
        KeyAction::ColumnRight => move_to(|state| {
            let lines = state.canvas.config.list.lines.get();
            menu::column_right(state.cursor, state.shown.len(), lines)
        }),
        KeyAction::DeleteWord => edit(Line::delete_word),
        KeyAction::Backspace => edit(Line::backspace),
        KeyAction::Delete => edit(Line::delete),
        KeyAction::Left => edit(Line::left),
        KeyAction::Right => edit(Line::right),
        KeyAction::Home => edit(Line::home),
        KeyAction::End => edit(Line::end),
        KeyAction::Paste => {
            let pasted = clipboard()?;
            edit(|line| line.insert(&pasted))
        }
    }
}

/// Finishes with `action` on the selected match, or does nothing when there is none.
fn on_selected(action: fn(usize) -> Action<usize>) -> Result<(), Error> {
    match with(|state| state.shown.get(state.cursor).copied())? {
        Some(row) => finish(Ok(action(row))),
        None => Ok(()),
    }
}

/// Inserts a typed UTF-16 unit. Control characters, which Backspace and Enter also type, insert nothing, and neither
/// does an orphan surrogate.
fn typed(unit: u16) -> Result<(), Error> {
    let units = with(|state| match (state.surrogate.take(), unit) {
        (_, 0xd800..=0xdbff) => {
            state.surrogate = Some(unit);
            Vec::new()
        }
        (Some(high), _) => vec![high, unit],
        (None, _) => vec![unit],
    })?;
    let text: String = char::decode_utf16(units).filter_map(Result::ok).collect();
    edit(|line| line.insert(&text))
}

/// Applies `change` to the query, filtering again when its text changed.
fn edit(change: impl FnOnce(&Line) -> Line) -> Result<(), Error> {
    with(|state| {
        let line = change(&state.line);
        if line.text() != state.line.text() {
            state.shown = menu::filter(candidates(&state.items), &line.text());
            state.cursor = 0;
        }
        state.line = line;
    })?;
    render()
}

fn move_by(by: isize) -> Result<(), Error> {
    move_to(|state| menu::step(state.cursor, state.shown.len(), by))
}

fn move_to(cursor: impl FnOnce(&State) -> usize) -> Result<(), Error> {
    with(|state| state.cursor = cursor(state))?;
    render()
}

fn scrolled(delta: i16) -> Result<(), Error> {
    with(|state| {
        let scroll = menu::scroll(state.wheel, i32::from(delta));
        state.wheel = scroll.pending;
        state.cursor = menu::clamped(state.cursor, state.shown.len(), scroll.rows);
    })?;
    render()
}

/// The low and high words of a mouse message's `lparam`, its signed client coordinates.
fn client(lparam: LPARAM) -> (i16, i16) {
    (lparam.0 as i16, (lparam.0 >> 16) as i16)
}

/// Selects the match under the pointer. Windows also sends a move without one when the window appears under a
/// resting pointer or redraws, so only a change from the last known position counts.
fn hovered(at: (i16, i16)) -> Result<(), Error> {
    let moved = with(|state| {
        let last = state.pointer.replace(at);
        if last.is_none_or(|last| last == at) {
            return false;
        }
        let row = state
            .canvas
            .layout
            .cell_at(f32::from(at.0), f32::from(at.1))
            .map(|slot| menu::first(state.cursor, state.page()) + slot)
            .filter(|&row| row < state.shown.len() && row != state.cursor);
        if let Some(row) = row {
            state.cursor = row;
        }
        row.is_some()
    })?;
    if moved { render() } else { Ok(()) }
}

fn clicked(x: f32, y: f32) -> Result<(), Error> {
    if with(|state| state.switch.is_some() && state.canvas.layout.on_switch(x, y))? {
        return finish(Ok(Action::Switch));
    }
    let choice = with(|state| {
        let slot = state.canvas.layout.cell_at(x, y)?;
        let row = menu::first(state.cursor, state.page()) + slot;
        state
            .shown
            .get(row)
            .map(|_| menu::accept(&state.shown, row, ""))
    })?;
    match choice {
        Some(choice) => finish(Ok(Action::Pick(choice))),
        None => Ok(()),
    }
}

fn held(key: VIRTUAL_KEY) -> bool {
    // SAFETY: reads this thread's keyboard state for a virtual key code.
    let state = unsafe { GetKeyState(i32::from(key.0)) };
    state < 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The action of `key` with the caret mid-query and a switch shown.
    fn action(key: VIRTUAL_KEY, ctrl: bool, shift: bool) -> Option<KeyAction> {
        let context = KeyContext {
            ctrl,
            shift,
            caret_at_start: false,
            caret_at_end: false,
            switch: true,
        };
        key_action(key, &context)
    }

    #[test]
    fn modifiers_pick_what_enter_does() {
        let enter = |ctrl, shift| action(VK_RETURN, ctrl, shift);
        assert_eq!(enter(false, false), Some(KeyAction::Accept));
        assert_eq!(enter(false, true), Some(KeyAction::AcceptTyped));
        assert_eq!(enter(true, false), Some(KeyAction::Terminal));
        assert_eq!(enter(true, true), Some(KeyAction::Admin));
    }

    #[test]
    fn ctrl_letters_move_and_paste_only_with_ctrl() {
        assert_eq!(action(VK_N, true, false), Some(KeyAction::Down));
        assert_eq!(action(VK_P, true, false), Some(KeyAction::Up));
        assert_eq!(action(VK_V, true, false), Some(KeyAction::Paste));
        assert_eq!(action(VK_N, false, false), None);
        assert_eq!(action(VK_P, false, false), None);
        assert_eq!(action(VK_V, false, false), None);
    }

    #[test]
    fn ctrl_backspace_deletes_a_word() {
        assert_eq!(action(VK_BACK, true, false), Some(KeyAction::DeleteWord));
        assert_eq!(action(VK_BACK, false, false), Some(KeyAction::Backspace));
    }

    #[test]
    fn arrows_past_the_query_ends_change_column() {
        let context = |caret_at_start, caret_at_end| KeyContext {
            ctrl: false,
            shift: false,
            caret_at_start,
            caret_at_end,
            switch: false,
        };
        assert_eq!(
            key_action(VK_LEFT, &context(true, false)),
            Some(KeyAction::ColumnLeft)
        );
        assert_eq!(
            key_action(VK_LEFT, &context(false, true)),
            Some(KeyAction::Left)
        );
        assert_eq!(
            key_action(VK_RIGHT, &context(false, true)),
            Some(KeyAction::ColumnRight)
        );
        assert_eq!(
            key_action(VK_RIGHT, &context(true, false)),
            Some(KeyAction::Right)
        );
    }

    #[test]
    fn tab_switches_only_with_a_switch() {
        assert_eq!(action(VK_TAB, false, false), Some(KeyAction::Switch));
        let context = KeyContext {
            ctrl: false,
            shift: false,
            caret_at_start: false,
            caret_at_end: false,
            switch: false,
        };
        assert_eq!(key_action(VK_TAB, &context), None);
    }
}
