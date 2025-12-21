//! Windows-specific mouse capture using Win32 Console API.
//!
//! Windows Terminal does not properly handle ANSI escape codes for mouse capture
//! (see <https://github.com/crossterm-rs/crossterm/issues/446>). This module
//! provides a workaround by using the Win32 Console API directly via
//! `SetConsoleMode` with `ENABLE_MOUSE_INPUT`.
//!
//! On classic cmd.exe and PowerShell, crossterm's ANSI-based mouse capture works,
//! but Windows Terminal requires the WinAPI approach.

use std::io;
use std::sync::Mutex;
use std::sync::Once;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Console::CONSOLE_MODE;
use windows_sys::Win32::System::Console::ENABLE_EXTENDED_FLAGS;
use windows_sys::Win32::System::Console::ENABLE_MOUSE_INPUT;
use windows_sys::Win32::System::Console::ENABLE_QUICK_EDIT_MODE;
use windows_sys::Win32::System::Console::ENABLE_VIRTUAL_TERMINAL_INPUT;
use windows_sys::Win32::System::Console::ENABLE_WINDOW_INPUT;
use windows_sys::Win32::System::Console::FlushConsoleInputBuffer;
use windows_sys::Win32::System::Console::GetConsoleMode;
use windows_sys::Win32::System::Console::GetStdHandle;
use windows_sys::Win32::System::Console::STD_INPUT_HANDLE;
use windows_sys::Win32::System::Console::SetConsoleMode;

/// Stores the original console mode captured at startup, before any mouse commands.
/// This is set once via `save_original_mode()` and never modified afterward.
static ORIGINAL_MODE: Mutex<Option<CONSOLE_MODE>> = Mutex::new(None);
static ORIGINAL_MODE_INIT: Once = Once::new();

/// Save the original console mode at startup, before any ANSI escape sequences.
///
/// This must be called once during TUI initialization, before any mouse capture
/// commands are sent. The saved mode will be used to properly restore the console
/// when mouse capture is disabled.
pub fn save_original_mode() -> io::Result<()> {
    let mut result = Ok(());
    ORIGINAL_MODE_INIT.call_once(|| {
        // SAFETY: Win32 Console API calls are safe when used with valid handles.
        unsafe {
            let handle: HANDLE = GetStdHandle(STD_INPUT_HANDLE);
            if handle.is_null() || handle == -1_isize as HANDLE {
                result = Err(io::Error::last_os_error());
                return;
            }

            let mut mode: CONSOLE_MODE = 0;
            if GetConsoleMode(handle, &mut mode) == 0 {
                result = Err(io::Error::last_os_error());
                return;
            }

            if let Ok(mut guard) = ORIGINAL_MODE.lock() {
                *guard = Some(mode);
            }

            tracing::debug!("Windows console original mode saved: {mode:#x}");
        }
    });
    result
}

/// Enable mouse input capture using the Win32 Console API.
///
/// This function:
/// 1. Gets the console input handle
/// 2. Enables `ENABLE_MOUSE_INPUT` and `ENABLE_EXTENDED_FLAGS`
/// 3. Disables `ENABLE_QUICK_EDIT_MODE` (required for mouse events to work)
/// 4. Preserves `ENABLE_VIRTUAL_TERMINAL_INPUT` (required for ANSI escape processing)
///
/// This is optimized to be a no-op if the mode is already correct, to avoid
/// overhead when called frequently (e.g., on every draw frame).
pub fn enable_mouse_capture() -> io::Result<()> {
    // SAFETY: Win32 Console API calls are safe when used with valid handles.
    unsafe {
        let handle: HANDLE = GetStdHandle(STD_INPUT_HANDLE);
        if handle.is_null() || handle == -1_isize as HANDLE {
            return Err(io::Error::last_os_error());
        }

        let mut mode: CONSOLE_MODE = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return Err(io::Error::last_os_error());
        }

        // Quick check: if mouse input is already enabled and quick edit is disabled,
        // skip the expensive SetConsoleMode call.
        let has_mouse = (mode & ENABLE_MOUSE_INPUT) != 0;
        let has_quick_edit = (mode & ENABLE_QUICK_EDIT_MODE) != 0;
        if has_mouse && !has_quick_edit {
            return Ok(());
        }

        // Get the original mode to preserve ENABLE_VIRTUAL_TERMINAL_INPUT.
        // Crossterm's ANSI escape sequences can strip this flag on Windows Terminal,
        // which breaks subsequent ANSI processing.
        let original_vti = ORIGINAL_MODE
            .lock()
            .ok()
            .and_then(|g| *g)
            .map(|m| m & ENABLE_VIRTUAL_TERMINAL_INPUT)
            .unwrap_or(ENABLE_VIRTUAL_TERMINAL_INPUT);

        // Enable mouse input, window input (for scroll events), and extended flags
        // Disable quick edit mode (required for mouse capture to work)
        // Preserve virtual terminal input (required for ANSI escape processing)
        let new_mode = (mode
            | ENABLE_MOUSE_INPUT
            | ENABLE_WINDOW_INPUT
            | ENABLE_EXTENDED_FLAGS
            | original_vti)
            & !ENABLE_QUICK_EDIT_MODE;

        if SetConsoleMode(handle, new_mode) == 0 {
            return Err(io::Error::last_os_error());
        }

        // Flush the input buffer to ensure the new mode takes effect immediately
        let _ = FlushConsoleInputBuffer(handle);

        tracing::debug!(
            "Windows mouse capture enabled via Win32 API (current mode: {mode:#x}, new mode: {new_mode:#x})"
        );
    }
    Ok(())
}

