//! Failures that used to abort the process.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Usage(String),
    Config(String),
    /// Columns or rows outside `1..=u16::MAX`. `rio-vt` asserts on zero columns.
    GridSize {
        cols: usize,
        rows: usize,
    },
    EventLoop(winit::error::EventLoopError),
    Window(winit::error::OsError),
    Surface(softbuffer::SoftBufferError),
    Font(String),
    Pty(std::io::Error),
    Machine(Box<dyn std::error::Error>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(msg) => f.write_str(msg),
            Error::Config(msg) => f.write_str(msg),
            Error::GridSize { cols, rows } => write!(
                f,
                "terminal size {cols}x{rows} is invalid; both sides must be in 1..=65535"
            ),
            Error::EventLoop(err) => write!(f, "event loop failed: {err}"),
            Error::Window(err) => write!(f, "failed to create the window: {err}"),
            Error::Surface(err) => write!(f, "failed to create the drawing surface: {err}"),
            Error::Font(err) => f.write_str(err),
            Error::Pty(err) => write!(f, "failed to create the pty: {err}"),
            Error::Machine(err) => write!(f, "failed to start the terminal: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::EventLoop(err) => Some(err),
            Error::Window(err) => Some(err),
            Error::Surface(err) => Some(err),
            Error::Pty(err) => Some(err),
            Error::Machine(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl From<winit::error::EventLoopError> for Error {
    fn from(err: winit::error::EventLoopError) -> Self {
        Error::EventLoop(err)
    }
}

impl From<winit::error::OsError> for Error {
    fn from(err: winit::error::OsError) -> Self {
        Error::Window(err)
    }
}

impl From<softbuffer::SoftBufferError> for Error {
    fn from(err: softbuffer::SoftBufferError) -> Self {
        Error::Surface(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Pty(err)
    }
}
