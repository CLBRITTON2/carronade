//! The picker window: an input bar over a grid of matches, drawn with Direct2D into a layered window so its rounded
//! corners and translucent colors blend with the desktop.

use std::cell::RefCell;
use std::num::NonZeroUsize;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap, ID2D1Factory,
    ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_TEXT_ALIGNMENT_CENTER, DWriteCreateFactory, IDWriteFactory,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, SelectObject,
};
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PostMessageW, RegisterClassW, SW_SHOW, SetForegroundWindow, ShowWindow,
    TranslateMessage, WM_APP, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Owned, PCWSTR, w};

use crate::config::Config;
use crate::error::{Error, last, win32};
use crate::icons::{self, Cached, Pixels};
use crate::layout::{self, Rect};
use crate::menu::{self, Candidate, Choice, Line};
use crate::platform;
use crate::store::{self, UnixSeconds};
use canvas::{Canvas, MemoryDc, d2d_color, d2d_rect, format, image, measure, rounded};

mod canvas;
mod clipboard;
mod input;

const CLASS: PCWSTR = w!("carronade");

/// What the picker shows for an item: its label, and the picture beside it.
pub trait Row {
    fn label(&self) -> &str;
    /// Matched like the label, a little below it, but never drawn.
    fn alias(&self) -> Option<&str>;
    fn icon(&self) -> Option<Picture>;
    /// Added to the score of the row's matches, and ranks the rows before anything is typed.
    fn boost(&self) -> i32;
}

impl Row for String {
    fn label(&self) -> &str {
        self
    }

    fn alias(&self) -> Option<&str> {
        None
    }

    fn icon(&self) -> Option<Picture> {
        None
    }

    fn boost(&self) -> i32 {
        0
    }
}

/// What goes beside a row's label.
pub enum Picture {
    /// A shell target, shown with the icon the shell has for it.
    Shell(String),
    /// A glyph of `GLYPH_FONT`.
    Glyph(char),
}

/// Installed with every Windows 10 and 11, Server included, unlike its successor Segoe Fluent Icons.
const GLYPH_FONT: &str = "Segoe MDL2 Assets";

/// What the person did in the picker.
pub enum Action<T> {
    Pick(Choice<T>),
    /// Tab or a click on the switch icon.
    Switch,
    /// Ctrl+Enter on a match.
    Terminal(T),
    /// Ctrl+Shift+Enter on a match.
    Admin(T),
}

/// What `browse` does after an `Action`.
pub enum Step<T, R> {
    Done(R),
    /// Replaces the items and the switch icon in the same window, filtered by the query typed so far.
    Show {
        items: Vec<T>,
        switch: String,
    },
    Stay,
}

enum Icon {
    Unloaded(String),
    Loaded(ID2D1Bitmap),
    Glyph(char),
}

/// What the picker keeps of an item.
struct Listed {
    candidate: Candidate,
    icon: Option<Icon>,
}

struct State {
    items: Vec<Listed>,
    /// Indices into `items` of the matches, best first.
    shown: Vec<usize>,
    cursor: usize,
    line: Line,
    /// The glyph at the bar's right end, when there is a list to switch to.
    switch: Option<String>,
    /// A typed high surrogate whose low half has not arrived yet.
    surrogate: Option<u16>,
    /// Where the last mouse move put the pointer, in client coordinates.
    pointer: Option<(i16, i16)>,
    /// The part of a wheel notch turned so far.
    wheel: i32,
    /// `icons.bin`, read when the first page with shell icons shows.
    cache: Option<Vec<Cached>>,
    /// The icons the shell gave this run, for `icons.bin` when the picker closes.
    fetched: Vec<Cached>,
    canvas: Canvas,
}

// Win32 and shell calls re-enter the window procedure, so every borrow ends before the next call that can.
thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static OUTCOME: RefCell<Option<Result<Action<usize>, Error>>> = const { RefCell::new(None) };
}

