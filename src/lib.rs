//! uterm: a small, standalone terminal emulator built on Rio's VT/PTY stack.

mod app;
mod cli;
mod config;
mod draw;
mod error;
mod event;
mod fonts;
mod handler;
mod keybindings;
mod terminal;

use crate::cli::{RunOptions, Startup};
use crate::event::{Listener, UiEvent};
use crate::handler::Uterm;
use std::cell::RefCell;
use std::fmt::Display;
use std::io::{self, Write};
use std::rc::Rc;
use std::sync::mpsc;
use winit::event_loop::{ControlFlow, EventLoop};

pub use crate::error::Error;

/// Print lines to stdout, stopping quietly if the reader closes the pipe.
fn print_lines(lines: impl IntoIterator<Item = impl Display>) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in lines {
        if writeln!(out, "{line}").is_err() {
            break;
        }
    }
}

pub fn run() -> Result<(), Error> {
    let RunOptions {
        geometry,
        profile,
        config,
        list_fonts,
        command,
    } = match cli::parse_args()? {
        Startup::Help(text) => {
            print_lines(text.lines());
            return Ok(());
        }
        Startup::Run(options) => options,
    };
    if list_fonts {
        print_lines(fonts::font_families());
        return Ok(());
    }
    let config::Session {
        geometry,
        face,
        font_size,
        command,
    } = config::load(config.as_deref(), profile.as_deref(), geometry, command)?;
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let (sender, receiver) = mpsc::channel::<UiEvent>();
    let listener = Listener {
        sender,
        proxy: event_loop.create_proxy(),
    };
    // `run_app` consumes the handler, so the first error is kept in a shared
    // slot that we can read after the event loop returns.
    let error = Rc::new(RefCell::new(None));
    let app = Uterm {
        listener,
        receiver,
        state: None,
        geometry,
        face: Some(face),
        font_size,
        command,
        error: error.clone(),
        pending_drop: None,
    };
    event_loop.run_app(app)?;
    let result = error.borrow_mut().take().map_or(Ok(()), Err);
    result
}
