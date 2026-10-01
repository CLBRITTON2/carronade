//! The picker window: a query line over the matching items, in the intarsia menu's colors.

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DC_BRUSH, DC_PEN,
    DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW, FW_NORMAL,
    FillRect, GetMonitorInfoW, GetStockObject, HBRUSH, HDC, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromWindow, OUT_DEFAULT_PRECIS, RoundRect, SelectObject, SetBkColor, SetBkMode,
    SetDCBrushColor, SetDCPenColor, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, EM_GETSEL, EM_SETSEL, ODS_SELECTED};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput, SetFocus,
    VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_N, VK_P, VK_RETURN, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EN_CHANGE, ES_AUTOHSCROLL,
    GetForegroundWindow, GetMessageW, GetWindowTextLengthW, GetWindowTextW, IDC_ARROW, LB_ERR,
    LB_GETCURSEL, LB_SETCOUNT, LB_SETCURSEL, LB_SETITEMHEIGHT, LBN_SELCHANGE, LBS_NODATA,
    LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, LBS_OWNERDRAWFIXED, LoadCursorW, MSG, PostQuitMessage,
    RegisterClassW, SW_SHOW, SendMessageW, SetForegroundWindow, SetWindowTextW, ShowWindow,
    TranslateMessage, WA_INACTIVE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_COMMAND,
    WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_DRAWITEM, WM_KEYDOWN, WM_SETFONT, WNDCLASSW, WS_CHILD,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::error::{Error, last, win32};
use crate::menu::{self, Choice};

const BACKGROUND: COLORREF = rgb(0x1f, 0x1d, 0x2e);
const SELECTED: COLORREF = rgb(0x40, 0x3d, 0x52);
const FOREGROUND: COLORREF = rgb(0xe0, 0xde, 0xf4);
const BORDER: COLORREF = rgb(0x90, 0x8c, 0xaa);
const FONT: PCWSTR = w!("JetBrainsMono NF");
const CLASS: PCWSTR = w!("carronade");

// Lengths in px at 96 DPI.
const WIDTH: i32 = 640;
const ROW: i32 = 32;
const ROWS: i32 = 10;
const PAD: i32 = 8;
const INSET: i32 = 10;
const RADIUS: i32 = 8;
const FONT_SIZE: i32 = 18;
const LINE_HEIGHT: i32 = 24;

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | (green as u32) << 8 | (blue as u32) << 16)
}

struct State {
    items: Vec<String>,
    shown: Vec<usize>,
    cursor: usize,
    edit: HWND,
    list: HWND,
    dpi: i32,
}

// Win32 calls re-enter the window procedure, so every borrow ends before the next call.
thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static OUTCOME: RefCell<Option<Result<Choice<usize>, Error>>> = const { RefCell::new(None) };
}

