//! Terminal setup and restoration.
//!
//! Raw mode and the alternate screen must always be undone, including when the
//! event loop panics. [`TerminalGuard::arm`] installs a panic hook that restores
//! the terminal *before* the previously installed hook prints its message, so a
//! panic never leaves the user with a broken shell.

use std::io::{self, Stdout, Write};

use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub type TuiResult<T> = Result<T, TuiError>;

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("terminal i/o error: {0}")]
    Io(#[from] io::Error),
}

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

/// Restores the terminal when dropped.
///
/// The panic hook installed by [`arm`](Self::arm) is left in place for the rest
/// of the process: it is idempotent, so calling it after a normal exit is a no-op,
/// and it is what makes a panic inside the event loop recoverable.
#[derive(Debug)]
pub struct TerminalGuard {
    armed: bool,
}

impl TerminalGuard {
    /// Installs the panic hook. Call this *before* [`init`], so that a failure
    /// while entering the alternate screen is still reported on a sane terminal.
    pub fn arm() -> Self {
        let previous = std::panic::take_hook();
        let hook: PanicHook = Box::new(move |info| {
            let _ = restore_terminal();
            previous(info);
        });
        std::panic::set_hook(hook);
        Self { armed: true }
    }

    /// Leaves the alternate screen and disables raw mode. Safe to call repeatedly.
    pub fn restore(&mut self) -> io::Result<()> {
        if self.disarm() {
            restore_terminal()
        } else {
            Ok(())
        }
    }

    /// Clears the armed flag and reports whether io restoration is still needed.
    fn disarm(&mut self) -> bool {
        let armed = self.armed;
        self.armed = false;
        armed
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.restore();
        }
    }
}

/// Undoes [`init`]. Idempotent, so a panic path can call it without checking.
fn restore_terminal() -> io::Result<()> {
    let mut out = io::stdout();
    let left = execute!(out, LeaveAlternateScreen).map_err(io::Error::other);
    let raw = disable_raw_mode();
    let flush = out.flush();
    left.and(raw).and(flush)
}

/// Puts the terminal in raw mode, switches to the alternate screen, and builds
/// the ratatui terminal.
pub fn init() -> TuiResult<Tui> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    out.flush()?;
    Terminal::new(CrosstermBackend::new(out)).map_err(TuiError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Covers the bookkeeping without emitting escape codes into the test output.
    #[test]
    fn a_guard_disarms_exactly_once() {
        let mut guard = TerminalGuard::arm();
        assert!(guard.is_armed());
        assert!(guard.disarm(), "the first restore must do the io work");
        assert!(!guard.is_armed());
        assert!(!guard.disarm(), "a second restore must be a no-op");
    }
}
