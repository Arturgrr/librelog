//! Copying a capture on Linux. A screenshot goes as PNG data, a GIF or a
//! movie as a file reference (`text/uri-list` and GNOME's
//! `x-special/gnome-copied-files`, which file managers and chat apps
//! paste as the file), each with the other formats that fit. Librelog
//! owns the X11 CLIPBOARD selection and serves every format from a thread
//! until another client takes it; GNOME bridges that selection to Wayland
//! apps through XWayland. Without X11, `wl-copy` serves the main format.
//! The selection owner is adapted from Hex's `src/linux/clipboard.rs` (MIT,
//! see THIRD_PARTY_NOTICES.md).

use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

use anlg_annotate_macos::{AnnotateError, codes};

use super::fail;

pub const PNG: &str = "image/png";
pub const GIF: &str = "image/gif";
pub const URI_LIST: &str = "text/uri-list";
pub const GNOME_FILES: &str = "x-special/gnome-copied-files";
/// What the clipboard serves, larger formats past this are left out.
const MAX_DATA: u64 = 64 * 1024 * 1024;

/// Puts formats on the clipboard, the first one the main one.
pub trait Clipboard: Send + Sync {
    fn set(&self, formats: Vec<(String, Vec<u8>)>) -> Result<(), AnnotateError>;
}

/// The formats a capture of `kind` at `path` goes on the clipboard as.
pub fn formats(path: &Path, kind: &str) -> Result<Vec<(String, Vec<u8>)>, AnnotateError> {
    let uri = url::Url::from_file_path(path)
        .map_err(|()| {
            fail(
                codes::INVALID_ARGUMENT,
                "the capture's path is not absolute",
            )
        })?
        .to_string();
    let files = vec![
        (URI_LIST.to_owned(), format!("{uri}\r\n").into_bytes()),
        (GNOME_FILES.to_owned(), format!("copy\n{uri}").into_bytes()),
    ];
    let small = std::fs::metadata(path)
        .map_err(|error| fail(codes::IO, error))?
        .len()
        <= MAX_DATA;
    Ok(match kind {
        "screenshot" => {
            let bytes = std::fs::read(path).map_err(|error| fail(codes::IO, error))?;
            // A PNG goes as it is; a JPEG is converted.
            let png = if bytes.starts_with(b"\x89PNG") {
                bytes
            } else {
                let picture =
                    image::load_from_memory(&bytes).map_err(|error| fail(codes::IO, error))?;
                let mut png = Cursor::new(Vec::new());
                picture
                    .write_to(&mut png, image::ImageFormat::Png)
                    .map_err(|error| fail(codes::IO, error))?;
                png.into_inner()
            };
            std::iter::once((PNG.to_owned(), png))
                .chain(files)
                .collect()
        }
        "gif" if small => {
            let gif = std::fs::read(path).map_err(|error| fail(codes::IO, error))?;
            files
                .into_iter()
                .chain(std::iter::once((GIF.to_owned(), gif)))
                .collect()
        }
        _ => files,
    })
}

/// The desktop's clipboard: X11 (also under XWayland), else `wl-copy`.
pub struct Desktop;

impl Clipboard for Desktop {
    fn set(&self, formats: Vec<(String, Vec<u8>)>) -> Result<(), AnnotateError> {
        let x11 = std::env::var_os("DISPLAY").is_some_and(|display| !display.is_empty());
        let wayland =
            std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty());
        let x11_error = if x11 {
            match x11::serve(formats.clone()) {
                Ok(()) => return Ok(()),
                Err(error) => Some(error),
            }
        } else {
            None
        };
        if wayland && super::gst::installed("wl-copy") {
            let Some((mime, data)) = formats.into_iter().next() else {
                return Ok(());
            };
            return wl_copy(&mime, data).map_err(|error| fail(codes::IO, error));
        }
        Err(fail(
            codes::IO,
            match x11_error {
                Some(error) => format!("could not use the clipboard: {error}"),
                None => "could not use the clipboard: there is no X11 display, and wl-copy (the wl-clipboard package) is not installed".to_owned(),
            },
        ))
    }
}