/// Shows `items` under a query line until one is picked. Call it once per process: it registers the window class.
pub fn pick<T: AsRef<str>>(items: Vec<T>) -> Result<Choice<T>, Error> {
    let window = open(items.iter().map(|item| item.as_ref().to_owned()).collect())?;
    let mut message = MSG::default();
    loop {
        match unsafe { GetMessageW(&mut message, None, 0, 0) }.0 {
            -1 => return Err(last("GetMessageW")),
            0 => break,
            _ => {}
        }
        if message.message == WM_KEYDOWN {
            match key(VIRTUAL_KEY(message.wParam.0 as u16)) {
                Ok(false) => {}
                Ok(true) => continue,
                Err(error) => finish(Err(error)),
            }
        }
        unsafe {
            _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    unsafe { DestroyWindow(window) }.map_err(win32("DestroyWindow"))?;
    match OUTCOME.take().ok_or(Error::NoChoice)?? {
        Choice::Item(row) => {
            let len = items.len();
            items
                .into_iter()
                .nth(row)
                .map(Choice::Item)
                .ok_or(Error::Row { row, len })
        }
        Choice::Text(text) => Ok(Choice::Text(text)),
        Choice::Cancel => Ok(Choice::Cancel),
    }
}

fn open(items: Vec<String>) -> Result<HWND, Error> {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .map_err(win32("SetProcessDpiAwarenessContext"))?;
    let monitor = unsafe { MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(monitor, &mut info) }
        .ok()
        .map_err(win32("GetMonitorInfoW"))?;
    let (mut dpi, mut dpi_y) = (0, 0);
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) }
        .map_err(win32("GetDpiForMonitor"))?;
    let dpi = dpi as i32;
    let px = |length: i32| scale(length, dpi);

    let instance = unsafe { GetModuleHandleW(None) }.map_err(win32("GetModuleHandleW"))?;
    let background = unsafe { CreateSolidBrush(BACKGROUND) };
    if background.is_invalid() {
        return Err(last("CreateSolidBrush"));
    }
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.map_err(win32("LoadCursorW"))?,
        hbrBackground: background,
        lpszClassName: CLASS,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(last("RegisterClassW"));
    }

    let (width, height) = (px(WIDTH), px(PAD * 2 + ROW * (ROWS + 1)));
    let work = info.rcWork;
    let window = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            CLASS,
            WS_POPUP,
            work.left + (work.right - work.left - width) / 2,
            work.top + (work.bottom - work.top - height) / 2,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(win32("CreateWindowExW"))?;
    let (corners, border) = (DWMWCP_ROUND, BORDER);
    unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const corners).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    }
    .map_err(win32("DwmSetWindowAttribute"))?;
    unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_BORDER_COLOR,
            (&raw const border).cast(),
            size_of::<COLORREF>() as u32,
        )
    }
    .map_err(win32("DwmSetWindowAttribute"))?;

    let text_left = px(PAD + INSET);
    let edit = child(
        window,
        w!("EDIT"),
        WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        text_left,
        px(PAD + (ROW - LINE_HEIGHT) / 2),
        width - 2 * text_left,
        px(LINE_HEIGHT),
    )?;
    let list = child(
        window,
        w!("LISTBOX"),
        WINDOW_STYLE((LBS_OWNERDRAWFIXED | LBS_NODATA | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
        px(PAD),
        px(PAD + ROW),
        width - 2 * px(PAD),
        px(ROW * ROWS),
    )?;
    let font = unsafe {
        CreateFontW(
            -px(FONT_SIZE),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            FONT,
        )
    };
    if font.is_invalid() {
        return Err(last("CreateFontW"));
    }
    for control in [edit, list] {
        send(control, WM_SETFONT, font.0 as usize, 0);
    }
    if send(list, LB_SETITEMHEIGHT, 0, px(ROW) as isize) == LB_ERR as isize {
        return Err(last("LB_SETITEMHEIGHT"));
    }

    let len = items.len();
    STATE.set(Some(State {
        items,
        shown: (0..len).collect(),
        cursor: 0,
        edit,
        list,
        dpi,
    }));
    show(list, len)?;
    unsafe {
        _ = ShowWindow(window, SW_SHOW);
        // The foreground lock admits only the process that got the last input, so inject a zero-length mouse move.
        let nudge = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_MOVE,
                    ..Default::default()
                },
            },
        };
        if SendInput(&[nudge], size_of::<INPUT>() as i32) != 1 {
            return Err(last("SendInput"));
        }
        if !SetForegroundWindow(window).as_bool() {
            return Err(Error::Foreground);
        }
        SetFocus(Some(edit)).map_err(win32("SetFocus"))?;
    }
    Ok(window)
}

fn child(
    parent: HWND,
    class: PCWSTR,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND, Error> {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | style,
            x,
            y,
            width,
            height,
            Some(parent),
            None,
            None,
            None,
        )
    }
    .map_err(win32("CreateWindowExW"))
}

extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let result = match message {
        WM_COMMAND => match (wparam.0 >> 16) as u32 {
            EN_CHANGE => refresh(),
            LBN_SELCHANGE => clicked(),
            _ => Ok(()),
        }
        .map(|()| LRESULT(0)),
        // rofi closes when it loses focus.
        WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_INACTIVE => {
            finish(Ok(Choice::Cancel));
            Ok(LRESULT(0))
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => Ok(color(HDC(wparam.0 as _))),
        WM_DRAWITEM => draw(unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) }).map(|()| LRESULT(1)),
        _ => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
    };
    result.unwrap_or_else(|error| {
        finish(Err(error));
        LRESULT(0)
    })
}

