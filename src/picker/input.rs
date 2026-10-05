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
    let ctrl = held(VK_CONTROL);
    let shift = held(VK_SHIFT);
    match key {
        VK_ESCAPE => finish(Ok(Action::Pick(Choice::Cancel)))?,
        VK_RETURN if ctrl => {
            if let Some(row) = with(|state| state.shown.get(state.cursor).copied())? {
                finish(Ok(if shift {
                    Action::Admin(row)
                } else {
                    Action::Terminal(row)
                }))?;
            }
        }
        VK_RETURN => {
            let choice = with(|state| {
                let query = state.line.text();
                if shift {
                    menu::typed(&query)
                } else {
                    menu::accept(&state.shown, state.cursor, &query)
                }
            })?;
            finish(Ok(Action::Pick(choice)))?;
        }
        VK_TAB if with(|state| state.switch.is_some())? => finish(Ok(Action::Switch))?,
        VK_DOWN => move_by(1)?,
        VK_UP => move_by(-1)?,
        VK_N if ctrl => move_by(1)?,
        VK_P if ctrl => move_by(-1)?,
        VK_BACK if ctrl => edit(Line::delete_word)?,
        VK_BACK => edit(Line::backspace)?,
        VK_DELETE => edit(Line::delete)?,
        VK_LEFT if with(|state| state.line.before.is_empty())? => {
            move_to(|state| menu::column_left(state.cursor, state.canvas.config.list.lines))?;
        }
        VK_RIGHT if with(|state| state.line.after.is_empty())? => move_to(|state| {
            let lines = state.canvas.config.list.lines;
            menu::column_right(state.cursor, state.shown.len(), lines)
        })?,
        VK_LEFT => edit(Line::left)?,
        VK_RIGHT => edit(Line::right)?,
        VK_HOME => edit(Line::home)?,
        VK_END => edit(Line::end)?,
        VK_V if ctrl => {
            let pasted = clipboard()?;
            edit(|line| line.insert(&pasted))?;
        }
        _ => return Ok(false),
    }
    Ok(true)
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
