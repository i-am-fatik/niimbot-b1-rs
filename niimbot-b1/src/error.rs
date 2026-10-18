use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;

use crate::protocol::{Hex, PRINTHEAD_PX};

type Cause = Box<dyn std::error::Error + Send + Sync>;

/// A failure talking to the printer.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum ErrorKind {
    Transport(Cause),
    Timeout { command: u8 },
    Printer { command: u8, payload: Vec<u8> },
    Malformed { command: u8 },
    TooLarge { width: usize, height: usize },
    Unfinished,
}

impl Error {
    /// Wraps a failure of the link to the printer, for use by [`Transport`](crate::Transport)
    /// implementations.
    pub fn transport(cause: impl Into<Cause>) -> Self {
        Self::new(ErrorKind::Transport(cause.into()))
    }

    pub(crate) fn timeout(command: u8) -> Self {
        Self::new(ErrorKind::Timeout { command })
    }

    pub(crate) fn printer(command: u8, payload: Vec<u8>) -> Self {
        Self::new(ErrorKind::Printer { command, payload })
    }

    pub(crate) fn malformed(command: u8) -> Self {
        Self::new(ErrorKind::Malformed { command })
    }

    pub(crate) fn too_large(width: usize, height: usize) -> Self {
        Self::new(ErrorKind::TooLarge { width, height })
    }

    pub(crate) fn unfinished() -> Self {
        Self::new(ErrorKind::Unfinished)
    }

    fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            backtrace: Backtrace::capture(),
        }
    }

    /// The link to the printer failed, so the connection is no longer usable.
    #[must_use]
    pub fn is_transport(&self) -> bool {
        matches!(self.kind, ErrorKind::Transport(_))
    }

    /// The printer did not answer in time, or did not finish the print in time.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        matches!(self.kind, ErrorKind::Timeout { .. } | ErrorKind::Unfinished)
    }

    /// The printer refused a command.
    #[must_use]
    pub fn is_printer(&self) -> bool {
        matches!(self.kind, ErrorKind::Printer { .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::Transport(cause) => write!(f, "printer link failed: {cause}")?,
            ErrorKind::Timeout { command } => write!(f, "no response to command {command:#04x}")?,
            ErrorKind::Printer { command, payload } => {
                write!(
                    f,
                    "printer error for command {command:#04x}: {}",
                    Hex(payload)
                )?;
            }
            ErrorKind::Malformed { command } => {
                write!(f, "malformed response to command {command:#04x}")?;
            }
            ErrorKind::TooLarge { width, height } => {
                write!(
                    f,
                    "image {width}x{height} px does not fit the {PRINTHEAD_PX} px print head"
                )?;
            }
            ErrorKind::Unfinished => f.write_str("print did not finish")?,
        }
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}
