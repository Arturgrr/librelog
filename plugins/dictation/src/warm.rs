//! An optional microphone kept open between dictations, so a dictation can
//! include what was said just before its shortcut: Hex's warm capture and its
//! 450 ms pre-roll (MIT, Christopher Langton).

use std::collections::VecDeque;
use std::ops::Range;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::stream::BoxStream;
use futures_util::{Stream, StreamExt};
use tauri::async_runtime::JoinHandle;
use tokio::sync::mpsc;

use crate::Error;
use crate::recorder::SAMPLE_RATE;

/// Audio a dictation keeps from before its shortcut was pressed.
pub(crate) const PRE_ROLL: Duration = Duration::from_millis(450);
/// The longest a start may take to reach the recorder after the press (HUD,
/// permission checks) and still count back to the press.
const START_DELAY: Duration = Duration::from_millis(1_500);
/// What the open microphone remembers: the longest lookback.
const RING: Duration = START_DELAY.saturating_add(PRE_ROLL);
/// How long a microphone that stopped waits before it opens again, and how
/// many times it tries until the settings or the default input change.
const REOPEN_DELAY: Duration = Duration::from_millis(500);
const MAX_REOPENS: u32 = 5;

pub(crate) type Samples = BoxStream<'static, Result<Arc<[f32]>, Error>>;
type Sender = mpsc::UnboundedSender<Result<Arc<[f32]>, Error>>;

fn samples_in(duration: Duration) -> u64 {
    (u128::from(SAMPLE_RATE) * duration.as_millis() / 1_000) as u64
}

/// How far back a dictation starts from the moment its start reaches the
/// recorder: through the time since the press and the pre-roll before it.
/// Muting, lowering or pausing other audio takes effect only after the press,
/// so the audio before it holds whatever was playing (Hex #1309, #1468):
/// those dictations start at once.
pub(crate) fn lookback(
    since_press: Duration,
    behavior: Option<&anlg_recording_environment::AudioBehavior>,
) -> Duration {
    match behavior {
        None | Some(anlg_recording_environment::AudioBehavior::Nothing) => {
            since_press.min(START_DELAY) + PRE_ROLL
        }
        Some(_) => Duration::ZERO,
    }
}

/// Where a recording starts in the open microphone's audio, and the start
/// cue it leaves out, both as offsets in samples from now (negative is
/// earlier).
#[derive(Debug, Clone, Default)]
pub(crate) struct Window {
    pub(crate) lookback: Duration,
    pub(crate) cue: Option<Range<i64>>,
}

impl Window {
    /// Sample offsets from now for the span between `start` and `end`.
    pub(crate) fn cue_between(
        start: std::time::SystemTime,
        end: std::time::SystemTime,
    ) -> Range<i64> {
        let now = std::time::SystemTime::now();
        let offset = |at: std::time::SystemTime| match at.duration_since(now) {
            Ok(ahead) => samples_in(ahead) as i64,
            Err(behind) => -(samples_in(behind.duration()) as i64),
        };
        offset(start)..offset(end)
    }
}

struct Listener {
    sender: Sender,
    /// Where its audio starts, counted in samples since the microphone opened.
    from: u64,
    skip: Range<u64>,
    /// Waiting for an earlier recording still reading, so the two never
    /// share audio.
    waiting: bool,
    /// How far it has been sent.
    sent: u64,
}

impl Listener {
    /// Sends the part of `samples`, which start at `at`, that is its own;
    /// false once nobody reads it.
    fn send(&mut self, at: u64, samples: &[f32]) -> bool {
        let end = at + samples.len() as u64;
        let kept: Vec<f32> = (at.max(self.from)..end)
            .filter(|position| !self.skip.contains(position))
            .map(|position| samples[(position - at) as usize])
            .collect();
        self.sent = self.sent.max(end);
        kept.is_empty() || self.sender.send(Ok(kept.into())).is_ok()
    }
}

