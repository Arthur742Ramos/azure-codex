use std::io::IsTerminal;
use std::io::Result;
use std::io::Stdout;
use std::io::stdin;
use std::io::stdout;
use std::panic;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use crossterm::SynchronizedUpdate;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::DisableFocusChange;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableBracketedPaste;
use crossterm::event::EnableFocusChange;
use crossterm::event::EnableMouseCapture;
use crossterm::event::Event;
use crossterm::event::KeyEvent;
use crossterm::event::KeyboardEnhancementFlags;
use crossterm::event::PopKeyboardEnhancementFlags;
use crossterm::event::PushKeyboardEnhancementFlags;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::supports_keyboard_enhancement;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::disable_raw_mode;
use ratatui::crossterm::terminal::enable_raw_mode;
use ratatui::layout::Offset;
use ratatui::layout::Rect;
use ratatui::text::Line;
use tokio::select;
use tokio::sync::broadcast;
use tokio_stream::Stream;

pub use self::frame_requester::FrameRequester;
use crate::custom_terminal;
use crate::custom_terminal::Terminal as CustomTerminal;
use crate::notifications::DesktopNotificationBackend;
use crate::notifications::NotificationBackendKind;
use crate::notifications::detect_backend;
#[cfg(unix)]
use crate::tui::job_control::SUSPEND_KEY;
#[cfg(unix)]
use crate::tui::job_control::SuspendContext;

mod frame_requester;
#[cfg(unix)]
mod job_control;
pub(crate) mod scrolling;

/// A type alias for the terminal type used in this application
pub type Terminal = CustomTerminal<CrosstermBackend<Stdout>>;

/// Set up terminal modes for the TUI.
///
/// # Arguments
/// * `disable_mouse_capture` - When `true`, mouse capture is disabled, allowing native
///   terminal text selection/copy/paste. When `false` (default), mouse events are captured
///   by the application for scrolling and selection.
pub fn set_modes(disable_mouse_capture: bool) -> Result<()> {
    // On Windows, save the original console mode before any ANSI escapes are sent.
    // This ensures we can properly restore the console state when toggling mouse capture.
    #[cfg(windows)]
    {
        if let Err(e) = crate::windows_mouse::save_original_mode() {
            tracing::warn!("Failed to save original Windows console mode: {e}");
        }
    }

    execute!(stdout(), EnableBracketedPaste)?;

    enable_raw_mode()?;
    // Enable keyboard enhancement flags so modifiers for keys like Enter are disambiguated.
    // chat_composer.rs is using a keyboard event listener to enter for any modified keys
    // to create a new line that require this.
    // Some terminals (notably legacy Windows consoles) do not support
    // keyboard enhancement flags. Attempt to enable them, but continue
    // gracefully if unsupported.
    let _ = execute!(
        stdout(),
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        )
    );

    let _ = execute!(stdout(), EnableFocusChange);

    // Enable application mouse mode so scroll events are delivered as
    // Mouse events instead of arrow keys - unless disabled by config.
    // Always do a disable->enable cycle to normalize console state; without this,
    // mouse capture can be stuck until the user toggles it manually.
    disable_mouse_capture_internal();
    enable_mouse_capture_internal();

    if disable_mouse_capture {
        disable_mouse_capture_internal();
    }

    Ok(())
}

/// Enable mouse capture (internal helper).
fn enable_mouse_capture_internal() {
    let _ = execute!(stdout(), EnableMouseCapture);

    // On Windows, also enable mouse capture via the Win32 Console API.
    // Windows Terminal does not properly handle ANSI escape codes for mouse capture
    // (see https://github.com/crossterm-rs/crossterm/issues/446), so we use
    // SetConsoleMode with ENABLE_MOUSE_INPUT directly as a workaround.
    #[cfg(windows)]
    {
        if let Err(e) = crate::windows_mouse::enable_mouse_capture() {
            tracing::warn!("Failed to enable Windows mouse capture via Win32 API: {e}");
        }
    }
}

