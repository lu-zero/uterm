//! uterm: a small, standalone terminal emulator built on Rio's VT/PTY stack.

mod app;
mod cli;
mod config;
mod draw;
mod error;
mod event;
mod handler;
mod terminal;

use crate::cli::{RunOptions, Startup};
use crate::event::UiEvent;
use crate::handler::Uterm;
use winit::event_loop::{ControlFlow, EventLoop};

pub use crate::error::Error;

pub fn run() -> Result<(), Error> {
    let RunOptions {
        geometry,
        profile,
        config,
    } = match cli::parse_args()? {
        Startup::Help => {
            println!("{}", cli::help_text());
            return Ok(());
        }
        Startup::Run(options) => options,
    };
    let session = config::load(config.as_deref(), profile.as_deref(), geometry)?;
    let geometry = session.geometry;
    let fonts = session.fonts;
    let event_loop = EventLoop::<UiEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = Uterm {
        proxy,
        state: None,
        geometry,
        fonts,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}