#[derive(Default)]
struct Ring {
    samples: VecDeque<f32>,
    /// Samples heard since the microphone opened.
    total: u64,
    listeners: Vec<Listener>,
    /// Where the last recording to stop reading stopped; no later one starts
    /// before it, so a quick re-press never repeats its last words.
    recorded_until: u64,
    /// The Settings level meter, which reads beside a recording.
    meter: Option<Sender>,
    open: bool,
}

impl Ring {
    fn start(&self) -> u64 {
        self.total - self.samples.len() as u64
    }

    fn forget_closed(&mut self) {
        let recorded_until = &mut self.recorded_until;
        self.listeners.retain(|listener| {
            let closed = listener.sender.is_closed();
            if closed && !listener.waiting {
                *recorded_until = (*recorded_until).max(listener.sent);
            }
            !closed
        });
    }

    fn in_use(&mut self) -> bool {
        self.forget_closed();
        !self.listeners.is_empty()
    }

    /// Hands the waiting listeners the audio from where the last recording
    /// stopped, once no recording reads.
    fn wake_waiting(&mut self) {
        if self.listeners.iter().any(|listener| !listener.waiting) {
            return;
        }
        let start = self.start();
        let backlog: Vec<f32> = self.samples.iter().copied().collect();
        let recorded_until = self.recorded_until;
        self.listeners.retain_mut(|listener| {
            listener.waiting = false;
            listener.from = listener.from.max(recorded_until);
            listener.send(start, &backlog)
        });
    }

    fn push(&mut self, samples: Arc<[f32]>) {
        self.forget_closed();
        let at = self.total;
        self.samples.extend(samples.iter().copied());
        self.total += samples.len() as u64;
        let excess = self.samples.len().saturating_sub(samples_in(RING) as usize);
        self.samples.drain(..excess);
        if let Some(meter) = &self.meter
            && meter.send(Ok(samples.clone())).is_err()
        {
            self.meter = None;
        }
        let recorded_until = &mut self.recorded_until;
        self.listeners.retain_mut(|listener| {
            if listener.waiting || listener.send(at, &samples) {
                return true;
            }
            *recorded_until = (*recorded_until).max(at);
            false
        });
        self.wake_waiting();
    }

    /// The live audio for the level meter, without taking it from a
    /// recording. A new meter replaces the last one.
    fn meter(&mut self) -> Option<Samples> {
        if !self.open {
            return None;
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        self.meter = Some(sender);
        Some(receive(receiver))
    }

    /// The live audio for one recording, from `window.lookback` before now
    /// but never before the last recording stopped, without the start cue.
    /// While an earlier recording still reads (its release tail), this one
    /// starts where that one stops.
    fn listen(&mut self, window: &Window) -> Option<Samples> {
        if !self.open {
            return None;
        }
        self.forget_closed();
        let now = self.total as i64;
        let skip = window
            .cue
            .as_ref()
            .map(|cue| (now + cue.start).max(0) as u64..(now + cue.end).max(0) as u64)
            .unwrap_or(0..0);
        let (sender, receiver) = mpsc::unbounded_channel();
        let from = self
            .total
            .saturating_sub(samples_in(window.lookback))
            .max(self.start())
            .max(self.recorded_until);
        let mut listener = Listener {
            sender,
            from,
            skip,
            waiting: true,
            sent: from,
        };
        if !self.listeners.iter().any(|listener| !listener.waiting) {
            listener.waiting = false;
            let backlog: Vec<f32> = self.samples.iter().copied().collect();
            listener.send(self.start(), &backlog);
        }
        self.listeners.push(listener);
        Some(receive(receiver))
    }

    fn end(&mut self, error: Option<Error>) {
        self.open = false;
        self.meter = None;
        let listeners = std::mem::take(&mut self.listeners);
        if let Some(error) = error {
            let message = error.to_string();
            for listener in listeners {
                let _ = listener.sender.send(Err(Error::Capture(message.clone())));
            }
        }
    }
}

fn receive(mut receiver: mpsc::UnboundedReceiver<Result<Arc<[f32]>, Error>>) -> Samples {
    futures_util::stream::poll_fn(move |context| receiver.poll_recv(context)).boxed()
}

/// The open microphone; dropping it closes the microphone.
struct WarmMicrophone {
    device: Option<String>,
    ring: Arc<Mutex<Ring>>,
    task: JoinHandle<()>,
}

impl WarmMicrophone {
    fn open(
        audio: &dyn anlg_audio::AudioProvider,
        device: Option<String>,
        chunk_size: usize,
        ended: impl FnOnce() + Send + 'static,
    ) -> Result<Self, Error> {
        let mut stream = audio
            .open_mic_capture(device.clone(), SAMPLE_RATE, chunk_size)
            .map_err(|error| Error::Capture(error.to_string()))?;
        let ring = Arc::new(Mutex::new(Ring {
            open: true,
            ..Ring::default()
        }));
        let task_ring = ring.clone();
        let task = tauri::async_runtime::spawn(async move {
            let error = loop {
                match stream.next().await {
                    Some(Ok(frame)) => lock(&task_ring).push(frame.preferred_mic()),
                    Some(Err(error)) => break Some(Error::Capture(error.to_string())),
                    None => break None,
                }
            };
            tracing::warn!(?error, "the open dictation microphone stopped");
            lock(&task_ring).end(error);
            drop(stream);
            ended();
        });
        Ok(Self { device, ring, task })
    }

