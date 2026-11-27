use rio_vt::clipboard::ClipboardType;
use rio_vt::event::{EventListener, RioEvent, WindowId};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;

pub(crate) enum UiEvent {
    Wakeup,
    TitleSync,
    TabExited(usize),
    TitleChanged(usize, String),
    ClipboardStore(ClipboardType, String),
    ClipboardLoad(
        usize,
        ClipboardType,
        Arc<dyn Fn(&str) -> String + Sync + Send + 'static>,
    ),
}

/// Bridge between rio's background threads and the winit event loop.
///
/// Winit 0.31's proxy only carries an untyped wake-up; the event data goes
/// through a channel that `ApplicationHandler::proxy_wake_up` drains.
#[derive(Clone)]
pub(crate) struct Listener {
    pub(crate) sender: Sender<UiEvent>,
    pub(crate) proxy: EventLoopProxy,
}

impl Listener {
    fn notify(&self, event: UiEvent) {
        if self.sender.send(event).is_ok() {
            self.proxy.wake_up();
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: RioEvent, _id: WindowId) {
        let ui = match event {
            RioEvent::CloseTerminal(route) | RioEvent::ChildExited(route, _) => {
                UiEvent::TabExited(route)
            }
            RioEvent::Title(route, title) => UiEvent::TitleChanged(route, title),
            RioEvent::ClipboardStore(ty, text) => UiEvent::ClipboardStore(ty, text),
            RioEvent::ClipboardLoad(route, ty, formatter) => {
                UiEvent::ClipboardLoad(route, ty, formatter)
            }
            RioEvent::SyncWindowTitle => UiEvent::TitleSync,
            _ => UiEvent::Wakeup,
        };
        self.notify(ui);
    }
}
