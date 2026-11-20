use rio_vt::clipboard::ClipboardType;
use rio_vt::event::{EventListener, RioEvent, WindowId};
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

#[derive(Clone)]
pub(crate) struct Listener {
    pub(crate) proxy: EventLoopProxy<UiEvent>,
}

impl EventListener for Listener {
    fn send_event(&self, event: RioEvent, _id: WindowId) {
        match event {
            RioEvent::CloseTerminal(route) | RioEvent::ChildExited(route, _) => {
                let _ = self.proxy.send_event(UiEvent::TabExited(route));
            }
            RioEvent::Title(route, title) => {
                let _ = self.proxy.send_event(UiEvent::TitleChanged(route, title));
            }
            RioEvent::ClipboardStore(ty, text) => {
                let _ = self.proxy.send_event(UiEvent::ClipboardStore(ty, text));
            }
            RioEvent::ClipboardLoad(route, ty, formatter) => {
                let _ = self
                    .proxy
                    .send_event(UiEvent::ClipboardLoad(route, ty, formatter));
            }
            RioEvent::ResetTitle => {
                let _ = self.proxy.send_event(UiEvent::Wakeup);
            }
            RioEvent::SyncWindowTitle => {
                let _ = self.proxy.send_event(UiEvent::TitleSync);
            }
            _ => {
                let _ = self.proxy.send_event(UiEvent::Wakeup);
            }
        }
    }
}