/// Shows `items` under an input bar until one is picked. Call it once per process: it registers the window class.
pub fn pick<T: Row + Clone>(config: Config, items: Vec<T>) -> Result<Choice<T>, Error> {
    browse(config, items, None, |action| {
        Ok(match action {
            Action::Pick(choice) => Step::Done(choice),
            Action::Switch | Action::Terminal(_) | Action::Admin(_) => Step::Stay,
        })
    })
}

/// Shows `items` under an input bar, with the `switch` glyph at its right end when there is one, asking `next` what
/// each action leads to until it is done. Call it once per process: it registers the window class.
pub fn browse<T: Row + Clone, R>(
    config: Config,
    items: Vec<T>,
    switch: Option<String>,
    next: impl FnMut(Action<T>) -> Result<Step<T, R>, Error>,
) -> Result<R, Error> {
    let window = open(config, listed(&items), switch)?;
    let choice = steps(items, next);
    // SAFETY: `open` created `window` on this thread and nothing destroyed it since.
    unsafe { DestroyWindow(window) }.map_err(win32("DestroyWindow"))?;
    // Released now: a COM object released by the thread-local destructors at exit changes the exit code.
    let icons = STATE.take().map(|state| (state.fetched, state.cache));
    if let Some((fetched, Some(previous))) = icons
        && !fetched.is_empty()
    {
        icons::save(&icons::path()?, &icons::merged(fetched, previous))?;
    }
    choice
}

fn steps<T: Row + Clone, R>(
    items: Vec<T>,
    mut next: impl FnMut(Action<T>) -> Result<Step<T, R>, Error>,
) -> Result<R, Error> {
    let mut items = items;
    loop {
        let item = |row: usize| {
            let len = items.len();
            items.get(row).cloned().ok_or(Error::Row { row, len })
        };
        let action = match pump()? {
            Action::Pick(Choice::Item(row)) => Action::Pick(Choice::Item(item(row)?)),
            Action::Pick(Choice::Text(text)) => Action::Pick(Choice::Text(text)),
            Action::Pick(Choice::Cancel) => Action::Pick(Choice::Cancel),
            Action::Switch => Action::Switch,
            Action::Terminal(row) => Action::Terminal(item(row)?),
            Action::Admin(row) => Action::Admin(item(row)?),
        };
        match next(action)? {
            Step::Done(choice) => return Ok(choice),
            Step::Show {
                items: shown,
                switch,
            } => {
                let listed = listed(&shown);
                with(|state| state.show(listed, switch))?;
                items = shown;
                render()?;
            }
            Step::Stay => {}
        }
    }
}

fn listed<T: Row>(items: &[T]) -> Vec<Listed> {
    items
        .iter()
        .map(|item| Listed {
            candidate: Candidate {
                label: item.label().to_owned(),
                alias: item.alias().map(str::to_owned),
                boost: item.boost(),
            },
            icon: item.icon().map(|picture| match picture {
                Picture::Shell(target) => Icon::Unloaded(target),
                Picture::Glyph(glyph) => Icon::Glyph(glyph),
            }),
        })
        .collect()
}

fn candidates(items: &[Listed]) -> impl Iterator<Item = &Candidate> {
    items.iter().map(|item| &item.candidate)
}

/// Dispatches messages until one leads to an action. It returns right after that message, so keys typed later reach
/// what the action shows next.
fn pump() -> Result<Action<usize>, Error> {
    let mut message = MSG::default();
    loop {
        if let Some(outcome) = OUTCOME.take() {
            return outcome;
        }
        // SAFETY: `message` is a live local the call fills.
        match unsafe { GetMessageW(&mut message, None, 0, 0) }.0 {
            -1 => return Err(last("GetMessageW")),
            0 => return Err(Error::NoChoice),
            _ => {}
        }
        // SAFETY: `message` is the one GetMessageW just filled.
        _ = unsafe { TranslateMessage(&message) };
        // SAFETY: as above, and the window procedure it reaches runs on this thread.
        unsafe { DispatchMessageW(&message) };
    }
}