/// Disable mouse input capture.
///
/// This explicitly clears the `ENABLE_MOUSE_INPUT` flag and restores `ENABLE_QUICK_EDIT_MODE`
/// to allow native terminal text selection. We don't simply restore the original mode because
/// Windows Terminal may have `ENABLE_MOUSE_INPUT` enabled by default.
/// We also preserve `ENABLE_VIRTUAL_TERMINAL_INPUT` for ANSI escape processing.
pub fn disable_mouse_capture() -> io::Result<()> {
    // SAFETY: Win32 Console API calls are safe when used with valid handles.
    unsafe {
        let handle: HANDLE = GetStdHandle(STD_INPUT_HANDLE);
        if handle.is_null() || handle == -1_isize as HANDLE {
            return Err(io::Error::last_os_error());
        }

        let mut mode: CONSOLE_MODE = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return Err(io::Error::last_os_error());
        }

        let original_mode = ORIGINAL_MODE.lock().ok().and_then(|g| *g);
        let original_vti = original_mode
            .map(|m| m & ENABLE_VIRTUAL_TERMINAL_INPUT)
            .unwrap_or(ENABLE_VIRTUAL_TERMINAL_INPUT);
        let original_quick_edit = original_mode
            .map(|m| (m & ENABLE_QUICK_EDIT_MODE) != 0)
            .unwrap_or(true);

        // Explicitly disable mouse input and restore quick edit mode to its original state.
        // We keep ENABLE_EXTENDED_FLAGS set as it's needed for the quick edit flag to take effect.
        // Preserve virtual terminal input for ANSI escape processing.
        let mut new_mode = (mode & !ENABLE_MOUSE_INPUT) | ENABLE_EXTENDED_FLAGS | original_vti;
        if original_quick_edit {
            new_mode |= ENABLE_QUICK_EDIT_MODE;
        } else {
            new_mode &= !ENABLE_QUICK_EDIT_MODE;
        }

        if SetConsoleMode(handle, new_mode) == 0 {
            return Err(io::Error::last_os_error());
        }

        // Flush the input buffer to ensure the mode change takes effect immediately
        let _ = FlushConsoleInputBuffer(handle);

        tracing::debug!(
            "Windows mouse capture disabled (previous mode: {mode:#x}, new mode: {new_mode:#x})"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_enable_disable_mouse_capture() {
        // This test verifies the functions don't panic
        // Actual mouse capture behavior requires a real console
        let save_result = save_original_mode();
        // May fail in CI without a real console, that's OK
        if save_result.is_ok() {
            let enable_result = enable_mouse_capture();
            if enable_result.is_ok() {
                let disable_result = disable_mouse_capture();
                assert!(
                    disable_result.is_ok(),
                    "disable should succeed after enable"
                );
                // Test that we can toggle again (mode is preserved)
                let enable_again = enable_mouse_capture();
                assert!(
                    enable_again.is_ok(),
                    "should be able to enable again after disable"
                );
                let disable_again = disable_mouse_capture();
                assert!(disable_again.is_ok(), "should be able to disable again");
            }
        }
    }
}
