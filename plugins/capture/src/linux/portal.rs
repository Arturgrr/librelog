//! The desktop's portals through `ashpd`, on a runtime of their own so the
//! blocking native calls can wait on them from any worker thread. Every
//! call has a deadline; a cancelled dialog is `None`, a missing portal or a
//! refusal an error that says which.

use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anlg_annotate_macos::{AnnotateError, codes};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::screenshot::Screenshot;
use ashpd::desktop::{PersistMode, ResponseError, Session};
use futures_util::StreamExt;

use super::area::Stream;
use super::fail;
use super::record::{Cancel, Cast, CastRequest, CastSession, Source};
use super::screenshot::Shots;

/// How long the desktop's picker or sharing dialog may stay up.
const DIALOG_WITHIN: Duration = Duration::from_secs(180);
/// A call that shows nothing.
const CALL_WITHIN: Duration = Duration::from_secs(20);
const CANCEL_POLL: Duration = Duration::from_millis(100);

pub struct Desktop {
    runtime: tokio::runtime::Runtime,
}

impl Desktop {
    pub fn new() -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("capture-portal")
            .enable_all()
            .build()?;
        Ok(Self { runtime })
    }
}

fn portal_error(what: &str, error: ashpd::Error) -> AnnotateError {
    match error {
        ashpd::Error::PortalNotFound(_) => fail(
            codes::INTERNAL,
            format!(
                "this desktop has no {what} portal: install xdg-desktop-portal and your desktop's portal (xdg-desktop-portal-gnome on GNOME, xdg-desktop-portal-kde on KDE)"
            ),
        ),
        ashpd::Error::Response(ResponseError::Other) => {
            fail(codes::INTERNAL, format!("the desktop refused the {what}"))
        }
        other => fail(
            codes::INTERNAL,
            format!("the {what} portal failed: {other}"),
        ),
    }
}

fn timed_out(what: &str) -> AnnotateError {
    fail(
        codes::TIMEOUT,
        format!("the desktop's {what} did not answer in time"),
    )
}

fn cancelled_by_user(error: &ashpd::Error) -> bool {
    matches!(error, ashpd::Error::Response(ResponseError::Cancelled))
}

impl Shots for Desktop {
    fn grab(&self, interactive: bool) -> Result<Option<PathBuf>, AnnotateError> {
        let within = if interactive {
            DIALOG_WITHIN
        } else {
            CALL_WITHIN
        };
        let taken = self.runtime.block_on(tokio::time::timeout(within, async {
            Screenshot::request()
                .connection(Some(connection().await?))
                .interactive(interactive)
                .modal(false)
                .send()
                .await?
                .response()
        }));
        let shot = match taken {
            Err(_) => return Err(timed_out("screenshot")),
            Ok(Err(error)) if cancelled_by_user(&error) => return Ok(None),
            Ok(Err(error)) => return Err(portal_error("screenshot", error)),
            Ok(Ok(shot)) => shot,
        };
        url::Url::parse(shot.uri().as_str())
            .ok()
            .and_then(|uri| uri.to_file_path().ok())
            .map(Some)
            .ok_or_else(|| {
                fail(
                    codes::IO,
                    format!("the screenshot is not a local file: {}", shot.uri()),
                )
            })
    }
}

/// The bus connection registered with the portal under Librelog's app id,
/// so the desktop knows who asks (and keeps its choices for it).
async fn connection() -> Result<ashpd::zbus::Connection, ashpd::Error> {
    anlg_desktop_portal::connection()
        .await
        .map_err(ashpd::Error::Zbus)
}

struct Live {
    handle: tokio::runtime::Handle,
    proxy: Screencast,
    session: Option<Arc<Session<Screencast>>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    stream: Stream,
    token: Option<String>,
    closed: Arc<AtomicBool>,
}

impl CastSession for Live {
    fn open_remote(&mut self) -> Result<OwnedFd, AnnotateError> {
        let what = "screen sharing";
        let session = self
            .session
            .clone()
            .ok_or_else(|| fail(codes::INTERNAL, "the screen-sharing session is closed"))?;
        self.handle
            .block_on(tokio::time::timeout(
                CALL_WITHIN,
                self.proxy
                    .open_pipe_wire_remote(&session, Default::default()),
            ))
            .map_err(|_| timed_out(what))?
            .map_err(|error| portal_error(what, error))
    }