    fn is_open(&self) -> bool {
        lock(&self.ring).open
    }
}

impl Drop for WarmMicrophone {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// What the open microphone should be.
#[derive(Clone)]
pub(crate) struct WarmRequest {
    pub(crate) audio: Arc<dyn anlg_audio::AudioProvider>,
    /// The microphone the person chose; `None` follows the system default.
    pub(crate) device: Option<String>,
    pub(crate) avoid_bluetooth: bool,
    pub(crate) chunk_size: usize,
}

#[derive(Default)]
struct State {
    microphone: Option<WarmMicrophone>,
    wanted: Option<WarmRequest>,
    /// The default input changed, so a microphone on it opens again.
    default_changed: bool,
    reopens: u32,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    watch: Option<anlg_device_monitor::DeviceMonitorHandle>,
}

/// The open microphone and what it should be. A change while a recording
/// reads it waits for that recording to stop reading, so a dictation never
/// loses its microphone mid-sentence.
#[derive(Clone, Default)]
pub(crate) struct Warm(Arc<Mutex<State>>);

impl Warm {
    /// Opens the microphone as `wanted` says, or closes it for `None`.
    pub(crate) fn set(&self, wanted: Option<WarmRequest>) -> Result<(), Error> {
        let mut state = lock(&self.0);
        state.wanted = wanted;
        state.reopens = 0;
        settle(&self.0, &mut state)
    }

    /// The open microphone's audio for one recording on `device`, or `None`
    /// when it is not open on it. Once the recording lets go of it, a change
    /// that waited for it applies.
    pub(crate) fn listen(&self, device: &Option<String>, window: &Window) -> Option<Samples> {
        let samples = {
            let state = lock(&self.0);
            let microphone = state.microphone.as_ref().filter(|m| &m.device == device)?;
            lock(&microphone.ring).listen(window)?
        };
        Some(
            Settling {
                samples: Some(samples),
                warm: Arc::downgrade(&self.0),
            }
            .boxed(),
        )
    }