/// Disable mouse capture (internal helper).
fn disable_mouse_capture_internal() {
    let _ = execute!(stdout(), DisableMouseCapture);

    #[cfg(windows)]
    {
        if let Err(e) = crate::windows_mouse::disable_mouse_capture() {
            tracing::warn!("Failed to disable Windows mouse capture via Win32 API: {e}");
        }
    }
}

/// Restore the terminal to its original state.
/// Inverse of `set_modes`.
pub fn restore() -> Result<()> {
    // Pop may fail on platforms that didn't support the push; ignore errors.
    let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    let _ = execute!(stdout(), DisableMouseCapture);

    // On Windows, restore the original console mode (disables Win32 mouse capture).
    #[cfg(windows)]
    {
        if let Err(e) = crate::windows_mouse::disable_mouse_capture() {
            tracing::warn!("Failed to disable Windows mouse capture via Win32 API: {e}");
        }
    }

    execute!(stdout(), DisableBracketedPaste)?;
    let _ = execute!(stdout(), DisableFocusChange);
    disable_raw_mode()?;
    let _ = execute!(stdout(), crossterm::cursor::Show);
    Ok(())
}

/// Initialize the terminal (inline viewport; history stays in normal scrollback)
///
/// # Arguments
/// * `disable_mouse_capture` - When `true`, mouse capture is disabled to allow native
///   terminal text selection. Pass `false` for the default behavior.
pub fn init(disable_mouse_capture: bool) -> Result<Terminal> {
    use crossterm::terminal::size;
    use ratatui::layout::Position;
    use std::io::Write;

    if !stdin().is_terminal() {
        return Err(std::io::Error::other("stdin is not a terminal"));
    }
    if !stdout().is_terminal() {
        return Err(std::io::Error::other("stdout is not a terminal"));
    }

    // Get terminal size BEFORE any mode changes
    let (_, term_height) = size()?;

    // Query cursor position BEFORE setting terminal modes.
    let initial_cursor = crossterm::cursor::position().unwrap_or((0, 0));
    tracing::debug!(
        "Initial cursor position: ({}, {}), terminal height: {}",
        initial_cursor.0,
        initial_cursor.1,
        term_height
    );

    // Print newlines to scroll the terminal down and make room for the TUI.
    // This pushes any existing content into scrollback before we start drawing.
    // We want the TUI to start at the bottom of the visible screen.
    let lines_to_scroll = term_height
        .saturating_sub(initial_cursor.1)
        .saturating_sub(1);
    if lines_to_scroll > 0 {
        let mut stdout_handle = stdout();
        for _ in 0..lines_to_scroll {
            let _ = writeln!(stdout_handle);
        }
        let _ = stdout_handle.flush();
    }

    // Now query cursor position again - it should be near the bottom
    let cursor_pos = crossterm::cursor::position()
        .map(|(x, y)| Position { x, y })
        .unwrap_or(Position {
            x: 0,
            y: term_height.saturating_sub(1),
        });
    tracing::debug!(
        "Cursor position after scrolling: ({}, {})",
        cursor_pos.x,
        cursor_pos.y
    );

    set_modes(disable_mouse_capture)?;

    set_panic_hook();

    let backend = CrosstermBackend::new(stdout());

    // Start viewport at current cursor position (should be near bottom after scrolling)
    let tui = CustomTerminal::with_cursor_position(backend, cursor_pos)?;

    tracing::debug!(
        "Terminal initialized with viewport starting at y={}",
        cursor_pos.y
    );
    Ok(tui)
}

fn set_panic_hook() {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = restore(); // ignore any errors as we are already failing
        hook(panic_info);
    }));
}

#[derive(Debug)]
pub enum TuiEvent {
    Key(KeyEvent),
    Paste(String),
    Draw,
    Mouse(crossterm::event::MouseEvent),
}

