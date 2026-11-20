use crate::error::Error;
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

/// Columns and rows accepted by the PTY winsize and by `rio-vt`.
///
/// Zero columns reach an unsafe row initializer that only `debug_assert`s
/// `columns >= 1`. Values above `u16::MAX` truncate when stored in a winsize.
pub(crate) fn checked_grid(cols: usize, rows: usize) -> Result<(u16, u16), Error> {
    match (u16::try_from(cols), u16::try_from(rows)) {
        (Ok(cols), Ok(rows)) if cols >= 1 && rows >= 1 => Ok((cols, rows)),
        _ => Err(Error::GridSize { cols, rows }),
    }
}

/// Pixel extent of `cells` cells. Saturates at `u16::MAX` because winsize
/// stores pixels in a `u16` and the value is advisory.
fn winsize_px(cells: u16, cell_px: usize) -> u16 {
    usize::from(cells)
        .checked_mul(cell_px)
        .and_then(|px| u16::try_from(px).ok())
        .unwrap_or(u16::MAX)
}

/// Pixel extent that has to fit in a window size (`u32`).
pub(crate) fn cells_to_px(
    cells: usize,
    cell_px: usize,
    cols: usize,
    rows: usize,
) -> Result<u32, Error> {
    cells
        .checked_mul(cell_px)
        .and_then(|px| u32::try_from(px).ok())
        .ok_or(Error::GridSize { cols, rows })
}

impl Tab {
    pub(crate) fn new(
        id: usize,
        cols: usize,
        rows: usize,
        cell_w: usize,
        cell_h: usize,
        listener: Listener,
    ) -> Result<Self, Error> {
        let (cols_u16, rows_u16) = checked_grid(cols, rows)?;
        let cols = usize::from(cols_u16);
        let rows = usize::from(rows_u16);
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
            cols_u16,
            rows_u16,
            winsize_px(cols_u16, cell_w),
            winsize_px(rows_u16, cell_h),
        )?;
        let machine = Machine::new(terminal.clone(), pty, listener, WindowId::from(0), id)
            .map_err(Error::Machine)?;
        let channel = machine.channel();
        let thread = machine.spawn();
        Ok(Tab {
            id,
            title: "shell".to_string(),
            terminal,
            pty: PtyHandle {
                channel,
                _thread: thread,
            },
        })
    }

    pub(crate) fn resize(
        &self,
        cols: usize,
        rows: usize,
        cell_w: usize,
        cell_h: usize,
    ) -> Result<(), Error> {
        let (cols_u16, rows_u16) = checked_grid(cols, rows)?;
        let size = TermSize {
            cols: usize::from(cols_u16),
            rows: usize::from(rows_u16),
            cell_w: cell_w as f32,
            cell_h: cell_h as f32,
        };
        {
            let mut term = self.terminal.lock();
            term.resize(size);
        }
        let _ = self.pty.channel.send(Msg::Resize(WindowSize {
            rows: rows_u16,
            cols: cols_u16,
            width: winsize_px(cols_u16, cell_w),
            height: winsize_px(rows_u16, cell_h),
        }));
        Ok(())
    }
}
