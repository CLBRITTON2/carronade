//! The picker window: an input bar over a grid of matches, drawn with Direct2D into a layered window so its rounded
//! corners and translucent colors blend with the desktop.

use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::path::Path;

use windows::Win32::Foundation::{
    GENERIC_READ, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap,
    ID2D1BitmapBrush, ID2D1DCRenderTarget, ID2D1Factory, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_HIT_TEST_METRICS, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, GetMonitorInfoW, HBITMAP,
    HDC, HPALETTE, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, SelectObject,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant,
    WICBitmapInterpolationModeHighQualityCubic, WICBitmapPaletteTypeMedianCut,
    WICBitmapUsePremultipliedAlpha, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput, VIRTUAL_KEY,
    VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_N, VK_P,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetMessageW, IDC_ARROW, LoadCursorW, MSG, PostMessageW, RegisterClassW, SW_SHOW,
    SetForegroundWindow, ShowWindow, TranslateMessage, ULW_ALPHA, UpdateLayeredWindow, WA_INACTIVE,
    WM_ACTIVATE, WM_APP, WM_CHAR, WM_KEYDOWN, WM_LBUTTONDOWN, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};
use windows_numerics::Matrix3x2;

use crate::apps;
use crate::config::{Color, Config, Length};
use crate::error::{Error, last, win32};
use crate::layout::{self, Layout, Rect};
use crate::menu::{self, Choice, Line};

const CLASS: PCWSTR = w!("carronade");

/// What the picker shows for an item: its label, and the shell target whose icon goes beside it.
pub trait Row {
    fn label(&self) -> &str;
    fn icon(&self) -> Option<String>;
}

impl Row for String {
    fn label(&self) -> &str {
        self
    }

    fn icon(&self) -> Option<String> {
        None
    }
}

/// What the person did in the picker.
pub enum Action<T> {
    Pick(Choice<T>),
    /// Tab or a click on the switch icon.
    Switch,
}

/// What `browse` does after an `Action`.
pub enum Step<T> {
    Done(Choice<T>),
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
}

struct State {
    items: Vec<String>,
    icons: Vec<Option<Icon>>,
    shown: Vec<usize>,
    cursor: usize,
    line: Line,
    /// The glyph at the bar's right end, when there is a list to switch to.
    switch: Option<String>,
    /// A typed high surrogate whose low half has not arrived yet.
    surrogate: Option<u16>,
    canvas: Canvas,
}

/// The window and everything that draws into it.
struct Canvas {
    window: HWND,
    origin: POINT,
    dc: HDC,
    target: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    text: IDWriteTextFormat,
    prompt: IDWriteTextFormat,
    /// The prompt font, centered for the switch icon.
    icon: IDWriteTextFormat,
    image: Option<ID2D1BitmapBrush>,
    layout: Layout,
    config: Config,
    em: f32,
    scale: f32,
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
            Action::Switch => Step::Stay,
        })
    })
}

/// Shows `items` under an input bar, with the `switch` glyph at its right end when there is one, asking `next` what
/// each action leads to until it is done. Call it once per process: it registers the window class.
pub fn browse<T: Row + Clone>(
    config: Config,
    items: Vec<T>,
    switch: Option<String>,
    next: impl FnMut(Action<T>) -> Result<Step<T>, Error>,
) -> Result<Choice<T>, Error> {
    let (labels, icons) = rows(&items);
    let window = open(config, labels, icons, switch)?;
    let choice = steps(items, next);
    unsafe { DestroyWindow(window) }.map_err(win32("DestroyWindow"))?;
    // Released now: a COM object released by the thread-local destructors at exit changes the exit code.
    STATE.take();
    choice
}