pub struct Tui {
    frame_requester: FrameRequester,
    draw_tx: broadcast::Sender<()>,
    pub(crate) terminal: Terminal,
    pending_history_lines: Vec<Line<'static>>,
    alt_saved_viewport: Option<ratatui::layout::Rect>,
    #[cfg(unix)]
    suspend_context: SuspendContext,
    // True when overlay alt-screen UI is active
    alt_screen_active: Arc<AtomicBool>,
    // True when terminal/tab is focused; updated internally from crossterm events
    terminal_focused: Arc<AtomicBool>,
    enhanced_keys_supported: bool,
    notification_backend: Option<DesktopNotificationBackend>,
    // True when mouse capture is active (app handles mouse events for scrolling/selection)
    mouse_capture_enabled: bool,
}

impl Tui {
    /// Create a new Tui instance.
    ///
    /// # Arguments
    /// * `terminal` - The terminal backend
    /// * `mouse_capture_enabled` - Whether mouse capture is initially enabled
    pub fn new(terminal: Terminal, mouse_capture_enabled: bool) -> Self {
        let (draw_tx, _) = broadcast::channel(1);
        let frame_requester = FrameRequester::new(draw_tx.clone());

        // Detect keyboard enhancement support before any EventStream is created so the
        // crossterm poller can acquire its lock without contention.
        let enhanced_keys_supported = supports_keyboard_enhancement().unwrap_or(false);
        // Cache this to avoid contention with the event reader.
        supports_color::on_cached(supports_color::Stream::Stdout);
        let _ = crate::terminal_palette::default_colors();

        Self {
            frame_requester,
            draw_tx,
            terminal,
            pending_history_lines: vec![],
            alt_saved_viewport: None,
            #[cfg(unix)]
            suspend_context: SuspendContext::new(),
            alt_screen_active: Arc::new(AtomicBool::new(false)),
            terminal_focused: Arc::new(AtomicBool::new(true)),
            enhanced_keys_supported,
            notification_backend: Some(detect_backend()),
            mouse_capture_enabled,
        }
    }

    /// Returns whether mouse capture is currently enabled.
    #[allow(dead_code)]
    pub fn is_mouse_capture_enabled(&self) -> bool {
        self.mouse_capture_enabled
    }

    /// Toggle mouse capture mode.
    ///
    /// When mouse capture is enabled, the application handles mouse events for scrolling
    /// and text selection. When disabled, the terminal's native mouse handling is used,
    /// allowing native text selection/copy/paste.
    ///
    /// Returns the new state (true = enabled, false = disabled).
    pub fn toggle_mouse_capture(&mut self) -> bool {
        if self.mouse_capture_enabled {
            disable_mouse_capture_internal();
            self.mouse_capture_enabled = false;
        } else {
            enable_mouse_capture_internal();
            self.mouse_capture_enabled = true;
        }
        self.mouse_capture_enabled
    }

    /// Set mouse capture mode explicitly.
    #[allow(dead_code)]
    pub fn set_mouse_capture(&mut self, enabled: bool) {
        if enabled != self.mouse_capture_enabled {
            if enabled {
                enable_mouse_capture_internal();
            } else {
                disable_mouse_capture_internal();
            }
            self.mouse_capture_enabled = enabled;
        }
    }

    pub fn frame_requester(&self) -> FrameRequester {
        self.frame_requester.clone()
    }

    pub fn enhanced_keys_supported(&self) -> bool {
        self.enhanced_keys_supported
    }

    /// Emit a desktop notification now if the terminal is unfocused.
    /// Returns true if a notification was posted.
    pub fn notify(&mut self, message: impl AsRef<str>) -> bool {
        if self.terminal_focused.load(Ordering::Relaxed) {
            return false;
        }

        let Some(backend) = self.notification_backend.as_mut() else {
            return false;
        };

        let message = message.as_ref().to_string();
        match backend.notify(&message) {
            Ok(()) => true,
            Err(err) => match backend.kind() {
                NotificationBackendKind::WindowsToast => {
                    tracing::error!(
                        error = %err,
                        "Failed to send Windows toast notification; falling back to OSC 9"
                    );
                    self.notification_backend = Some(DesktopNotificationBackend::osc9());
                    if let Some(backend) = self.notification_backend.as_mut() {
                        if let Err(osc_err) = backend.notify(&message) {
                            tracing::warn!(
                                error = %osc_err,
                                "Failed to emit OSC 9 notification after toast fallback; \
                                 disabling future notifications"
                            );
                            self.notification_backend = None;
                            return false;
                        }
                        return true;
                    }
                    false
                }
                NotificationBackendKind::Osc9 => {
                    tracing::warn!(
                        error = %err,
                        "Failed to emit OSC 9 notification; disabling future notifications"
                    );
                    self.notification_backend = None;
                    false
                }
            },
        }
    }