    /// The open microphone's audio for the level meter, when it is open on
    /// `device`.
    pub(crate) fn meter(&self, device: &Option<String>) -> Option<Samples> {
        let state = lock(&self.0);
        let microphone = state.microphone.as_ref().filter(|m| &m.device == device)?;
        lock(&microphone.ring).meter()
    }
}

/// Brings the open microphone in line with what is wanted, unless a
/// recording reads it.
fn settle(slot: &Arc<Mutex<State>>, state: &mut State) -> Result<(), Error> {
    if let Some(microphone) = &state.microphone
        && lock(&microphone.ring).in_use()
    {
        return Ok(());
    }
    let Some(wanted) = state.wanted.clone() else {
        state.microphone = None;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            state.watch = None;
        }
        return Ok(());
    };
    let device = crate::microphone::choose(
        wanted.audio.as_ref(),
        wanted.device.clone(),
        wanted.avoid_bluetooth,
    )
    .device;
    let default_changed = std::mem::take(&mut state.default_changed);
    // A stream on the default input may stay on the input it opened with.
    let keep = state.microphone.as_ref().is_some_and(|microphone| {
        microphone.device == device
            && microphone.is_open()
            && !(default_changed && device.is_none())
    });
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        if wanted.device.is_none() {
            if state.watch.is_none() {
                state.watch = Some(follow_default_input(Arc::downgrade(slot)));
            }
        } else {
            state.watch = None;
        }
    }
    if keep {
        return Ok(());
    }
    // The old stream closes before another one opens.
    state.microphone = None;
    let weak = Arc::downgrade(slot);
    state.microphone = Some(WarmMicrophone::open(
        wanted.audio.as_ref(),
        device,
        wanted.chunk_size,
        move || reopen_later(weak),
    )?);
    Ok(())
}

/// Settles off the async runtime: choosing and opening a microphone blocks.
fn settle_soon(slot: Weak<Mutex<State>>, change: impl FnOnce(&mut State) -> bool + Send + 'static) {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(slot) = slot.upgrade() else {
            return;
        };
        let mut state = lock(&slot);
        if !change(&mut state) {
            return;
        }
        if let Err(error) = settle(&slot, &mut state) {
            tracing::warn!(%error, "the dictation microphone could not open again");
            if state.wanted.is_some() && state.reopens < MAX_REOPENS {
                drop(state);
                reopen_later(Arc::downgrade(&slot));
            }
        }
    });
}

/// A microphone that stopped (unplugged, a stream error) opens again.
fn reopen_later(slot: Weak<Mutex<State>>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(REOPEN_DELAY).await;
        settle_soon(slot, |state| {
            if state
                .microphone
                .as_ref()
                .is_some_and(WarmMicrophone::is_open)
                || state.reopens >= MAX_REOPENS
            {
                return false;
            }
            state.reopens += 1;
            true
        });
    });
}

/// Opens the microphone again on the new default input, or on the wired
/// one a newly connected headset gives way to.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn follow_default_input(slot: Weak<Mutex<State>>) -> anlg_device_monitor::DeviceMonitorHandle {
    use anlg_device_monitor::{DeviceSwitch, DeviceSwitchMonitor};
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = DeviceSwitchMonitor::spawn_debounced(sender);
    std::thread::spawn(move || {
        for event in receiver {
            if matches!(event, DeviceSwitch::DefaultOutputChanged { .. }) {
                continue;
            }
            if slot.strong_count() == 0 {
                break;
            }
            settle_soon(slot.clone(), |state| {
                state.default_changed = true;
                state.reopens = 0;
                true
            });
        }
    });
    handle
}

/// A recording's audio from the open microphone; dropping it applies a
/// change that waited for the recording.
struct Settling {
    samples: Option<Samples>,
    warm: Weak<Mutex<State>>,
}

impl Stream for Settling {
    type Item = Result<Arc<[f32]>, Error>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.samples.as_mut() {
            Some(samples) => samples.poll_next_unpin(context),
            None => Poll::Ready(None),
        }
    }
}

