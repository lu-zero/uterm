//! uterm: a small, standalone terminal emulator built on Rio's VT/PTY stack.

mod app;
mod cli;
mod draw;
mod event;
mod handler;
mod terminal;

use crate::event::UiEvent;
use crate::handler::Uterm;
use winit::event_loop::{ControlFlow, EventLoop};

pub fn run() {
    let event_loop = EventLoop::<UiEvent>::with_user_event().build().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = Uterm { proxy, state: None };
    event_loop.run_app(&mut app).unwrap();
}