    pub fn event_stream(&self) -> Pin<Box<dyn Stream<Item = TuiEvent> + Send + 'static>> {
        use tokio_stream::StreamExt;

        // Re-enable mouse capture now that the event stream is starting.
        // On Windows, the initial enable during set_modes() happens before the
        // EventStream is created, so mouse events may not be captured. This
        // re-enable ensures the console mode is set correctly after the reader starts.
        if self.mouse_capture_enabled {
            enable_mouse_capture_internal();
        }

        let mut crossterm_events = crossterm::event::EventStream::new();
        let mut draw_rx = self.draw_tx.subscribe();

        // State for tracking how we should resume from ^Z suspend.
        #[cfg(unix)]
        let suspend_context = self.suspend_context.clone();
        #[cfg(unix)]
        let alt_screen_active = self.alt_screen_active.clone();

        let terminal_focused = self.terminal_focused.clone();
        let event_stream = async_stream::stream! {
            loop {
                select! {
                    event_result = crossterm_events.next() => {
                        match event_result {
                            Some(Ok(event)) => {
                                match event {
                                    Event::Key(key_event) => {
                                        #[cfg(unix)]
                                        if SUSPEND_KEY.is_press(key_event) {
                                            let _ = suspend_context.suspend(&alt_screen_active);
                                            // We continue here after resume.
                                            yield TuiEvent::Draw;
                                            continue;
                                        }
                                        yield TuiEvent::Key(key_event);
                                    }
                                    Event::Resize(_, _) => {
                                        yield TuiEvent::Draw;
                                    }
                                    Event::Paste(pasted) => {
                                        yield TuiEvent::Paste(pasted);
                                    }
                                    Event::Mouse(mouse_event) => {
                                        yield TuiEvent::Mouse(mouse_event);
                                    }
                                    Event::FocusGained => {
                                        terminal_focused.store(true, Ordering::Relaxed);
                                        crate::terminal_palette::requery_default_colors();
                                        yield TuiEvent::Draw;
                                    }
                                    Event::FocusLost => {
                                        terminal_focused.store(false, Ordering::Relaxed);
                                    }
                                }
                            }
                            Some(Err(_)) | None => {
                                // Exit the loop in case of broken pipe as we will never
                                // recover from it
                                break;
                            }
                        }
                    }
                    result = draw_rx.recv() => {
                        match result {
                            Ok(_) => {
                                // Re-enable mouse capture on Windows before draw.
                                // ANSI sequences during rendering can reset console mode.
                                #[cfg(windows)]
                                {
                                    let _ = crate::windows_mouse::enable_mouse_capture();
                                }
                                yield TuiEvent::Draw;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                // We dropped one or more draw notifications; coalesce to a single draw.
                                yield TuiEvent::Draw;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                // Sender dropped. This stream likely outlived its owning `Tui`;
                                // exit to avoid spinning on a permanently-closed receiver.
                                break;
                            }
                        }
                    }
                }
            }
        };
        Box::pin(event_stream)
    }

    /// Enter alternate screen and expand the viewport to full terminal size, saving the current
    /// inline viewport for restoration when leaving.
    pub fn enter_alt_screen(&mut self) -> Result<()> {
        let _ = execute!(self.terminal.backend_mut(), EnterAlternateScreen);
        if let Ok(size) = self.terminal.size() {
            self.alt_saved_viewport = Some(self.terminal.viewport_area);
            self.terminal.set_viewport_area(ratatui::layout::Rect::new(
                0,
                0,
                size.width,
                size.height,
            ));
            let _ = self.terminal.clear();
        }
        self.alt_screen_active.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Leave alternate screen and restore the previously saved inline viewport, if any.
    pub fn leave_alt_screen(&mut self) -> Result<()> {
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        if let Some(saved) = self.alt_saved_viewport.take() {
            self.terminal.set_viewport_area(saved);
        }
        self.alt_screen_active.store(false, Ordering::Relaxed);
        Ok(())
    }

    pub fn insert_history_lines(&mut self, lines: Vec<Line<'static>>) {
        self.pending_history_lines.extend(lines);
        self.frame_requester().schedule_frame();
    }

    pub fn draw(
        &mut self,
        height: u16,
        draw_fn: impl FnOnce(&mut custom_terminal::Frame),
    ) -> Result<()> {
        // If we are resuming from ^Z, we need to prepare the resume action now so we can apply it
        // in the synchronized update.
        #[cfg(unix)]
        let mut prepared_resume = self
            .suspend_context
            .prepare_resume_action(&mut self.terminal, &mut self.alt_saved_viewport);

        // Precompute any viewport updates that need a cursor-position query before entering
        // the synchronized update, to avoid racing with the event reader.
        let mut pending_viewport_area = self.pending_viewport_area()?;

        stdout().sync_update(|_| {
            #[cfg(unix)]
            if let Some(prepared) = prepared_resume.take() {
                prepared.apply(&mut self.terminal)?;
            }

            let terminal = &mut self.terminal;
            if let Some(new_area) = pending_viewport_area.take() {
                terminal.set_viewport_area(new_area);
                // When the viewport origin changes (e.g. resize + cursor moved),
                // the actual terminal contents no longer match our back buffer.
                // Clear the new viewport so we don't leave "ghost" text behind.
                terminal.clear()?;
            }

            let size = terminal.size()?;

            // Match original tui behavior: modify existing viewport_area in place
            // to preserve the y position (inline mode keeps scrollback).
            let mut area = terminal.viewport_area;
            area.height = height.min(size.height);
            area.width = size.width;
            // If the viewport has expanded past the screen bottom, adjust y position.
            if area.bottom() > size.height {
                area.y = size.height.saturating_sub(area.height);
            }
            if area != terminal.viewport_area {
                terminal.set_viewport_area(area);
                // When the viewport size changes, the new region may contain preexisting terminal
                // output. Clear it so rendering doesn't "blend" with whatever was there before.
                terminal.clear()?;
            }

            // Update the y position for suspending so Ctrl-Z can place the cursor correctly.
            #[cfg(unix)]
            {
                let inline_area_bottom = if self.alt_screen_active.load(Ordering::Relaxed) {
                    self.alt_saved_viewport
                        .map(|r| r.bottom().saturating_sub(1))
                        .unwrap_or_else(|| area.bottom().saturating_sub(1))
                } else {
                    area.bottom().saturating_sub(1)
                };
                self.suspend_context.set_cursor_y(inline_area_bottom);
            }

            terminal.draw(|frame| {
                draw_fn(frame);
            })
        })?
    }

    fn pending_viewport_area(&mut self) -> Result<Option<Rect>> {
        let terminal = &mut self.terminal;
        let screen_size = terminal.size()?;
        let last_known_screen_size = terminal.last_known_screen_size;
        if screen_size != last_known_screen_size
            && let Ok(cursor_pos) = terminal.get_cursor_position()
        {
            let last_known_cursor_pos = terminal.last_known_cursor_pos;
            // If we resized AND the cursor moved, we adjust the viewport area to keep the
            // cursor in the same position. This is a heuristic that seems to work well
            // at least in iTerm2.
            if cursor_pos.y != last_known_cursor_pos.y {
                let offset = Offset {
                    x: 0,
                    y: cursor_pos.y as i32 - last_known_cursor_pos.y as i32,
                };
                return Ok(Some(terminal.viewport_area.offset(offset)));
            }
        }
        Ok(None)
    }
}