fn steps<T: Row + Clone>(
    items: Vec<T>,
    mut next: impl FnMut(Action<T>) -> Result<Step<T>, Error>,
) -> Result<Choice<T>, Error> {
    let mut items = items;
    loop {
        let action = match pump()? {
            Action::Pick(Choice::Item(row)) => {
                let len = items.len();
                let item = items.get(row).cloned().ok_or(Error::Row { row, len })?;
                Action::Pick(Choice::Item(item))
            }
            Action::Pick(Choice::Text(text)) => Action::Pick(Choice::Text(text)),
            Action::Pick(Choice::Cancel) => Action::Pick(Choice::Cancel),
            Action::Switch => Action::Switch,
        };
        match next(action)? {
            Step::Done(choice) => return Ok(choice),
            Step::Show {
                items: shown,
                switch,
            } => {
                let (labels, icons) = rows(&shown);
                with(|state| state.show(labels, icons, switch))?;
                items = shown;
                render()?;
            }
            Step::Stay => {}
        }
    }
}

fn rows<T: Row>(items: &[T]) -> (Vec<String>, Vec<Option<Icon>>) {
    items
        .iter()
        .map(|item| (item.label().to_owned(), item.icon().map(Icon::Unloaded)))
        .unzip()
}

/// Dispatches messages until one leads to an action. It returns right after that message, so keys typed later reach
/// what the action shows next.
fn pump() -> Result<Action<usize>, Error> {
    let mut message = MSG::default();
    loop {
        if let Some(outcome) = OUTCOME.take() {
            return outcome;
        }
        match unsafe { GetMessageW(&mut message, None, 0, 0) }.0 {
            -1 => return Err(last("GetMessageW")),
            0 => return Err(Error::NoChoice),
            _ => {}
        }
        unsafe {
            _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn open(
    config: Config,
    items: Vec<String>,
    icons: Vec<Option<Icon>>,
    switch: Option<String>,
) -> Result<HWND, Error> {
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
    let scale = dpi as f32 / 96.0;
    let em = config.font.size * dpi as f32 / 72.0;

    apps::com()?;
    let d2d: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
        .map_err(win32("D2D1CreateFactory"))?;
    let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
        .map_err(win32("DWriteCreateFactory"))?;
    let wic: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
            .map_err(win32("CoCreateInstance(WICImagingFactory)"))?;
    let text = format(&dwrite, &config.font.family, em)?;
    let prompt = format(&dwrite, &config.input.prompt_font, em)?;
    let icon = format(&dwrite, &config.input.prompt_font, em)?;
    unsafe { icon.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) }
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

    let instance = unsafe { GetModuleHandleW(None) }.map_err(win32("GetModuleHandleW"))?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.map_err(win32("LoadCursorW"))?,
        lpszClassName: CLASS,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(last("RegisterClassW"));
    }
    let (width, height) = (layout.width as i32, layout.height as i32);
    let work = info.rcWork;
    let origin = POINT {
        x: work.left + (work.right - work.left - width) / 2,
        y: work.top + (work.bottom - work.top - height) / 2,
    };
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

    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.is_invalid() {
        return Err(last("CreateCompatibleDC"));
    }
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
    let dib =
        unsafe { CreateDIBSection(Some(dc), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0) }
            .map_err(win32("CreateDIBSection"))?;
    if unsafe { SelectObject(dc, dib.into()) }.is_invalid() {
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
    let target =
        unsafe { d2d.CreateDCRenderTarget(&properties) }.map_err(win32("CreateDCRenderTarget"))?;
    let bounds = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    unsafe { target.BindDC(dc, &bounds) }.map_err(win32("ID2D1DCRenderTarget::BindDC"))?;
    // ClearType needs an opaque background to blend against.
    unsafe { target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
    let brush = unsafe { target.CreateSolidColorBrush(&d2d_color(config.input.color), None) }
        .map_err(win32("CreateSolidColorBrush"))?;
    let image = match &config.window.image {
        Some(path) => Some(image(&wic, &target, path, layout.width, layout.height)?),
        None => None,
    };

    let len = items.len();
    STATE.set(Some(State {
        items,
        icons,
        shown: (0..len).collect(),
        cursor: 0,
        line: Line::default(),
        switch,
        surrogate: None,
        canvas: Canvas {
            window,
            origin,
            dc,
            target,
            brush,
            dwrite,
            wic,
            text,
            prompt,
            icon,
            image,
            layout,
            config,
            em,
            scale,
        },
    }));
    // The first frame goes up without icons, which take tens of ms each to load.
    with(State::draw)??;
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
    }
    render()?;
    Ok(window)
}

/// A single-line text format of `family` at `size` px that ends overlong text with an ellipsis.
fn format(dwrite: &IDWriteFactory, family: &str, size: f32) -> Result<IDWriteTextFormat, Error> {
    let mut fonts: Option<IDWriteFontCollection> = None;
    unsafe { dwrite.GetSystemFontCollection(&mut fonts, false) }
        .map_err(win32("GetSystemFontCollection"))?;
    let (mut index, mut exists) = (0, false.into());
    let name = HSTRING::from(family);
    if let Some(fonts) = fonts {
        unsafe { fonts.FindFamilyName(&name, &mut index, &mut exists) }
            .map_err(win32("IDWriteFontCollection::FindFamilyName"))?;
    }
    if !exists.as_bool() {
        return Err(Error::Font(family.to_owned()));
    }
    let format = unsafe {
        dwrite.CreateTextFormat(
            &name,
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )
    }
    .map_err(win32("CreateTextFormat"))?;
    let ellipsis = unsafe { dwrite.CreateEllipsisTrimmingSign(&format) }
        .map_err(win32("CreateEllipsisTrimmingSign"))?;
    let trimming = DWRITE_TRIMMING {
        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
        delimiter: 0,
        delimiterCount: 0,
    };
    unsafe {
        format
            .SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)
            .map_err(win32("SetWordWrapping"))?;
        format
            .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)
            .map_err(win32("SetParagraphAlignment"))?;
        format
            .SetTrimming(&trimming, &ellipsis)
            .map_err(win32("SetTrimming"))?;
    }
    Ok(format)
}

