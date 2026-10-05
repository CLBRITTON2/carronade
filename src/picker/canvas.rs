//! What draws the picker: the Direct2D target over the window's bitmap, and its brushes, text formats and pictures.

use std::path::Path;

use windows::Win32::Foundation::{COLORREF, GENERIC_READ, HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_ROUNDED_RECT, ID2D1Bitmap, ID2D1BitmapBrush, ID2D1DCRenderTarget, ID2D1RenderTarget,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_HIT_TEST_METRICS, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, DeleteDC, HBITMAP, HDC,
};
use windows::Win32::Graphics::Imaging::{
    GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::{HSTRING, Owned, w};
use windows_numerics::Matrix3x2;

use crate::config::{Color, Config, Length};
use crate::error::{Error, win32};
use crate::icons::Pixels;
use crate::layout::{Layout, Rect};

/// The window and everything that draws into it.
pub(super) struct Canvas {
    pub(super) window: HWND,
    pub(super) origin: POINT,
    pub(super) target: ID2D1DCRenderTarget,
    pub(super) brush: ID2D1SolidColorBrush,
    /// The matched letters' color, its own brush since a text layout keeps the brush, not its color.
    pub(super) highlight: ID2D1SolidColorBrush,
    pub(super) dwrite: IDWriteFactory,
    pub(super) wic: IWICImagingFactory,
    pub(super) text: IDWriteTextFormat,
    pub(super) prompt: IDWriteTextFormat,
    /// The prompt font, centered for the switch icon.
    pub(super) icon: IDWriteTextFormat,
    /// `GLYPH_FONT`, centered in a row's icon box.
    pub(super) glyph: IDWriteTextFormat,
    pub(super) image: Option<ID2D1BitmapBrush>,
    pub(super) layout: Layout,
    pub(super) config: Config,
    pub(super) em: f32,
    pub(super) scale: f32,
    // Fields drop in order: `target` draws into `dc`, and `dib` cannot be deleted while `dc` holds it.
    pub(super) dc: MemoryDc,
    #[expect(dead_code, reason = "held only so drop deletes it")]
    pub(super) dib: Owned<HBITMAP>,
}

/// A `CreateCompatibleDC` device context, deleted on drop. Drop cannot report a failed `DeleteDC`.
pub(super) struct MemoryDc(pub(super) HDC);

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `CreateCompatibleDC`, this is its only owner, and drop runs once.
        _ = unsafe { DeleteDC(self.0) };
    }
}

impl Canvas {
    pub(super) fn px(&self, length: Length) -> f32 {
        length.px(self.em, self.scale)
    }

    pub(super) fn icon_size(&self) -> u32 {
        // Tens of px, never negative.
        self.px(self.config.element.icon).round() as u32
    }

    pub(super) fn fill(&self, rect: Rect, radius: f32, color: Color) {
        // SAFETY: the color is a stack value read during the call, and `brush` lives as long as `self`.
        unsafe { self.brush.SetColor(&d2d_color(color)) };
        // SAFETY: called between `BeginDraw` and `EndDraw` on the picker thread, with a brush of the same target.
        unsafe {
            self.target
                .FillRoundedRectangle(&rounded(rect, radius), &self.brush);
        }
    }

