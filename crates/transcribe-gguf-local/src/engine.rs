use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::time::Instant;

use anlg_transcribe_gguf::{EngineTranscript, Request};

/// What the server transcribes with: the transcribe.cpp engine, or a fake in
/// tests.
pub trait Transcriber: Send + 'static {
    fn transcribe(
        &mut self,
        samples: &[f32],
        request: &Request,
    ) -> Result<EngineTranscript, anlg_transcribe_gguf::Error>;

    /// The model's name for the response metadata.
    fn model_name(&self) -> String;

    /// Whether the model times its own words or segments, so nearby speech
    /// can go to it in one run: fewer, longer runs are much faster, while a
    /// model without timestamps gets each stretch of speech alone, which
    /// keeps its spread-out word times close to the speech.
    fn times_words(&self) -> bool {
        false
    }
}

impl Transcriber for anlg_transcribe_gguf::Engine {
    fn transcribe(
        &mut self,
        samples: &[f32],
        request: &Request,
    ) -> Result<EngineTranscript, anlg_transcribe_gguf::Error> {
        anlg_transcribe_gguf::Engine::transcribe(self, samples, request)
    }

    fn model_name(&self) -> String {
        let (architecture, variant) = anlg_transcribe_gguf::Engine::model_name(self);
        format!("{architecture}/{variant}")
    }

    fn times_words(&self) -> bool {
        self.timestamps() != anlg_transcribe_gguf::TimestampSource::None
    }
}

pub type BoxedTranscriber = Box<dyn Transcriber>;
pub type LoadFn = Arc<dyn Fn() -> Result<BoxedTranscriber, String> + Send + Sync>;

/// An idle model is released after this long; the next request loads it again.
#[cfg(not(test))]
const IDLE_UNLOAD: Duration = Duration::from_secs(60);
#[cfg(not(test))]
const IDLE_CHECK: Duration = Duration::from_secs(3);
#[cfg(test)]
const IDLE_UNLOAD: Duration = Duration::from_millis(400);
#[cfg(test)]
const IDLE_CHECK: Duration = Duration::from_millis(20);

struct SlotState {
    engine: Option<BoxedTranscriber>,
    last_used: Instant,
    /// A job has the model out.
    lent: bool,
    /// Kept loaded until then, idle or not: a recording that will be
    /// transcribed when it stops.
    held_until: Option<Instant>,
    /// The newest hold applied; holds are sent separately and may arrive
    /// out of order, so an older one is ignored.
    hold_generation: u64,
}

impl SlotState {
    fn held(&self) -> bool {
        self.held_until.is_some_and(|until| until > Instant::now())
    }
}

/// The loaded model, loaded on first use and released when idle, unless it
/// is held. Batch jobs run one at a time, so a job takes the model out and
/// puts it back.
#[derive(Clone)]
pub(crate) struct EngineSlot {
    load: LoadFn,
    state: Arc<tokio::sync::Mutex<SlotState>>,
}

impl EngineSlot {
    pub(crate) fn new(load: LoadFn, preloaded: Option<BoxedTranscriber>) -> Self {
        let slot = Self {
            load,
            state: Arc::new(tokio::sync::Mutex::new(SlotState {
                engine: preloaded,
                last_used: Instant::now(),
                lent: false,
                held_until: None,
                hold_generation: 0,
            })),
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(release_when_idle(Arc::downgrade(&slot.state)));
        }
        slot
    }

    /// The model, loading it when it isn't loaded.
    pub(crate) async fn take(&self) -> Result<BoxedTranscriber, String> {
        let mut state = self.state.lock().await;
        let engine = match state.engine.take() {
            Some(engine) => engine,
            None => self.load().await?,
        };
        state.lent = true;
        Ok(engine)
    }

    async fn load(&self) -> Result<BoxedTranscriber, String> {
        let load = Arc::clone(&self.load);
        tokio::task::spawn_blocking(move || load())
            .await
            .map_err(|error| format!("model load panicked: {error}"))?
    }