fn open(config: Config, items: Vec<Listed>, switch: Option<String>) -> Result<HWND, Error> {
    // SAFETY: takes a predefined context constant and runs before this process creates a window.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .map_err(win32("SetProcessDpiAwarenessContext"))?;
    // SAFETY: takes no arguments, and a null result is a valid input to MonitorFromWindow.
    let foreground = unsafe { GetForegroundWindow() };
    // SAFETY: MONITOR_DEFAULTTONEAREST returns a monitor for any window, a stale one included.
    let monitor = unsafe { MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `monitor` came from MonitorFromWindow and `info` is a local with `cbSize` set.
    unsafe { GetMonitorInfoW(monitor, &mut info) }
        .ok()
        .map_err(win32("GetMonitorInfoW"))?;
    let (mut dpi, mut dpi_y) = (0, 0);
    // SAFETY: `monitor` came from MonitorFromWindow and both outputs are live locals.
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) }
        .map_err(win32("GetDpiForMonitor"))?;
    // A DPI is in the hundreds, exact in f32.
    let scale = dpi as f32 / 96.0;
    let em = config.font.size * dpi as f32 / 72.0;

    platform::com()?;
    // SAFETY: the single-threaded factory is used only on this thread, and no debug options are passed.
    let d2d: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
        .map_err(win32("D2D1CreateFactory"))?;
    // SAFETY: takes only a factory type constant.
    let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
        .map_err(win32("DWriteCreateFactory"))?;
    // SAFETY: `platform::com` initialized COM on this thread above.
    let wic: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
            .map_err(win32("CoCreateInstance(WICImagingFactory)"))?;
    let text = format(&dwrite, &config.font.family, em)?;
    let prompt = format(&dwrite, &config.input.prompt_font, em)?;
    let icon = format(&dwrite, &config.input.prompt_font, em)?;
    // SAFETY: `icon` is a live text format and the alignment a defined constant.
    unsafe { icon.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) }
        .map_err(win32("SetTextAlignment"))?;
    // Its glyphs fill most of the em square, while shell icons leave a margin inside theirs.
    let glyph = format(&dwrite, GLYPH_FONT, config.element.icon.px(em, scale) * 0.6)?;
    // SAFETY: `glyph` is a live text format and the alignment a defined constant.
    unsafe { glyph.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) }
        .map_err(win32("SetTextAlignment"))?;
    let typed = measure(&dwrite, &text, &config.input.placeholder)?;
    let prompted = measure(&dwrite, &prompt, &config.input.prompt)?;
    let apps_icon = measure(&dwrite, &prompt, &config.input.apps_icon)?;
    let files_icon = measure(&dwrite, &prompt, &config.input.files_icon)?;
    let line = typed.height.max(prompted.height);
    let layout = layout::measure(
        &config,
        em,
        scale,
        line,
        prompted.widthIncludingTrailingWhitespace,
        apps_icon.width.max(files_icon.width),
    );

    // SAFETY: a null name asks for this exe's own module, which lives as long as the process.
    let instance = unsafe { GetModuleHandleW(None) }.map_err(win32("GetModuleHandleW"))?;
    // SAFETY: IDC_ARROW is a predefined system cursor, which needs no instance.
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.map_err(win32("LoadCursorW"))?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(input::window_proc),
        hInstance: instance.into(),
        hCursor: cursor,
        lpszClassName: CLASS,
        ..Default::default()
    };
    // SAFETY: `class` is filled in, and `CLASS` is a static wide string that outlives the registration.
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(last("RegisterClassW"));
    }
    // Whole pixels from `ceil`, a screen's size at most.
    let (width, height) = (layout.width as i32, layout.height as i32);
    let work = info.rcWork;
    let origin = POINT {
        x: work.left + (work.right - work.left - width) / 2,
        y: work.top + (work.bottom - work.top - height) / 2,
    };
    // SAFETY: `CLASS` was registered above with this `instance`, and no creation data is passed.
    let window = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            CLASS,
            WS_POPUP,
            origin.x,
            origin.y,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(win32("CreateWindowExW"))?;

    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        // Negative for a top-down bitmap, the row order Direct2D writes.
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let bitmap_info = BITMAPINFO {
        bmiHeader: header,
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    // Created before `dc`, so on an early return `dc` drops first and releases it. DIB_RGB_COLORS needs no DC.
    // SAFETY: `bitmap_info` describes a 32-bit DIB and `bits` is a live local the call fills.
    let section =
        unsafe { CreateDIBSection(None, &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0) }
            .map_err(win32("CreateDIBSection"))?;
    // SAFETY: `section` is a fresh bitmap nothing else owns, so `Owned` deletes it once.
    let dib = unsafe { Owned::new(section) };
    // SAFETY: a null DC asks for one compatible with the screen.
    let dc = MemoryDc(unsafe { CreateCompatibleDC(None) });
    if dc.0.is_invalid() {
        return Err(last("CreateCompatibleDC"));
    }
    // SAFETY: both handles are valid, and `dc` drops before `dib`, so the bitmap is never deleted while selected.
    if unsafe { SelectObject(dc.0, (*dib).into()) }.is_invalid() {
        return Err(last("SelectObject"));
    }
    let properties = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        // At 96 DPI one Direct2D unit is one pixel, the unit the layout is in.
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    // SAFETY: `properties` is a filled-in local that outlives the call.
    let target =
        unsafe { d2d.CreateDCRenderTarget(&properties) }.map_err(win32("CreateDCRenderTarget"))?;
    let bounds = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    // SAFETY: `dc` holds the DIB the size of `bounds`, and the canvas keeps both alive as long as `target`.
    unsafe { target.BindDC(dc.0, &bounds) }.map_err(win32("ID2D1DCRenderTarget::BindDC"))?;
    // ClearType needs an opaque background to blend against.
    // SAFETY: `target` is live and the mode a defined constant.
    unsafe { target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
    // SAFETY: `target` is live and the color a local that outlives the call.
    let brush = unsafe { target.CreateSolidColorBrush(&d2d_color(config.input.color), None) }
        .map_err(win32("CreateSolidColorBrush"))?;
    // SAFETY: as for `brush`.
    let highlight =
        unsafe { target.CreateSolidColorBrush(&d2d_color(config.element.highlight), None) }
            .map_err(win32("CreateSolidColorBrush"))?;
    let image = match &config.window.image {
        Some(path) => Some(image(&wic, &target, path, layout.width, layout.height)?),
        None => None,
    };

    STATE.set(Some(State {
        shown: menu::filter(candidates(&items), ""),
        items,
        cursor: 0,
        line: Line::default(),
        switch,
        surrogate: None,
        pointer: None,
        wheel: 0,
        cache: None,
        fetched: Vec::new(),
        canvas: Canvas {
            window,
            origin,
            target,
            brush,
            highlight,
            dwrite,
            wic,
            text,
            prompt,
            icon,
            glyph,
            image,
            layout,
            config,
            em,
            scale,
            dc,
            dib,
        },
    }));
    // The first frame goes up without icons, which take tens of ms each to load.
    with(State::draw)??;
    // SAFETY: `window` was created above on this thread.
    _ = unsafe { ShowWindow(window, SW_SHOW) };
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
    // SAFETY: `nudge` is a mouse input with the `mi` member set, and the size passed is that of `INPUT`.
    if unsafe { SendInput(&[nudge], size_of::<INPUT>() as i32) } != 1 {
        return Err(last("SendInput"));
    }
    // SAFETY: `window` was created above on this thread.
    if !unsafe { SetForegroundWindow(window) }.as_bool() {
        return Err(Error::Foreground);
    }
    render()?;
    Ok(window)
}

/// Records the outcome for `pump`, keeping the first when two arrive before it looks.
fn finish(outcome: Result<Action<usize>, Error>) -> Result<(), Error> {
    let first = OUTCOME.with_borrow_mut(|slot| {
        if slot.is_some() {
            return false;
        }
        *slot = Some(outcome);
        true
    });
    if first {
        // A thread message: `pump` stays in GetMessageW after a sent message such as WM_ACTIVATE until one arrives.
        // SAFETY: a null window posts to this thread's queue, and WM_APP carries no pointers.
        unsafe { PostMessageW(None, WM_APP, WPARAM(0), LPARAM(0)) }
            .map_err(win32("PostMessageW"))?;
    }
    Ok(())
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> Result<T, Error> {
    STATE.with(|cell| {
        let mut state = cell.try_borrow_mut().map_err(|_| Error::Reentered)?;
        state.as_mut().map(f).ok_or(Error::NoState)
    })
}

/// Loads the icons of the page on show, from `icons.bin` while they are fresh there, then draws it.
fn render() -> Result<(), Error> {
    let (pending, side, wic) = with(|state| {
        let canvas = &state.canvas;
        (state.unloaded(), canvas.icon_size(), canvas.wic.clone())
    })?;
    if !pending.is_empty() && with(|state| state.cache.is_none())? {
        let cache = icons::load(&icons::path()?)?;
        with(|state| state.cache = Some(cache))?;
    }
    let now = store::now()?;
    for (index, target) in pending {
        let pixels = if let Some(pixels) = with(|state| state.cached(&target, side, now))? {
            pixels
        } else {
            // Outside the borrow: the shell pumps messages while it loads.
            let pixels = icons::fetch(&wic, &target, side)?;
            let fetched = Cached {
                target,
                fetched: now,
                pixels: pixels.clone(),
            };
            with(|state| state.fetched.push(fetched))?;
            pixels
        };
        with(|state| {
            state
                .canvas
                .bitmap(&pixels)
                .map(|bitmap| state.set_icon(index, bitmap))
        })??;
    }
    with(State::draw)?
}

impl State {
    fn page(&self) -> NonZeroUsize {
        let list = &self.canvas.config.list;
        list.columns.saturating_mul(list.lines)
    }

    /// The matches on the page that holds the cursor, as indices into `items`.
    fn on_page(&self) -> &[usize] {
        let first = menu::first(self.cursor, self.page());
        let rest = self.shown.get(first..).unwrap_or_default();
        rest.get(..self.page().get()).unwrap_or(rest)
    }

    /// The icon for `target` the shell gave this run, else the one `icons.bin` holds while it is fresh.
    fn cached(&self, target: &str, side: u32, now: UnixSeconds) -> Option<Pixels> {
        let this_run = icons::find(&self.fetched, target, side, now);
        let saved = || icons::find(self.cache.as_deref()?, target, side, now);
        this_run.or_else(saved).cloned()
    }

    fn unloaded(&self) -> Vec<(usize, String)> {
        self.on_page()
            .iter()
            .filter_map(
                |&index| match self.items.get(index).map(|item| &item.icon) {
                    Some(Some(Icon::Unloaded(target))) => Some((index, target.clone())),
                    _ => None,
                },
            )
            .collect()
    }

    fn show(&mut self, items: Vec<Listed>, switch: String) {
        self.shown = menu::filter(candidates(&items), &self.line.text());
        self.items = items;
        self.cursor = 0;
        self.switch = Some(switch);
    }

    fn set_icon(&mut self, index: usize, bitmap: ID2D1Bitmap) {
        if let Some(item) = self.items.get_mut(index) {
            item.icon = Some(Icon::Loaded(bitmap));
        }
    }

    fn draw(&mut self) -> Result<(), Error> {
        let canvas = &self.canvas;
        let (layout, config) = (&canvas.layout, &canvas.config);
        let target: &ID2D1RenderTarget = &canvas.target;
        // SAFETY: `target` is bound to the canvas DC, on the thread that made it.
        unsafe { target.BeginDraw() };
        // SAFETY: inside the BeginDraw above, with a color that outlives the call.
        unsafe { target.Clear(Some(&D2D1_COLOR_F::default())) };
        let border = layout.border;
        let outside = Rect {
            left: 0.0,
            top: 0.0,
            right: layout.width,
            bottom: layout.height,
        };
        let frame = outside.inset(border);
        let radius = canvas.px(config.window.radius);
        canvas.fill(frame, radius - border, config.window.background);
        if let Some(image) = &canvas.image {
            // SAFETY: `image` is a brush made by this `target`, and the rectangle a local.
            unsafe { target.FillRoundedRectangle(&rounded(frame, radius - border), image) };
        }
        // The list's background takes the frame's rounded bottom corners and a square top edge.
        // SAFETY: inside the draw, and popped right after the fill below.
        unsafe { target.PushAxisAlignedClip(&d2d_rect(layout.list), D2D1_ANTIALIAS_MODE_ALIASED) };
        canvas.fill(frame, radius - border, config.list.background);
        // SAFETY: pops the clip pushed above, inside the same draw.
        unsafe { target.PopAxisAlignedClip() };
        // SAFETY: the brush is live and the color a local that outlives the call.
        unsafe {
            canvas
                .brush
                .SetColor(&d2d_color(config.window.border_color))
        };
        // SAFETY: inside the draw, with a brush made by this `target`.
        unsafe {
            target.DrawRoundedRectangle(
                &rounded(outside.inset(border / 2.0), radius - border / 2.0),
                &canvas.brush,
                border,
                None,
            );
        }

        let input = &config.input;
        canvas.fill(layout.input, canvas.px(input.radius), input.background);
        canvas.text(&input.prompt, &canvas.prompt, layout.prompt, input.color);
        let typed = self.line.text();
        let caret = match typed.as_str() {
            "" => {
                canvas.text(
                    &input.placeholder,
                    &canvas.text,
                    layout.entry,
                    input.placeholder_color,
                );
                Ok(0.0)
            }
            _ => canvas.entry(&typed, self.line.caret()),
        }?;
        let entry = layout.entry;
        let (top, bottom) = (entry.top, entry.bottom);
        let caret_left = entry.left + caret;
        canvas.fill(
            Rect {
                left: caret_left,
                top,
                right: caret_left + canvas.scale.max(1.0),
                bottom,
            },
            0.0,
            input.color,
        );
        if let Some(switch) = &self.switch {
            canvas.text(switch, &canvas.icon, layout.switch, input.placeholder_color);
        }

        let element = &config.element;
        let first = menu::first(self.cursor, self.page());
        for ((slot, cell), &index) in layout.cells.iter().enumerate().zip(self.on_page()) {
            if first + slot == self.cursor {
                canvas.fill(cell.area, canvas.px(element.radius), element.selected);
            }
            let Some(item) = self.items.get(index) else {
                continue;
            };
            let label = match &item.icon {
                Some(Icon::Loaded(bitmap)) => {
                    // Whole pixels at the bitmap's own size, so it is copied rather than resampled.
                    // SAFETY: `bitmap` is a live bitmap made by this `target`.
                    let size = unsafe { bitmap.GetSize() };
                    let (left, top) = (cell.icon.left.round(), cell.icon.top.round());
                    let icon = Rect {
                        left,
                        top,
                        right: left + size.width,
                        bottom: top + size.height,
                    };
                    // SAFETY: inside the draw, with a bitmap made by this `target`.
                    unsafe {
                        target.DrawBitmap(
                            bitmap,
                            Some(&d2d_rect(icon)),
                            1.0,
                            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                            None,
                        );
                    };
                    cell.label
                }
                Some(Icon::Glyph(glyph)) => {
                    let glyph = glyph.to_string();
                    canvas.text(&glyph, &canvas.glyph, cell.icon, element.color);
                    cell.label
                }
                _ => Rect {
                    left: cell.icon.left,
                    ..cell.label
                },
            };
            let text = &item.candidate.label;
            canvas.label(text, &menu::matched(text, &typed), label)?;
        }
        // SAFETY: ends the draw begun above, and the tag outputs are optional.
        unsafe { target.EndDraw(None, None) }.map_err(win32("ID2D1RenderTarget::EndDraw"))?;
        canvas.show()
    }
}
