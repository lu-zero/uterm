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
use crate::event::UiEvent;
use crate::handler::Uterm;
use std::fmt::Display;
use std::io::{self, Write};
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
    let event_loop = EventLoop::<UiEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = Uterm {
        proxy,
        state: None,
        geometry,
        face: Some(face),
        font_size,
        command,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}