    pub(crate) async fn put_back(&self, engine: BoxedTranscriber) {
        let mut state = self.state.lock().await;
        state.engine = Some(engine);
        state.lent = false;
        state.last_used = Instant::now();
    }

    /// Loads the model unless a job has it, and keeps it loaded until
    /// `until`; `None` lets it go idle from now. Ignored unless `generation`
    /// is newer than the last hold's.
    pub(crate) async fn hold(&self, generation: u64, until: Option<Instant>) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if generation <= state.hold_generation {
            return Ok(());
        }
        state.hold_generation = generation;
        state.last_used = Instant::now();
        state.held_until = until;
        if !state.held() || state.engine.is_some() || state.lent {
            return Ok(());
        }
        let engine = self.load().await?;
        state.engine = Some(engine);
        state.last_used = Instant::now();
        Ok(())
    }

    /// Loads the model now, so the first request doesn't wait for it.
    pub(crate) async fn warm(&self) -> Result<(), String> {
        let engine = self.take().await?;
        self.put_back(engine).await;
        Ok(())
    }
}

async fn release_when_idle(state: Weak<tokio::sync::Mutex<SlotState>>) {
    loop {
        tokio::time::sleep(IDLE_CHECK).await;
        let Some(state) = state.upgrade() else {
            return;
        };
        let mut state = state.lock().await;
        if state.engine.is_some() && !state.held() && state.last_used.elapsed() >= IDLE_UNLOAD {
            tracing::info!("releasing the idle transcription model");
            if let Some(engine) = state.engine.take() {
                drop(state);
                let _ = tokio::task::spawn_blocking(move || drop(engine)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Fake;

    impl Transcriber for Fake {
        fn transcribe(
            &mut self,
            _samples: &[f32],
            _request: &Request,
        ) -> Result<EngineTranscript, anlg_transcribe_gguf::Error> {
            unreachable!()
        }

        fn model_name(&self) -> String {
            "fake".into()
        }
    }

    async fn loaded(slot: &EngineSlot) -> bool {
        slot.state.lock().await.engine.is_some()
    }

    #[tokio::test]
    async fn a_held_model_is_loaded_ahead_and_kept_past_idle_until_released() {
        let loads = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&loads);
        let slot = EngineSlot::new(
            Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(Fake) as BoxedTranscriber)
            }),
            None,
        );
        let until = |hold: Duration| Some(Instant::now() + hold);
        slot.hold(1, until(Duration::from_secs(25 * 60)))
            .await
            .unwrap();
        assert!(loaded(&slot).await);

        tokio::time::sleep(IDLE_UNLOAD * 5).await;
        assert!(loaded(&slot).await);
        let engine = slot.take().await.unwrap();
        slot.put_back(engine).await;
        assert_eq!(loads.load(Ordering::SeqCst), 1);

        slot.hold(2, None).await.unwrap();
        tokio::time::sleep(IDLE_UNLOAD / 2).await;
        assert!(loaded(&slot).await);
        tokio::time::sleep(IDLE_UNLOAD / 2 + IDLE_CHECK * 5).await;
        assert!(!loaded(&slot).await);

        // A lost release: the hold ends by itself.
        slot.hold(3, until(IDLE_UNLOAD)).await.unwrap();
        tokio::time::sleep(IDLE_UNLOAD + IDLE_CHECK * 5).await;
        assert!(!loaded(&slot).await);
    }

    #[tokio::test]
    async fn a_hold_that_arrives_after_its_release_is_ignored() {
        let slot = EngineSlot::new(Arc::new(|| Ok(Box::new(Fake) as BoxedTranscriber)), None);
        slot.hold(2, None).await.unwrap();
        slot.hold(1, Some(Instant::now() + Duration::from_secs(25 * 60)))
            .await
            .unwrap();
        assert!(!loaded(&slot).await);

        slot.warm().await.unwrap();
        tokio::time::sleep(IDLE_UNLOAD + IDLE_CHECK * 5).await;
        assert!(!loaded(&slot).await);
    }
}