/// Handles the keys that steer the picker, returning false for the ones the query line should get.
fn key(key: VIRTUAL_KEY) -> Result<bool, Error> {
    let ctrl = held(VK_CONTROL);
    match key {
        VK_ESCAPE => finish(Ok(Choice::Cancel)),
        VK_RETURN => {
            let query = query()?;
            let choice = match held(VK_SHIFT) {
                true => menu::typed(&query),
                false => with(|state| menu::accept(&state.shown, state.cursor, &query))?,
            };
            finish(Ok(choice));
        }
        VK_DOWN => move_by(1)?,
        VK_UP => move_by(-1)?,
        VK_N if ctrl => move_by(1)?,
        VK_P if ctrl => move_by(-1)?,
        VK_BACK if ctrl => delete_word()?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn finish(outcome: Result<Choice<usize>, Error>) {
    OUTCOME.with_borrow_mut(|slot| {
        if slot.is_none() {
            *slot = Some(outcome);
            unsafe { PostQuitMessage(0) };
        }
    });
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> Result<T, Error> {
    STATE
        .with_borrow_mut(|state| state.as_mut().map(f))
        .ok_or(Error::NoState)
}

fn refresh() -> Result<(), Error> {
    let query = query()?;
    let (list, len) = with(|state| {
        state.shown = menu::filter(&state.items, &query);
        state.cursor = 0;
        (state.list, state.shown.len())
    })?;
    show(list, len)
}

/// Gives the list `len` rows with the first selected.
fn show(list: HWND, len: usize) -> Result<(), Error> {
    if send(list, LB_SETCOUNT, len, 0) < 0 {
        return Err(last("LB_SETCOUNT"));
    }
    match len {
        0 => Ok(()),
        _ => select(list, 0),
    }
}

fn select(list: HWND, row: usize) -> Result<(), Error> {
    match send(list, LB_SETCURSEL, row, 0) == LB_ERR as isize {
        true => Err(last("LB_SETCURSEL")),
        false => Ok(()),
    }
}

fn move_by(by: isize) -> Result<(), Error> {
    let (list, len, row) = with(|state| {
        state.cursor = menu::step(state.cursor, state.shown.len(), by);
        (state.list, state.shown.len(), state.cursor)
    })?;
    match len {
        0 => Ok(()),
        _ => select(list, row),
    }
}

fn clicked() -> Result<(), Error> {
    let list = with(|state| state.list)?;
    // LB_ERR, so no row, when the click lands below the last one.
    let Ok(row) = usize::try_from(send(list, LB_GETCURSEL, 0, 0)) else {
        return Ok(());
    };
    finish(Ok(with(|state| menu::accept(&state.shown, row, ""))?));
    Ok(())
}

fn delete_word() -> Result<(), Error> {
    let edit = with(|state| state.edit)?;
    let text = text(edit)?;
    // EM_GETSEL puts the selection's end, where the caret sits, in the high word.
    let caret = (send(edit, EM_GETSEL, 0, 0) as usize >> 16) & 0xffff;
    let (before, after) = text.split_at(caret.min(text.len()));
    let before = String::from_utf16(before)?;
    let kept: Vec<u16> = menu::delete_word(&before).encode_utf16().collect();
    let joined: Vec<u16> = kept.iter().chain(after).copied().chain([0]).collect();
    unsafe { SetWindowTextW(edit, PCWSTR(joined.as_ptr())) }.map_err(win32("SetWindowTextW"))?;
    send(edit, EM_SETSEL, kept.len(), kept.len() as isize);
    Ok(())
}

fn query() -> Result<String, Error> {
    Ok(String::from_utf16(&text(with(|state| state.edit)?)?)?)
}

fn text(window: HWND) -> Result<Vec<u16>, Error> {
    let len = unsafe { GetWindowTextLengthW(window) }.max(0) as usize;
    let mut buffer = vec![0; len + 1];
    let copied = unsafe { GetWindowTextW(window, &mut buffer) }.max(0) as usize;
    buffer.truncate(copied);
    Ok(buffer)
}

fn color(hdc: HDC) -> LRESULT {
    unsafe {
        SetTextColor(hdc, FOREGROUND);
        SetBkColor(hdc, BACKGROUND);
        SetDCBrushColor(hdc, BACKGROUND);
        LRESULT(GetStockObject(DC_BRUSH).0 as isize)
    }
}

fn draw(item: &DRAWITEMSTRUCT) -> Result<(), Error> {
    let row = item.itemID as usize;
    let label = with(|state| {
        let label = state
            .shown
            .get(row)
            .and_then(|&index| state.items.get(index));
        label.map(|label| (label.encode_utf16().collect::<Vec<u16>>(), state.dpi))
    })?;
    // No label for the focus rectangle of an empty list.
    let Some((mut label, dpi)) = label else {
        return Ok(());
    };
    let hdc = item.hDC;
    let mut rect = item.rcItem;
    unsafe {
        SetDCBrushColor(hdc, BACKGROUND);
        if FillRect(hdc, &rect, HBRUSH(GetStockObject(DC_BRUSH).0)) == 0 {
            return Err(last("FillRect"));
        }
        if item.itemState.0 & ODS_SELECTED.0 != 0 {
            SetDCBrushColor(hdc, SELECTED);
            SetDCPenColor(hdc, SELECTED);
            SelectObject(hdc, GetStockObject(DC_BRUSH));
            SelectObject(hdc, GetStockObject(DC_PEN));
            let radius = scale(RADIUS, dpi);
            RoundRect(
                hdc,
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                radius,
                radius,
            )
            .ok()
            .map_err(win32("RoundRect"))?;
        }
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, FOREGROUND);
        rect.left += scale(INSET, dpi);
        rect.right -= scale(INSET, dpi);
        if !label.is_empty()
            && DrawTextW(
                hdc,
                &mut label,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            ) == 0
        {
            return Err(last("DrawTextW"));
        }
    }
    Ok(())
}

fn held(key: VIRTUAL_KEY) -> bool {
    let state = unsafe { GetKeyState(i32::from(key.0)) };
    state < 0
}

fn send(window: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(window, message, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }.0
}

fn scale(length: i32, dpi: i32) -> i32 {
    length * dpi / 96
}