/// `text` in `format`, unbounded so nothing trims it, with its box `height` tall.
fn text_layout(
    dwrite: &IDWriteFactory,
    format: &IDWriteTextFormat,
    text: &str,
    height: f32,
) -> Result<IDWriteTextLayout, Error> {
    let units: Vec<u16> = text.encode_utf16().collect();
    unsafe { dwrite.CreateTextLayout(&units, format, f32::MAX, height) }
        .map_err(win32("CreateTextLayout"))
}

fn measure(
    dwrite: &IDWriteFactory,
    format: &IDWriteTextFormat,
    text: &str,
) -> Result<DWRITE_TEXT_METRICS, Error> {
    let layout = text_layout(dwrite, format, text, 0.0)?;
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut metrics) }.map_err(win32("IDWriteTextLayout::GetMetrics"))?;
    Ok(metrics)
}

/// The picture at `path` as a brush that covers a `width` by `height` window, cropping what overflows.
fn image(
    wic: &IWICImagingFactory,
    target: &ID2D1RenderTarget,
    path: &Path,
    width: f32,
    height: f32,
) -> Result<ID2D1BitmapBrush, Error> {
    let failed = |source| Error::Image {
        path: path.to_owned(),
        source,
    };
    let brush = unsafe {
        let decoder = wic
            .CreateDecoderFromFilename(
                &HSTRING::from(path),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(failed)?;
        let frame = decoder.GetFrame(0).map_err(failed)?;
        let (mut source_width, mut source_height) = (0, 0);
        frame
            .GetSize(&mut source_width, &mut source_height)
            .map_err(failed)?;
        // Decoded straight to the size it covers the window at: a photo at full size is tens of MB and slow to decode.
        let zoom = (width / source_width as f32).max(height / source_height as f32);
        let scaler = wic.CreateBitmapScaler().map_err(failed)?;
        scaler
            .Initialize(
                &frame,
                (source_width as f32 * zoom).ceil() as u32,
                (source_height as f32 * zoom).ceil() as u32,
                WICBitmapInterpolationModeFant,
            )
            .map_err(failed)?;
        let converter = wic.CreateFormatConverter().map_err(failed)?;
        converter
            .Initialize(
                &scaler,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )
            .map_err(failed)?;
        let bitmap = target
            .CreateBitmapFromWicBitmap(&converter, None)
            .map_err(failed)?;
        let brush = target
            .CreateBitmapBrush(&bitmap, None, None)
            .map_err(failed)?;
        let size = bitmap.GetSize();
        // Centered on whole pixels, so the bitmap is copied rather than resampled.
        brush.SetTransform(&Matrix3x2::translation(
            ((width - size.width) / 2.0).round(),
            ((height - size.height) / 2.0).round(),
        ));
        brush
    };
    Ok(brush)
}

extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let result = match message {
        WM_KEYDOWN => match key(VIRTUAL_KEY(wparam.0 as u16)) {
            Ok(false) => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
            handled => handled.map(|_| ()),
        },
        WM_CHAR => typed(wparam.0 as u16),
        // The low and high words are signed client coordinates.
        WM_LBUTTONDOWN => clicked(
            f32::from(lparam.0 as i16),
            f32::from((lparam.0 >> 16) as i16),
        ),
        WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_INACTIVE => {
            finish(Ok(Action::Pick(Choice::Cancel)))
        }
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
    match key {
        VK_ESCAPE => finish(Ok(Action::Pick(Choice::Cancel)))?,
        VK_RETURN => {
            let shift = held(VK_SHIFT);
            let choice = with(|state| {
                let query = state.line.text();
                match shift {
                    true => menu::typed(&query),
                    false => menu::accept(&state.shown, state.cursor, &query),
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

/// Inserts a typed UTF-16 unit. Control characters, which Backspace and Enter also type, insert nothing.
fn typed(unit: u16) -> Result<(), Error> {
    let units = with(|state| match (state.surrogate.take(), unit) {
        (_, 0xd800..=0xdbff) => {
            state.surrogate = Some(unit);
            Vec::new()
        }
        (Some(high), _) => vec![high, unit],
        (None, _) => vec![unit],
    })?;
    let text = String::from_utf16(&units)?;
    edit(|line| line.insert(&text))
}

/// Applies `change` to the query, filtering again when its text changed.
fn edit(change: impl FnOnce(&Line) -> Line) -> Result<(), Error> {
    with(|state| {
        let line = change(&state.line);
        if line.text() != state.line.text() {
            state.shown = menu::filter(&state.items, &line.text());
            state.cursor = 0;
        }
        state.line = line;
    })?;
    render()
}

fn move_by(by: isize) -> Result<(), Error> {
    with(|state| state.cursor = menu::step(state.cursor, state.shown.len(), by))?;
    render()
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

/// The clipboard's text, or nothing when it holds none.
fn clipboard() -> Result<String, Error> {
    let format = u32::from(CF_UNICODETEXT.0);
    if unsafe { IsClipboardFormatAvailable(format) }.is_err() {
        return Ok(String::new());
    }
    unsafe { OpenClipboard(None) }.map_err(win32("OpenClipboard"))?;
    let text = clipboard_text(format);
    unsafe { CloseClipboard() }.map_err(win32("CloseClipboard"))?;
    text
}

fn clipboard_text(format: u32) -> Result<String, Error> {
    let handle = unsafe { GetClipboardData(format) }.map_err(win32("GetClipboardData"))?;
    let global = HGLOBAL(handle.0);
    let data = unsafe { GlobalLock(global) }.cast::<u16>();
    if data.is_null() {
        return Err(last("GlobalLock"));
    }
    let len = (0..)
        .take_while(|&at| unsafe { *data.add(at) } != 0)
        .count();
    let text = String::from_utf16(unsafe { std::slice::from_raw_parts(data, len) });
    // GlobalUnlock reports releasing the last lock as a failure, so its result says nothing.
    _ = unsafe { GlobalUnlock(global) };
    Ok(text?)
}

/// Records the outcome for `pump`, keeping the first when two arrive before it looks.
fn finish(outcome: Result<Action<usize>, Error>) -> Result<(), Error> {
    let first = OUTCOME.with_borrow_mut(|slot| match slot {
        Some(_) => false,
        None => {
            *slot = Some(outcome);
            true
        }
    });
    if first {
        // A thread message: `pump` stays in GetMessageW after a sent message such as WM_ACTIVATE until one arrives.
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

/// Loads the icons of the page on show, then draws it.
fn render() -> Result<(), Error> {
    let (pending, size) = with(|state| (state.unloaded(), state.canvas.icon_size()))?;
    for (index, target) in pending {
        // Outside the borrow: the shell pumps messages while it loads.
        let icon = apps::display_icon(&target, size)?;
        with(|state| {
            state
                .canvas
                .bitmap(icon, size)
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

    fn unloaded(&self) -> Vec<(usize, String)> {
        self.on_page()
            .iter()
            .filter_map(|&index| match self.icons.get(index) {
                Some(Some(Icon::Unloaded(target))) => Some((index, target.clone())),
                _ => None,
            })
            .collect()
    }

    fn show(&mut self, items: Vec<String>, icons: Vec<Option<Icon>>, switch: String) {
        self.shown = menu::filter(&items, &self.line.text());
        self.items = items;
        self.icons = icons;
        self.cursor = 0;
        self.switch = Some(switch);
    }

    fn set_icon(&mut self, index: usize, bitmap: ID2D1Bitmap) {
        if let Some(slot) = self.icons.get_mut(index) {
            *slot = Some(Icon::Loaded(bitmap));
        }
    }

    fn draw(&mut self) -> Result<(), Error> {
        let canvas = &self.canvas;
        let (layout, config) = (&canvas.layout, &canvas.config);
        let target: &ID2D1RenderTarget = &canvas.target;
        unsafe {
            target.BeginDraw();
            target.Clear(Some(&D2D1_COLOR_F::default()));
        }
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
            unsafe { target.FillRoundedRectangle(&rounded(frame, radius - border), image) };
        }
        // The list's background takes the frame's rounded bottom corners and a square top edge.
        unsafe { target.PushAxisAlignedClip(&d2d_rect(layout.list), D2D1_ANTIALIAS_MODE_ALIASED) };
        canvas.fill(frame, radius - border, config.list.background);
        unsafe {
            target.PopAxisAlignedClip();
            canvas
                .brush
                .SetColor(&d2d_color(config.window.border_color));
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
            let label = match self.icons.get(index) {
                Some(Some(Icon::Loaded(bitmap))) => {
                    // Whole pixels at the bitmap's own size, so it is copied rather than resampled.
                    let size = unsafe { bitmap.GetSize() };
                    let (left, top) = (cell.icon.left.round(), cell.icon.top.round());
                    let icon = Rect {
                        left,
                        top,
                        right: left + size.width,
                        bottom: top + size.height,
                    };
                    unsafe {
                        target.DrawBitmap(
                            bitmap,
                            Some(&d2d_rect(icon)),
                            1.0,
                            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                            None,
                        )
                    };
                    cell.label
                }
                _ => Rect {
                    left: cell.icon.left,
                    ..cell.label
                },
            };
            let text = self.items.get(index).map_or("", String::as_str);
            canvas.text(text, &canvas.text, label, element.color);
        }
        unsafe { target.EndDraw(None, None) }.map_err(win32("ID2D1RenderTarget::EndDraw"))?;
        canvas.show()
    }
}

impl Canvas {
    fn px(&self, length: Length) -> f32 {
        length.px(self.em, self.scale)
    }

    fn icon_size(&self) -> i32 {
        self.px(self.config.element.icon).round() as i32
    }

    fn fill(&self, rect: Rect, radius: f32, color: Color) {
        unsafe {
            self.brush.SetColor(&d2d_color(color));
            self.target
                .FillRoundedRectangle(&rounded(rect, radius), &self.brush);
        }
    }

    fn text(&self, text: &str, format: &IDWriteTextFormat, rect: Rect, color: Color) {
        let units: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.brush.SetColor(&d2d_color(color));
            self.target.DrawText(
                &units,
                format,
                &d2d_rect(rect),
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// Draws the typed text, scrolled left far enough to keep the caret in view, and returns the caret's offset from
    /// the entry's left edge.
    fn entry(&self, typed: &str, caret: usize) -> Result<f32, Error> {
        let entry = self.layout.entry;
        let text = text_layout(&self.dwrite, &self.text, typed, entry.bottom - entry.top)?;
        let (mut x, mut y, mut hit) = (0.0, 0.0, DWRITE_HIT_TEST_METRICS::default());
        unsafe { text.HitTestTextPosition(caret as u32, false, &mut x, &mut y, &mut hit) }
            .map_err(win32("IDWriteTextLayout::HitTestTextPosition"))?;
        let scroll = (x - (entry.right - entry.left)).max(0.0);
        unsafe {
            self.brush.SetColor(&d2d_color(self.config.input.color));
            self.target
                .PushAxisAlignedClip(&d2d_rect(entry), D2D1_ANTIALIAS_MODE_ALIASED);
            self.target.DrawTextLayout(
                windows_numerics::Vector2 {
                    X: entry.left - scroll,
                    Y: entry.top,
                },
                &text,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
            self.target.PopAxisAlignedClip();
        }
        Ok(x - scroll)
    }

    /// An icon from the shell as a `size` px square bitmap for this canvas. Frees `icon`.
    fn bitmap(&self, icon: HBITMAP, size: i32) -> Result<ID2D1Bitmap, Error> {
        let bitmap = unsafe {
            self.wic.CreateBitmapFromHBITMAP(
                icon,
                HPALETTE::default(),
                WICBitmapUsePremultipliedAlpha,
            )
        }
        .map_err(win32("CreateBitmapFromHBITMAP"))
        .and_then(|wic| {
            let scaler =
                unsafe { self.wic.CreateBitmapScaler() }.map_err(win32("CreateBitmapScaler"))?;
            let side = size.unsigned_abs();
            unsafe {
                scaler.Initialize(&wic, side, side, WICBitmapInterpolationModeHighQualityCubic)
            }
            .map_err(win32("IWICBitmapScaler::Initialize"))?;
            unsafe { self.target.CreateBitmapFromWicBitmap(&scaler, None) }
                .map_err(win32("CreateBitmapFromWicBitmap"))
        });
        unsafe { DeleteObject(icon.into()) }
            .ok()
            .map_err(win32("DeleteObject"))?;
        bitmap
    }

    /// Puts the drawn frame on screen.
    fn show(&self) -> Result<(), Error> {
        let size = SIZE {
            cx: self.layout.width as i32,
            cy: self.layout.height as i32,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                self.window,
                None,
                Some(&self.origin),
                Some(&size),
                Some(self.dc),
                Some(&POINT::default()),
                Default::default(),
                Some(&blend),
                ULW_ALPHA,
            )
        }
        .map_err(win32("UpdateLayeredWindow"))
    }
}

/// `rect` with corners of `radius`, kept small enough to fit.
fn rounded(rect: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
    let radius = radius
        .min((rect.right - rect.left) / 2.0)
        .min((rect.bottom - rect.top) / 2.0)
        .max(0.0);
    D2D1_ROUNDED_RECT {
        rect: d2d_rect(rect),
        radiusX: radius,
        radiusY: radius,
    }
}

fn d2d_rect(rect: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn d2d_color(color: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: color.red,
        g: color.green,
        b: color.blue,
        a: color.alpha,
    }
}

fn held(key: VIRTUAL_KEY) -> bool {
    let state = unsafe { GetKeyState(i32::from(key.0)) };
    state < 0
}
