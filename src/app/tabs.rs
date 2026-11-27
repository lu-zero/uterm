use super::{AppState, LocalPointerState};
use crate::error::Error;
use crate::terminal::Tab;
use winit::event_loop::ActiveEventLoop;

impl AppState {
    pub(crate) fn clamp_active(&mut self) {
        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
    }

    pub(crate) fn select_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = idx;
            self.sync_window_title();
        }
    }

    pub(crate) fn remove_tab_index(&mut self, idx: usize, event_loop: &dyn ActiveEventLoop) {
        if idx >= self.tabs.len() {
            return;
        }
        let was_active = idx == self.active;
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.active = 0;
            self.local_pointer = LocalPointerState::Idle;
            event_loop.exit();
            self.window.set_title("uterm");
            return;
        }
        if was_active {
            self.active = idx.min(self.tabs.len() - 1);
        } else if idx < self.active {
            self.active = self.active.saturating_sub(1);
        }
        self.sync_window_title();
    }

    pub(crate) fn new_tab(&mut self) -> Result<(), Error> {
        let (cols, rows) = self.grid_dims();
        let listener = self.listener.clone();
        let id = self.next_tab_id;
        // A startup command belongs to the initial tab only; new tabs run the
        // default shell.
        let tab = Tab::new(id, cols, rows, self.cell_w, self.cell_h, listener, &[])?;
        self.next_tab_id += 1;
        self.tabs.push(tab);
        self.select_tab(self.tabs.len() - 1);
        Ok(())
    }

    pub(crate) fn sync_window_title(&self) {
        if let Some(tab) = self.tabs.get(self.active) {
            let title = if tab.title.trim().is_empty() {
                "uterm".to_string()
            } else {
                format!("{} — uterm", tab.title.trim())
            };
            self.window.set_title(&title);
        } else {
            self.window.set_title("uterm");
        }
    }

    pub(crate) fn close_active_tab(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.tabs.is_empty() {
            return;
        }
        self.clamp_active();
        let idx = self.active;
        self.remove_tab_index(idx, event_loop);
    }

    pub(crate) fn close_tab_by_id(&mut self, id: usize, event_loop: &dyn ActiveEventLoop) {
        if let Some(idx) = self.tabs.iter().position(|t| t.id == id) {
            self.remove_tab_index(idx, event_loop);
        }
    }

    pub(crate) fn next_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let idx = (self.active + 1) % self.tabs.len();
        self.select_tab(idx);
    }

    pub(crate) fn prev_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let idx = (self.active + self.tabs.len() - 1) % self.tabs.len();
        self.select_tab(idx);
    }

    pub(crate) fn set_tab_title(&mut self, id: usize, title: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            let t = title.trim().to_string();
            tab.title = if t.is_empty() { "shell".to_string() } else { t };
        }
        self.sync_window_title();
    }
}