/// `wl-copy` forks a server that outlives it and keeps whatever output it
/// was given, so it gets none: only its own exit is waited for.
fn wl_copy(mime: &str, data: Vec<u8>) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start wl-copy: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("wl-copy failed ({status})")),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("wl-copy did not finish in time".into());
            }
        }
    }
}

mod x11 {
    use std::collections::HashMap;

    use x11rb::connection::{Connection, RequestConnection};
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{
        AtomEnum, ChangeWindowAttributesAux, ConnectionExt, CreateWindowAux, EventMask, PropMode,
        Property, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, Window, WindowClass,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;
    use x11rb::{COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE};

    use std::time::{Duration, Instant};

    /// A chunked transfer whose reader has not taken a chunk for this long.
    const INCR_ABANDONED: Duration = Duration::from_secs(10);

    x11rb::atom_manager! {
        Atoms: AtomsCookie {
            CLIPBOARD,
            TARGETS,
            INCR,
        }
    }

    struct Format {
        target: u32,
        data: Vec<u8>,
    }

    type Result<T> = std::result::Result<T, String>;

    fn text(error: impl std::fmt::Display) -> String {
        error.to_string()
    }

    /// Takes the clipboard with `formats` and serves it on a thread until
    /// another client takes it.
    pub fn serve(formats: Vec<(String, Vec<u8>)>) -> Result<()> {
        let (connection, screen) = x11rb::connect(None).map_err(text)?;
        let atoms = Atoms::new(&connection)
            .map_err(text)?
            .reply()
            .map_err(text)?;
        let formats = formats
            .into_iter()
            .map(|(mime, data)| {
                let target = connection
                    .intern_atom(false, mime.as_bytes())
                    .map_err(text)?
                    .reply()
                    .map_err(text)?
                    .atom;
                Ok(Format { target, data })
            })
            .collect::<Result<Vec<_>>>()?;
        let root = connection.setup().roots[screen].root;
        let window = connection.generate_id().map_err(text)?;
        connection
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                COPY_FROM_PARENT,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .map_err(text)?
            .check()
            .map_err(text)?;
        connection
            .set_selection_owner(window, atoms.CLIPBOARD, CURRENT_TIME)
            .map_err(text)?
            .check()
            .map_err(text)?;
        let owner = connection
            .get_selection_owner(atoms.CLIPBOARD)
            .map_err(text)?
            .reply()
            .map_err(text)?
            .owner;
        if owner != window {
            return Err("another app kept the clipboard".into());
        }
        std::thread::Builder::new()
            .name("capture-clipboard".into())
            .spawn(move || serve_until_cleared(&connection, &atoms, window, &formats))
            .map_err(text)?;
        Ok(())
    }

    fn put(
        connection: &RustConnection,
        window: Window,
        property: u32,
        format: &Format,
        data: &[u8],
    ) -> bool {
        connection
            .change_property(
                PropMode::REPLACE,
                window,
                property,
                format.target,
                8,
                data.len() as u32,
                data,
            )
            .is_ok()
    }

    fn serve_until_cleared(
        connection: &RustConnection,
        atoms: &Atoms,
        window: Window,
        formats: &[Format],
    ) {
        // Large formats go in INCR chunks, well inside the request limit.
        let chunk = (connection.maximum_request_bytes() / 2).min(256 * 1024) & !3;
        // By requestor and property: format, offset, and when it last moved.
        let mut transfers = HashMap::<(Window, u32), (usize, usize, Instant)>::new();
        while let Ok(event) = connection.wait_for_event() {
            // A reader that stopped taking its chunks is forgotten.
            transfers.retain(|_, (_, _, moved)| moved.elapsed() < INCR_ABANDONED);
            match event {
                Event::SelectionRequest(request) => {
                    let property = if request.property == NONE {
                        request.target
                    } else {
                        request.property
                    };
                    let served = if request.target == atoms.TARGETS {
                        let targets: Vec<u32> = std::iter::once(atoms.TARGETS)
                            .chain(formats.iter().map(|format| format.target))
                            .collect();
                        connection
                            .change_property32(
                                PropMode::REPLACE,
                                request.requestor,
                                property,
                                AtomEnum::ATOM,
                                &targets,
                            )
                            .is_ok()
                    } else if let Some(index) = formats
                        .iter()
                        .position(|format| format.target == request.target)
                    {
                        let format = &formats[index];
                        if format.data.len() <= chunk {
                            put(
                                connection,
                                request.requestor,
                                property,
                                format,
                                &format.data,
                            )
                        } else {
                            let _ = connection.change_window_attributes(
                                request.requestor,
                                &ChangeWindowAttributesAux::new()
                                    .event_mask(EventMask::PROPERTY_CHANGE),
                            );
                            transfers
                                .insert((request.requestor, property), (index, 0, Instant::now()));
                            connection
                                .change_property32(
                                    PropMode::REPLACE,
                                    request.requestor,
                                    property,
                                    atoms.INCR,
                                    &[format.data.len() as u32],
                                )
                                .is_ok()
                        }
                    } else {
                        false
                    };
                    let _ = connection.send_event(
                        false,
                        request.requestor,
                        EventMask::default(),
                        SelectionNotifyEvent {
                            response_type: SELECTION_NOTIFY_EVENT,
                            sequence: 0,
                            time: request.time,
                            requestor: request.requestor,
                            selection: request.selection,
                            target: request.target,
                            property: if served { property } else { NONE },
                        },
                    );
                }
                Event::PropertyNotify(event) if event.state == Property::DELETE => {
                    let key = (event.window, event.atom);
                    if let Some((index, offset, moved)) = transfers.get_mut(&key) {
                        *moved = Instant::now();
                        let format = &formats[*index];
                        let end = (*offset + chunk).min(format.data.len());
                        put(
                            connection,
                            event.window,
                            event.atom,
                            format,
                            &format.data[*offset..end],
                        );
                        if *offset == end {
                            transfers.remove(&key);
                        } else {
                            *offset = end;
                        }
                    }
                }
                Event::SelectionClear(_) => break,
                _ => {}
            }
            let _ = connection.flush();
        }
        let _ = connection.destroy_window(window);
        let _ = connection.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screenshot_is_copied_as_png_and_a_recording_as_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = dir.path().join("Librelog Screenshot.jpg");
        image::RgbImage::from_pixel(3, 2, image::Rgb([9, 9, 9]))
            .save(&jpeg)
            .unwrap();
        let shot = formats(&jpeg, "screenshot").unwrap();
        let mimes: Vec<&str> = shot.iter().map(|(mime, _)| mime.as_str()).collect();
        assert_eq!(mimes, [PNG, URI_LIST, GNOME_FILES]);
        assert!(shot[0].1.starts_with(b"\x89PNG"));
        let uri = String::from_utf8(shot[1].1.clone()).unwrap();
        assert!(uri.ends_with("/Librelog%20Screenshot.jpg\r\n"), "{uri}");
        assert_eq!(
            String::from_utf8(shot[2].1.clone()).unwrap(),
            format!("copy\n{}", uri.trim_end())
        );

        let movie = dir.path().join("Librelog Recording.webm");
        std::fs::write(&movie, b"webm").unwrap();
        let mimes: Vec<String> = formats(&movie, "video")
            .unwrap()
            .into_iter()
            .map(|(mime, _)| mime)
            .collect();
        assert_eq!(mimes, [URI_LIST, GNOME_FILES]);
        let gif = dir.path().join("a.gif");
        std::fs::write(&gif, b"GIF89a").unwrap();
        let mimes: Vec<String> = formats(&gif, "gif")
            .unwrap()
            .into_iter()
            .map(|(mime, _)| mime)
            .collect();
        assert_eq!(mimes, [URI_LIST, GNOME_FILES, GIF]);
    }
}
