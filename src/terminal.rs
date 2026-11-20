use crate::event::Listener;
use rio_vt::ansi::CursorShape;
use rio_vt::crosswords::grid::Dimensions;
use rio_vt::crosswords::Crosswords;
use rio_vt::event::sync::FairMutex;
use rio_vt::event::{Msg, WindowId, WindowSize};
use rio_vt::performer::Machine;
use std::sync::Arc;

pub(crate) struct TermSize {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) cell_w: f32,
    pub(crate) cell_h: f32,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
    fn square_width(&self) -> f32 {
        self.cell_w
    }
    fn square_height(&self) -> f32 {
        self.cell_h
    }
}

pub(crate) struct PtyHandle {
    pub(crate) channel: rio_vt::corcovado::channel::Sender<Msg>,
    pub(crate) _thread: std::thread::JoinHandle<(
        Machine<rio_vt::teletypewriter::Pty, Listener>,
        rio_vt::performer::State,
    )>,
}

pub(crate) struct Tab {
    pub(crate) id: usize,
    pub(crate) title: String,
    pub(crate) terminal: Arc<FairMutex<Crosswords<Listener>>>,
    pub(crate) pty: PtyHandle,
}

impl Tab {
    pub(crate) fn new(
        id: usize,
        cols: usize,
        rows: usize,
        cell_w: usize,
        cell_h: usize,
        listener: Listener,
    ) -> Self {
        let terminal = Arc::new(FairMutex::new(Crosswords::new(
            TermSize {
                cols,
                rows,
                cell_w: cell_w as f32,
                cell_h: cell_h as f32,
            },
            CursorShape::Block,
            listener.clone(),
            WindowId::from(0),
            id,
            10_000,
        )));
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let pty = rio_vt::teletypewriter::create_pty_with_spawn(
            None,
            Vec::new(),
            &cwd,
            None,
            cols as u16,
            rows as u16,
            (cols * cell_w) as u16,
            (rows * cell_h) as u16,
        )
        .expect("create pty");
        let machine =
            Machine::new(terminal.clone(), pty, listener, WindowId::from(0), id).expect("machine");
        let channel = machine.channel();
        let thread = machine.spawn();
        Tab {
            id,
            title: "shell".to_string(),
            terminal,
            pty: PtyHandle {
                channel,
                _thread: thread,
            },
        }
    }

    pub(crate) fn resize(&self, cols: usize, rows: usize, cell_w: usize, cell_h: usize) {
        let size = TermSize {
            cols,
            rows,
            cell_w: cell_w as f32,
            cell_h: cell_h as f32,
        };
        {
            let mut term = self.terminal.lock();
            term.resize(size);
        }
        let _ = self.pty.channel.send(Msg::Resize(WindowSize {
            rows: rows as u16,
            cols: cols as u16,
            width: (cols * cell_w) as u16,
            height: (rows * cell_h) as u16,
        }));
    }
}