    pub(super) fn text(&self, text: &str, format: &IDWriteTextFormat, rect: Rect, color: Color) {
        let units: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: the color is a stack value read during the call, and `brush` lives as long as `self`.
        unsafe { self.brush.SetColor(&d2d_color(color)) };
        // SAFETY: called while drawing, `units` outlives the call and `brush` belongs to the same target.
        unsafe {
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

    /// Draws a match's label in `element.color`, with the chars at `matched` in `element.highlight`.
    pub(super) fn label(&self, text: &str, matched: &[usize], rect: Rect) -> Result<(), Error> {
        let units: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: `units` and the text format outlive the call, which copies the text into the layout.
        let layout = unsafe {
            self.dwrite.CreateTextLayout(
                &units,
                &self.text,
                rect.right - rect.left,
                rect.bottom - rect.top,
            )
        }
        .map_err(win32("CreateTextLayout"))?;
        let mut start: u32 = 0;
        for (at, letter) in text.chars().enumerate() {
            let length = letter.len_utf16() as u32; // 1 or 2.
            if matched.binary_search(&at).is_ok() {
                let range = DWRITE_TEXT_RANGE {
                    startPosition: start,
                    length,
                };
                // SAFETY: the range lies within `text`, and the layout keeps its own reference to the brush.
                unsafe { layout.SetDrawingEffect(&self.highlight, range) }
                    .map_err(win32("IDWriteTextLayout::SetDrawingEffect"))?;
            }
            start += length;
        }
        // SAFETY: the color is a stack value read during the call, and `brush` lives as long as `self`.
        unsafe { self.brush.SetColor(&d2d_color(self.config.element.color)) };
        // SAFETY: called while drawing, with a layout and brush that outlive the call.
        unsafe {
            self.target.DrawTextLayout(
                windows_numerics::Vector2 {
                    X: rect.left,
                    Y: rect.top,
                },
                &layout,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }
        Ok(())
    }

    /// Draws the typed text, scrolled left far enough to keep the caret in view, and returns the caret's offset from
    /// the entry's left edge.
    pub(super) fn entry(&self, typed: &str, caret: usize) -> Result<f32, Error> {
        let entry = self.layout.entry;
        let text = text_layout(&self.dwrite, &self.text, typed, entry.bottom - entry.top)?;
        let (mut x, mut y, mut hit) = (0.0, 0.0, DWRITE_HIT_TEST_METRICS::default());
        // The caret indexes a typed query, far below u32::MAX.
        // SAFETY: the out parameters are live locals, and a caret past the text is clamped by DirectWrite.
        unsafe {
            text.HitTestTextPosition(caret as u32, false, &raw mut x, &raw mut y, &raw mut hit)
        }
        .map_err(win32("IDWriteTextLayout::HitTestTextPosition"))?;
        let scroll = (x - (entry.right - entry.left)).max(0.0);
        // SAFETY: the color is a stack value read during the call, and `brush` lives as long as `self`.
        unsafe { self.brush.SetColor(&d2d_color(self.config.input.color)) };
        // SAFETY: called while drawing, and popped below before the frame ends.
        unsafe {
            self.target
                .PushAxisAlignedClip(&d2d_rect(entry), D2D1_ANTIALIAS_MODE_ALIASED);
        }
        // SAFETY: called while drawing, with a layout and brush that outlive the call.
        unsafe {
            self.target.DrawTextLayout(
                windows_numerics::Vector2 {
                    X: entry.left - scroll,
                    Y: entry.top,
                },
                &text,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }
        // SAFETY: pops the clip pushed above, in the same frame.
        unsafe { self.target.PopAxisAlignedClip() };
        Ok(x - scroll)
    }

    /// `pixels` as a bitmap for this canvas, a pixel per DIP like the render target.
    pub(super) fn bitmap(&self, pixels: &Pixels) -> Result<ID2D1Bitmap, Error> {
        let size = D2D_SIZE_U {
            width: pixels.side,
            height: pixels.side,
        };
        let properties = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let data = pixels.bgra.as_ptr().cast();
        // SAFETY: `bgra` holds `side` rows of `side * 4` bytes, which the call copies before returning.
        unsafe {
            self.target
                .CreateBitmap(size, Some(data), pixels.side * 4, &raw const properties)
        }
        .map_err(win32("ID2D1RenderTarget::CreateBitmap"))
    }

    /// Puts the drawn frame on screen.
    pub(super) fn show(&self) -> Result<(), Error> {
        // Whole pixels from `ceil`, a screen's size at most.
        let size = SIZE {
            cx: self.layout.width as i32,
            cy: self.layout.height as i32,
        };
        // windows-rs declares both AC_ constants as u32, and both are 0 or 1.
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let corner = POINT::default();
        // SAFETY: `window` is the picker's live window, `dc` holds the drawn DIB, and every pointer is a live local.
        unsafe {
            UpdateLayeredWindow(
                self.window,
                None,
                Some(&raw const self.origin),
                Some(&raw const size),
                Some(self.dc.0),
                Some(&raw const corner),
                COLORREF::default(),
                Some(&raw const blend),
                ULW_ALPHA,
            )
        }
        .map_err(win32("UpdateLayeredWindow"))
    }
}

/// A single-line text format of `family` at `size` px that ends overlong text with an ellipsis.
pub(super) fn format(
    dwrite: &IDWriteFactory,
    family: &str,
    size: f32,
) -> Result<IDWriteTextFormat, Error> {
    let mut fonts: Option<IDWriteFontCollection> = None;
    // SAFETY: `fonts` is a live local the call fills.
    unsafe { dwrite.GetSystemFontCollection(&raw mut fonts, false) }
        .map_err(win32("GetSystemFontCollection"))?;
    let fonts = fonts.ok_or(Error::NoFonts)?;
    let (mut index, mut exists) = (0, false.into());
    let name = HSTRING::from(family);
    // SAFETY: `name` is NUL-terminated and the out parameters are live locals.
    unsafe { fonts.FindFamilyName(&name, &raw mut index, &raw mut exists) }
        .map_err(win32("IDWriteFontCollection::FindFamilyName"))?;
    if !exists.as_bool() {
        return Err(Error::Font(family.to_owned()));
    }
    // SAFETY: `name` and the locale are NUL-terminated strings that outlive the call.
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
    // SAFETY: `format` is a live text format the sign is built for.
    let ellipsis = unsafe { dwrite.CreateEllipsisTrimmingSign(&format) }
        .map_err(win32("CreateEllipsisTrimmingSign"))?;
    let trimming = DWRITE_TRIMMING {
        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
        delimiter: 0,
        delimiterCount: 0,
    };
    // SAFETY: sets a valid enum value on a live format.
    unsafe { format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP) }
        .map_err(win32("SetWordWrapping"))?;
    // SAFETY: sets a valid enum value on a live format.
    unsafe { format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) }
        .map_err(win32("SetParagraphAlignment"))?;
    // SAFETY: `trimming` is read during the call, and the format keeps its own reference to `ellipsis`.
    unsafe { format.SetTrimming(&raw const trimming, &ellipsis) }.map_err(win32("SetTrimming"))?;
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
    // SAFETY: `units` and `format` outlive the call, which copies the text into the layout.
    unsafe { dwrite.CreateTextLayout(&units, format, f32::MAX, height) }
        .map_err(win32("CreateTextLayout"))
}

pub(super) fn measure(
    dwrite: &IDWriteFactory,
    format: &IDWriteTextFormat,
    text: &str,
) -> Result<DWRITE_TEXT_METRICS, Error> {
    let layout = text_layout(dwrite, format, text, 0.0)?;
    let mut metrics = DWRITE_TEXT_METRICS::default();
    // SAFETY: `metrics` is a live local the call fills.
    unsafe { layout.GetMetrics(&raw mut metrics) }
        .map_err(win32("IDWriteTextLayout::GetMetrics"))?;
    Ok(metrics)
}

/// The picture at `path` as a brush that covers a `width` by `height` window, cropping what overflows.
pub(super) fn image(
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
    // SAFETY: the path is NUL-terminated and outlives the call.
    let decoder = unsafe {
        wic.CreateDecoderFromFilename(
            &HSTRING::from(path),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
    }
    .map_err(failed)?;
    // SAFETY: every image has a frame 0, and a missing one comes back as an error.
    let frame = unsafe { decoder.GetFrame(0) }.map_err(failed)?;
    let (mut source_width, mut source_height) = (0, 0);
    // SAFETY: the out parameters are live locals.
    unsafe { frame.GetSize(&raw mut source_width, &raw mut source_height) }.map_err(failed)?;
    // Decoded straight to the size it covers the window at: a photo at full size is tens of MB and slow to decode.
    // Image and window sides stay well inside the range where f32 holds whole numbers exactly.
    let zoom = (width / source_width as f32).max(height / source_height as f32);
    // SAFETY: takes no arguments.
    let scaler = unsafe { wic.CreateBitmapScaler() }.map_err(failed)?;
    // SAFETY: `frame` is a live bitmap source the scaler keeps its own reference to.
    unsafe {
        scaler.Initialize(
            &frame,
            (source_width as f32 * zoom).ceil() as u32,
            (source_height as f32 * zoom).ceil() as u32,
            WICBitmapInterpolationModeFant,
        )
    }
    .map_err(failed)?;
    // SAFETY: takes no arguments.
    let converter = unsafe { wic.CreateFormatConverter() }.map_err(failed)?;
    // SAFETY: `scaler` is a live source the converter keeps its own reference to, and the GUID is a static.
    unsafe {
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeMedianCut,
        )
    }
    .map_err(failed)?;
    // SAFETY: `converter` yields 32bpp PBGRA, a format Direct2D bitmaps accept.
    let bitmap = unsafe { target.CreateBitmapFromWicBitmap(&converter, None) }.map_err(failed)?;
    // SAFETY: `bitmap` belongs to `target`, which makes the brush.
    let brush = unsafe { target.CreateBitmapBrush(&bitmap, None, None) }.map_err(failed)?;
    // SAFETY: reads the size of a live bitmap.
    let size = unsafe { bitmap.GetSize() };
    // Centered on whole pixels, so the bitmap is copied rather than resampled.
    let centered = Matrix3x2::translation(
        ((width - size.width) / 2.0).round(),
        ((height - size.height) / 2.0).round(),
    );
    // SAFETY: `centered` is a stack value read during the call.
    unsafe { brush.SetTransform(&raw const centered) };
    Ok(brush)
}

/// `rect` with corners of `radius`, kept small enough to fit.
pub(super) fn rounded(rect: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
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

pub(super) fn d2d_rect(rect: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

pub(super) fn d2d_color(color: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: color.red,
        g: color.green,
        b: color.blue,
        a: color.alpha,
    }
}
