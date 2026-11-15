//! The overlay's input shape as the X server sees it, on a connection of
//! our own: it confirms what GTK was asked to do, and it lets the pointer
//! go when GTK's main thread does not answer.

use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::{ClipOrdering, ConnectionExt as _};

/// The X server, behind a trait for tests.
pub trait Shape: Send + Sync {
    /// Whether the pointer goes through window `xid`: its input shape is
    /// empty.
    fn passes_pointer(&self, xid: u32) -> Result<bool, String>;
    /// Empties `xid`'s input shape.
    fn release(&self, xid: u32) -> Result<(), String>;
    fn unmap(&self, xid: u32) -> Result<(), String>;
}

const LATE: &str = "the app's windows did not answer in time";

/// After the main thread was asked to make the overlay take (`takes`) or
/// let go of the pointer, with its `answer` (None: not in time): Ok only
/// once the X server shows it. A release that isn't confirmed is made
/// through X directly, so the pointer never stays caught.
pub fn settle(
    answer: Option<Result<(), String>>,
    takes: bool,
    xid: u32,
    x11: &dyn Shape,
) -> Result<(), String> {
    let settled = match answer {
        Some(Ok(())) => match x11.passes_pointer(xid) {
            Ok(passes) if passes != takes => Ok(()),
            Ok(_) => Err("the overlay's input shape did not change".to_owned()),
            Err(error) => Err(error),
        },
        Some(Err(error)) => Err(error),
        None => Err(LATE.to_owned()),
    };
    if settled.is_err()
        && !takes
        && let Err(error) = x11.release(xid)
    {
        tracing::warn!(%error, "annotate: releasing the pointer through X");
    }
    settled
}

/// A window the main thread could not close in time is unmapped through X.
pub fn closed(answer: Option<()>, xid: Option<u32>, x11: &dyn Shape) {
    if answer.is_none()
        && let Some(xid) = xid
        && let Err(error) = x11.unmap(xid)
    {
        tracing::warn!(%error, "annotate: unmapping the overlay through X");
    }
}

/// A fresh connection each time: these calls are rare, and one must not
/// depend on the state of the last.
pub struct Server;

fn connect() -> Result<x11rb::rust_connection::RustConnection, String> {
    x11rb::connect(None)
        .map(|(connection, _)| connection)
        .map_err(|error| format!("could not reach the X server: {error}"))
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl Shape for Server {
    fn passes_pointer(&self, xid: u32) -> Result<bool, String> {
        let connection = connect()?;
        let reply = connection
            .shape_get_rectangles(xid, SK::INPUT)
            .map_err(text)?
            .reply()
            .map_err(text)?;
        Ok(reply.rectangles.is_empty())
    }

    fn release(&self, xid: u32) -> Result<(), String> {
        let connection = connect()?;
        connection
            .shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, xid, 0, 0, &[])
            .map_err(text)?
            .check()
            .map_err(text)?;
        connection.flush().map_err(text)
    }

    fn unmap(&self, xid: u32) -> Result<(), String> {
        let connection = connect()?;
        self.release(xid)?;
        connection
            .unmap_window(xid)
            .map_err(text)?
            .check()
            .map_err(text)?;
        connection.flush().map_err(text)
    }
}