    fn stream(&self) -> Stream {
        self.stream
    }

    fn restore_token(&self) -> Option<String> {
        self.token.clone()
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn close(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(session) = self.session.take() {
            let _ = self
                .handle
                .block_on(tokio::time::timeout(CALL_WITHIN, session.close()));
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.close();
    }
}

impl Cast for Desktop {
    fn start(
        &self,
        request: &CastRequest,
        cancel: &Cancel,
    ) -> Result<Option<Box<dyn CastSession>>, AnnotateError> {
        let handle = self.runtime.handle().clone();
        let what = "screen sharing";
        let proxy = self
            .runtime
            .block_on(tokio::time::timeout(CALL_WITHIN, async {
                Screencast::with_connection(connection().await?).await
            }))
            .map_err(|_| timed_out(what))?
            .map_err(|error| portal_error(what, error))?;
        let session = self
            .runtime
            .block_on(tokio::time::timeout(
                CALL_WITHIN,
                proxy.create_session(Default::default()),
            ))
            .map_err(|_| timed_out(what))?
            .map_err(|error| portal_error(what, error))?;
        let close = |session: Session<Screencast>| {
            let _ = handle.block_on(tokio::time::timeout(CALL_WITHIN, session.close()));
        };
        let sources: ashpd::enumflags2::BitFlags<SourceType> = match request.source {
            Source::Monitor => SourceType::Monitor.into(),
            Source::Window => SourceType::Window.into(),
        };
        let options = SelectSourcesOptions::default()
            .set_cursor_mode(if request.cursor {
                CursorMode::Embedded
            } else {
                CursorMode::Hidden
            })
            .set_sources(sources)
            .set_multiple(false)
            .set_persist_mode(match request.source {
                // A screen is remembered; a window is picked each time.
                Source::Monitor => PersistMode::ExplicitlyRevoked,
                Source::Window => PersistMode::DoNot,
            })
            .set_restore_token(request.restore.as_deref());
        let selected = self
            .runtime
            .block_on(tokio::time::timeout(CALL_WITHIN, async {
                proxy.select_sources(&session, options).await?.response()
            }));
        match selected {
            Err(_) => {
                close(session);
                return Err(timed_out(what));
            }
            Ok(Err(error)) if cancelled_by_user(&error) => {
                close(session);
                return Ok(None);
            }
            Ok(Err(error)) => {
                close(session);
                return Err(portal_error(what, error));
            }
            Ok(Ok(())) => {}
        }
        // The dialog: waits for the user, a cancel, or the deadline.
        let started = self.runtime.block_on(async {
            let start = async {
                proxy
                    .start(&session, None, Default::default())
                    .await?
                    .response()
            };
            let cancelled = async {
                while !cancel.cancelled() {
                    tokio::time::sleep(CANCEL_POLL).await;
                }
            };
            tokio::select! {
                started = tokio::time::timeout(DIALOG_WITHIN, start) => Some(started),
                () = cancelled => None,
            }
        });
        let streams = match started {
            None => {
                close(session);
                return Ok(None);
            }
            Some(Err(_)) => {
                close(session);
                return Err(timed_out(what));
            }
            Some(Ok(Err(error))) if cancelled_by_user(&error) => {
                close(session);
                return Ok(None);
            }
            Some(Ok(Err(error))) => {
                close(session);
                return Err(portal_error(what, error));
            }
            Some(Ok(Ok(streams))) => streams,
        };
        let Some(first) = streams.streams().first() else {
            close(session);
            return Ok(None);
        };
        let stream = Stream {
            node: first.pipe_wire_node_id(),
            size: first.size(),
            position: first.position(),
        };
        let token = streams.restore_token().map(str::to_owned);
        let closed = Arc::new(AtomicBool::new(false));
        let session = Arc::new(session);
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        {
            let session = session.clone();
            let closed = closed.clone();
            // Until the desktop closes the session, or we do.
            self.runtime.spawn(async move {
                let watch = async {
                    if let Ok(mut signal) = session.receive_closed().await {
                        signal.next().await;
                        closed.store(true, Ordering::SeqCst);
                    }
                };
                tokio::select! {
                    () = watch => {}
                    _ = stopped => {}
                }
            });
        }
        Ok(Some(Box::new(Live {
            handle,
            proxy,
            session: Some(session),
            stop: Some(stop),
            stream,
            token,
            closed,
        })))
    }
}
