//! The clipboard's text, which Ctrl+V types into the query.

use windows::Win32::Foundation::{HGLOBAL, SetLastError, WIN32_ERROR};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::error::{Error, last, win32};

/// The clipboard's text, or nothing when it holds none.
pub(super) fn clipboard() -> Result<String, Error> {
    let format = u32::from(CF_UNICODETEXT.0);
    // It returns false both for no text and for a failure, and sets the last error only for a failure.
    // SAFETY: sets this thread's last error, which nothing reads until the check below.
    unsafe { SetLastError(WIN32_ERROR(0)) };
    // SAFETY: takes a format id only.
    if let Err(error) = unsafe { IsClipboardFormatAvailable(format) } {
        if error.code().is_ok() {
            return Ok(String::new());
        }
        return Err(win32("IsClipboardFormatAvailable")(error));
    }
    // SAFETY: no owner window, and the clipboard is closed below on every path.
    unsafe { OpenClipboard(None) }.map_err(win32("OpenClipboard"))?;
    let text = clipboard_text(format);
    // SAFETY: closes the clipboard this thread opened above.
    let closed = unsafe { CloseClipboard() }.map_err(win32("CloseClipboard"));
    // A failed read is the cause, so it wins over a failed close.
    let text = text?;
    closed?;
    Ok(text)
}

fn clipboard_text(format: u32) -> Result<String, Error> {
    // SAFETY: the caller holds the clipboard open, so the handle stays valid until it closes.
    let handle = unsafe { GetClipboardData(format) }.map_err(win32("GetClipboardData"))?;
    let global = HGLOBAL(handle.0);
    // SAFETY: CF_UNICODETEXT data is a global memory block, unlocked below.
    let data = unsafe { GlobalLock(global) }.cast::<u16>();
    if data.is_null() {
        return Err(last("GlobalLock"));
    }
    // SAFETY: `data` came from GlobalLock on `global`, which stays locked until GlobalUnlock below.
    let text = unsafe { locked_text(global, data) };
    // GlobalUnlock reports releasing the last lock as a failure, so its result says nothing.
    // SAFETY: releases the lock taken above, after the last read of `data`.
    _ = unsafe { GlobalUnlock(global) };
    text
}

/// The text up to the first NUL in the locked `data`, within its block: another program can set one without a NUL.
///
/// # Safety
///
/// `data` must be the pointer `GlobalLock` returned for `global`, and the block must stay locked for the call.
unsafe fn locked_text(global: HGLOBAL, data: *const u16) -> Result<String, Error> {
    // SAFETY: the caller passes a live, locked global block.
    let units = unsafe { GlobalSize(global) } / size_of::<u16>();
    if units == 0 {
        return Err(last("GlobalSize"));
    }
    // SAFETY: `data` points at the locked block, which holds at least `units` u16s and stays locked for the call.
    let block = unsafe { std::slice::from_raw_parts(data, units) };
    let text = match block.iter().position(|&unit| unit == 0) {
        Some(len) => block.split_at(len).0,
        None => block,
    };
    Ok(String::from_utf16(text)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::GlobalFree;
    use windows::Win32::System::Memory::{GHND, GlobalAlloc};

    /// `locked_text` of a block holding `units`, zero-filled past them as `GlobalAlloc` leaves it.
    fn text_of(units: &[u16]) -> Result<String, Box<dyn std::error::Error>> {
        // SAFETY: allocates a fresh block, freed below.
        let global = unsafe { GlobalAlloc(GHND, size_of_val(units)) }?;
        // SAFETY: `global` is the movable block just allocated, unlocked below.
        let data = unsafe { GlobalLock(global) }.cast::<u16>();
        if data.is_null() {
            return Err("GlobalLock failed".into());
        }
        // SAFETY: the block was sized for `units`, and a fresh allocation cannot overlap the slice.
        unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), data, units.len()) };
        // SAFETY: `data` came from GlobalLock on `global`, which stays locked until GlobalUnlock below.
        let text = unsafe { locked_text(global, data) };
        // SAFETY: releases the lock taken above, after the last read of `data`.
        _ = unsafe { GlobalUnlock(global) };
        // GlobalFree returns NULL on success, which windows-rs reports as an error, so its result says nothing.
        // SAFETY: frees the unlocked block this function allocated, used no further.
        _ = unsafe { GlobalFree(Some(global)) };
        Ok(text?)
    }

    #[test]
    fn pasted_text_ends_at_its_nul_or_its_block() -> Result<(), Box<dyn std::error::Error>> {
        let units = |text: &str| -> Vec<u16> { text.encode_utf16().collect() };
        assert_eq!(text_of(&units("ab\0cd"))?, "ab");
        assert_eq!(text_of(&units("abc"))?, "abc");
        Ok(())
    }
}
