//! When the Wayland portal session binds. Each family's configure call only
//! changes what is wanted and returns; one task binds the whole set a moment
//! after the last change, so the calls Librelog makes at startup, or an
//! unmount and remount, never show the desktop's dialog for a part of the
//! set or for a set it already binds. Binding waits for the user's answer
//! outside every lock. A dismissed dialog is not shown again until the user
//! asks (Settings) or Librelog restarts; a session the desktop closes is
//! bound again.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

use super::bindings::Binding;
#[cfg(target_os = "linux")]
use super::portal::Triggers;
#[cfg(not(target_os = "linux"))]
type Triggers = std::collections::HashMap<String, String>;

pub(super) const DEBOUNCE: Duration = Duration::from_secs(1);
/// A session the desktop closes sooner than this is not bound again on its
/// own: the desktop would keep closing it.
pub(super) const MIN_LIFETIME: Duration = Duration::from_secs(10);

pub(super) enum Failure {
    Dismissed,
    Failed(String),
}

#[cfg(target_os = "linux")]
impl From<super::portal::Failure> for Failure {
    fn from(failure: super::portal::Failure) -> Self {
        match failure {
            super::portal::Failure::Dismissed => Self::Dismissed,
            super::portal::Failure::Failed(message) => Self::Failed(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Status {
    Unused,
    Waiting(Vec<Binding>),
    Bound(Vec<Binding>, Triggers),
    Dismissed(Vec<Binding>),
    Failed(String),
}

pub(super) trait Desktop: Send + Sync + 'static {
    type Session: Send + 'static;
    fn wanted(&self) -> impl Future<Output = Result<Vec<Binding>, String>> + Send;
    fn bind(
        &self,
        bindings: Vec<Binding>,
        binder: Arc<Binder>,
    ) -> impl Future<Output = Result<(Self::Session, Triggers), Failure>> + Send;
    fn close(&self, session: Self::Session) -> impl Future<Output = ()> + Send;
    fn ended(&self, session: &Self::Session) -> bool;
    fn report(&self, status: Status);
}

#[derive(Default)]
pub struct Binder {
    changed: Notify,
    dismissed: AtomicBool,
    started: AtomicBool,
}

impl Binder {
    /// What is wanted changed, or the desktop closed the session.
    pub fn changed(&self) {
        self.changed.notify_one();
    }

    /// The user asked to see the desktop's dialog again.
    pub fn ask_again(&self) {
        self.dismissed.store(false, Ordering::SeqCst);
        self.changed();
    }

    /// Whether this call should start the task: only the first.
    pub fn start(&self) -> bool {
        !self.started.swap(true, Ordering::SeqCst)
    }
}

pub(super) async fn run<D: Desktop>(binder: Arc<Binder>, desktop: D) {
    let mut session: Option<D::Session> = None;
    let mut bound: Vec<Binding> = Vec::new();
    let mut bound_at = tokio::time::Instant::now();
    loop {
        binder.changed.notified().await;
        // Settle: every further change restarts the wait.
        loop {
            tokio::select! {
                _ = binder.changed.notified() => {}
                _ = tokio::time::sleep(DEBOUNCE) => break,
            }
        }
        if session
            .as_ref()
            .is_some_and(|session| desktop.ended(session))
        {
            session = None;
            bound.clear();
            if bound_at.elapsed() < MIN_LIFETIME {
                desktop.report(Status::Failed(
                    "Your desktop closed Librelog's shortcuts. Ask your desktop again to bind them."
                        .into(),
                ));
                binder.dismissed.store(true, Ordering::SeqCst);
                continue;
            }
        }
        let wanted = match desktop.wanted().await {
            Ok(wanted) => wanted,
            Err(message) => {
                desktop.report(Status::Failed(message));
                continue;
            }
        };
        if session.is_some() && wanted == bound {
            continue;
        }
        if let Some(session) = session.take() {
            desktop.close(session).await;
        }
        bound.clear();
        if wanted.is_empty() {
            desktop.report(Status::Unused);
            continue;
        }
        if binder.dismissed.load(Ordering::SeqCst) {
            desktop.report(Status::Dismissed(wanted));
            continue;
        }
        desktop.report(Status::Waiting(wanted.clone()));
        match desktop.bind(wanted.clone(), binder.clone()).await {
            Ok((opened, triggers)) => {
                session = Some(opened);
                bound_at = tokio::time::Instant::now();
                desktop.report(Status::Bound(wanted.clone(), triggers));
                bound = wanted;
            }
            Err(Failure::Dismissed) => {
                binder.dismissed.store(true, Ordering::SeqCst);
                desktop.report(Status::Dismissed(wanted));
            }
            Err(Failure::Failed(message)) => desktop.report(Status::Failed(message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::super::bindings::Does;
    use super::super::{Action, parse_shortcut};
    use super::*;

    fn binding(id: &str) -> Binding {
        Binding {
            id: id.into(),
            description: id.into(),
            trigger: id.into(),
            does: Does::Act(Action::PasteLast),
            key: Some(parse_shortcut("Alt+KeyV").unwrap()),
        }
    }

    #[derive(Clone, Default)]
    struct Fake {
        wanted: Arc<Mutex<Vec<Binding>>>,
        binds: Arc<Mutex<Vec<Vec<String>>>>,
        dismiss: Arc<AtomicBool>,
        /// The sessions' "the desktop closed it" flags.
        sessions: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
        reports: Arc<Mutex<Vec<Status>>>,
    }

    impl Fake {
        fn want(&self, binder: &Binder, ids: &[&str]) {
            *self.wanted.lock().unwrap() = ids.iter().map(|id| binding(id)).collect();
            binder.changed();
        }
        fn binds(&self) -> Vec<Vec<String>> {
            self.binds.lock().unwrap().clone()
        }
    }

    impl Desktop for Fake {
        type Session = Arc<AtomicBool>;
        async fn wanted(&self) -> Result<Vec<Binding>, String> {
            Ok(self.wanted.lock().unwrap().clone())
        }
        async fn bind(
            &self,
            bindings: Vec<Binding>,
            _: Arc<Binder>,
        ) -> Result<(Self::Session, Triggers), Failure> {
            self.binds
                .lock()
                .unwrap()
                .push(bindings.into_iter().map(|binding| binding.id).collect());
            if self.dismiss.load(Ordering::SeqCst) {
                return Err(Failure::Dismissed);
            }
            let session = Arc::new(AtomicBool::new(false));
            self.sessions.lock().unwrap().push(session.clone());
            Ok((session, Triggers::default()))
        }
        async fn close(&self, _: Self::Session) {}
        fn ended(&self, session: &Self::Session) -> bool {
            session.load(Ordering::SeqCst)
        }
        fn report(&self, status: Status) {
            self.reports.lock().unwrap().push(status);
        }
    }

    async fn settle() {
        tokio::time::sleep(DEBOUNCE * 2).await;
    }

    /// Each family's configure call bound at once: four dialogs at startup,
    /// the first three for part of the set.
    #[tokio::test(start_paused = true)]
    async fn the_startup_calls_bind_the_whole_set_once_and_a_remount_binds_nothing() {
        let binder = Arc::new(Binder::default());
        let fake = Fake::default();
        tokio::spawn(run(binder.clone(), fake.clone()));

        fake.want(&binder, &["dictate"]);
        tokio::time::sleep(Duration::from_millis(300)).await;
        fake.want(&binder, &["dictate", "paste-last"]);
        tokio::time::sleep(Duration::from_millis(300)).await;
        fake.want(&binder, &["dictate", "paste-last", "annotate"]);
        settle().await;
        assert_eq!(fake.binds(), [["dictate", "paste-last", "annotate"]]);

        // Unmounted and mounted again inside the wait: the same set.
        fake.want(&binder, &["paste-last", "annotate"]);
        fake.want(&binder, &["dictate", "paste-last", "annotate"]);
        settle().await;
        assert_eq!(fake.binds().len(), 1);

        fake.want(&binder, &["dictate", "annotate"]);
        settle().await;
        assert_eq!(fake.binds().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_dismissed_dialog_waits_for_the_user_and_a_closed_session_is_bound_again() {
        let binder = Arc::new(Binder::default());
        let fake = Fake::default();
        tokio::spawn(run(binder.clone(), fake.clone()));

        fake.dismiss.store(true, Ordering::SeqCst);
        fake.want(&binder, &["dictate"]);
        settle().await;
        fake.want(&binder, &["dictate", "paste-last"]);
        settle().await;
        assert_eq!(fake.binds().len(), 1);
        assert!(matches!(
            fake.reports.lock().unwrap().last(),
            Some(Status::Dismissed(_))
        ));

        fake.dismiss.store(false, Ordering::SeqCst);
        binder.ask_again();
        settle().await;
        assert_eq!(fake.binds().len(), 2);

        // The desktop ends a session that ran a while: bound again.
        tokio::time::sleep(MIN_LIFETIME).await;
        fake.sessions.lock().unwrap()[0].store(true, Ordering::SeqCst);
        binder.changed();
        settle().await;
        assert_eq!(fake.binds().len(), 3);

        // One it ends at once is left for the user, so it cannot loop.
        fake.sessions.lock().unwrap()[1].store(true, Ordering::SeqCst);
        binder.changed();
        settle().await;
        assert_eq!(fake.binds().len(), 3);
        assert!(matches!(
            fake.reports.lock().unwrap().last(),
            Some(Status::Failed(_))
        ));
    }
}