impl Drop for Settling {
    fn drop(&mut self) {
        // Closed first, so the microphone no longer counts it as reading.
        drop(self.samples.take());
        settle_soon(self.warm.clone(), |_| true);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring() -> Ring {
        Ring {
            open: true,
            ..Ring::default()
        }
    }

    /// Each sample is its own position, so what a listener hears says where
    /// it came from.
    fn push(ring: &mut Ring, count: u64) {
        let at = ring.total;
        ring.push((at..at + count).map(|position| position as f32).collect());
    }

    async fn heard(samples: &mut Samples) -> Vec<f32> {
        let mut heard = Vec::new();
        while let Ok(Some(next)) =
            tokio::time::timeout(Duration::from_millis(5), samples.next()).await
        {
            heard.extend_from_slice(&next.unwrap());
        }
        heard
    }

    fn positions(range: Range<u64>) -> Vec<f32> {
        range.map(|position| position as f32).collect()
    }

    fn back(duration: Duration) -> Window {
        Window {
            lookback: duration,
            cue: None,
        }
    }

    /// A dictation from the open microphone starts with the audio from just
    /// before its press, then carries on live; what the ring forgot is gone,
    /// and it remembers the longest lookback.
    #[tokio::test]
    async fn a_listener_gets_the_pre_roll_then_live_audio() {
        let mut ring = ring();
        let second = u64::from(SAMPLE_RATE);
        push(&mut ring, 4 * second);
        let mut samples = ring.listen(&back(PRE_ROLL)).unwrap();
        push(&mut ring, 320);

        let total = 4 * second + 320;
        assert_eq!(
            heard(&mut samples).await,
            positions(4 * second - samples_in(PRE_ROLL)..total)
        );
        assert_eq!(ring.samples.len() as u64, samples_in(RING));
        assert!(
            lookback(Duration::from_secs(10), None) <= RING,
            "the longest lookback outgrows the ring"
        );
    }

    /// A quick re-press: the last dictation keeps its release tail, and the
    /// next one starts where it stopped instead of repeating its last words.
    #[tokio::test]
    async fn a_quick_re_press_neither_cuts_the_last_dictation_nor_repeats_it() {
        let mut ring = ring();
        push(&mut ring, 16_000);
        let mut first = ring.listen(&back(Duration::ZERO)).unwrap();
        push(&mut ring, 3_200);
        let mut second = ring.listen(&back(Duration::from_secs(1))).unwrap();
        push(&mut ring, 1_600);

        assert_eq!(heard(&mut first).await, positions(16_000..20_800));
        assert!(heard(&mut second).await.is_empty());
        drop(first);
        push(&mut ring, 320);
        assert_eq!(heard(&mut second).await, positions(20_800..21_120));
    }

    /// The start cue sounds just after the press, so it is left out of the
    /// pre-roll and of the live audio it reaches.
    #[tokio::test]
    async fn the_start_cue_is_left_out_of_the_recording() {
        let mut ring = ring();
        push(&mut ring, 16_000);
        let mut samples = ring
            .listen(&Window {
                lookback: Duration::from_millis(500),
                cue: Some(-3_200..1_600),
            })
            .unwrap();
        push(&mut ring, 3_200);

        let mut expected = positions(8_000..12_800);
        expected.extend(positions(17_600..19_200));
        assert_eq!(heard(&mut samples).await, expected);
    }

    /// Hex #1316: with other audio muted, lowered or paused while dictating,
    /// the audio before the press holds what was playing, so none is kept.
    #[test]
    fn quieting_other_audio_keeps_no_pre_roll() {
        use anlg_recording_environment::AudioBehavior;
        let since_press = Duration::from_millis(200);
        assert_eq!(lookback(since_press, None), since_press + PRE_ROLL);
        assert_eq!(
            lookback(since_press, Some(&AudioBehavior::Nothing)),
            since_press + PRE_ROLL
        );
        assert_eq!(
            lookback(since_press, Some(&AudioBehavior::Mute)),
            Duration::ZERO
        );
    }
}
