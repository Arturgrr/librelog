//! An optional microphone kept open between dictations, so a dictation can
//! include what was said just before its shortcut: Hex's warm capture and its
//! 450 ms pre-roll (MIT, Christopher Langton).

use std::collections::VecDeque;
use std::ops::Range;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::stream::BoxStream;
use futures_util::{Stream, StreamExt};
use tauri::async_runtime::JoinHandle;
use tokio::sync::mpsc;

use crate::Error;
use crate::recorder::SAMPLE_RATE;
use crate::tail::Voice;

/// Audio a dictation keeps from before its shortcut was pressed.
pub(crate) const PRE_ROLL: Duration = Duration::from_millis(450);
/// The longest a start may take to reach the recorder after the press (HUD,
/// permission checks) and still count back to the press.
const START_DELAY: Duration = Duration::from_millis(1_500);
/// What the open microphone remembers: the longest lookback.
const RING: Duration = START_DELAY.saturating_add(PRE_ROLL);
/// How long a microphone that stopped waits before it opens again, and how
/// many times in a row it tries without hearing a frame.
const REOPEN_DELAY: Duration = Duration::from_millis(if cfg!(test) { 10 } else { 500 });
pub(crate) const MAX_REOPENS: u32 = 5;
/// How much later than it was played the start cue may still reach the
/// recording: the output's and the input's buffers, and the room's echo.
const CUE_LATENCY: Duration = Duration::from_millis(150);
/// The audio after the start cue that tells whether someone talks over it.
const CUE_LOOKAHEAD: Duration = Duration::from_millis(120);

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

/// Where a recording starts in the open microphone's audio, and where the
/// start cue may sound in it, as offsets in samples from now (negative is
/// earlier).
#[derive(Debug, Clone, Default)]
pub(crate) struct Window {
    pub(crate) lookback: Duration,
    pub(crate) cue: Option<Range<i64>>,
}

impl Window {
    /// Sample offsets from now for the cue that started playing at `start`
    /// and stopped at `end`, widened by [`CUE_LATENCY`].
    pub(crate) fn cue_between(
        start: std::time::SystemTime,
        end: std::time::SystemTime,
    ) -> Range<i64> {
        let now = std::time::SystemTime::now();
        let offset = |at: std::time::SystemTime| match at.duration_since(now) {
            Ok(ahead) => samples_in(ahead) as i64,
            Err(behind) => -(samples_in(behind.duration()) as i64),
        };
        offset(start)..offset(end + CUE_LATENCY)
    }
}

/// What a recording tells the open microphone, and how far it has read.
#[derive(Default)]
struct Control {
    /// Thrown away (cancelled): the next recording may hear what it heard.
    discarded: AtomicBool,
    /// Samples the recording has taken from its stream.
    read: AtomicU64,
}

/// The start cue in one recording. Through speakers it reaches the
/// microphone, so the audio from where it may sound to [`CUE_LOOKAHEAD`]
/// after it is held back: when nobody talks into the cue or just after it,
/// the cue's span is silenced; otherwise it is kept as heard, so words said
/// over the cue are never lost. Either way the timeline stays whole.
struct Cue {
    span: Range<u64>,
    held: Vec<f32>,
    /// Where `held` starts.
    held_from: u64,
    /// Judged on the audio around the cue, never on the cue itself.
    voice: Voice,
    talking_before: bool,
}

impl Cue {
    fn hold_end(&self) -> u64 {
        self.span.end + samples_in(CUE_LOOKAHEAD)
    }

    /// Takes the start of `samples`, which begin at `position`, into `out`
    /// or the hold; returns how many it took and whether the cue is done.
    fn take(&mut self, position: u64, samples: &[f32], out: &mut Vec<f32>) -> (usize, bool) {
        let hold_end = self.hold_end();
        if position >= hold_end {
            self.release(out);
            return (0, true);
        }
        if position < self.span.start {
            let count = samples.len().min((self.span.start - position) as usize);
            self.voice.extend(&samples[..count]);
            out.extend_from_slice(&samples[..count]);
            return (count, false);
        }
        if self.held.is_empty() {
            self.held_from = position;
            self.talking_before = self.voice.talking();
        }
        let count = samples.len().min((hold_end - position) as usize);
        self.held.extend_from_slice(&samples[..count]);
        if position + count as u64 == hold_end {
            self.release(out);
            return (count, true);
        }
        (count, false)
    }

    /// Hands over what is held, deciding on what was heard after the cue so
    /// far.
    fn release(&mut self, out: &mut Vec<f32>) {
        let span_end = (self.span.end.saturating_sub(self.held_from) as usize).min(self.held.len());
        self.voice.extend(&self.held[span_end..]);
        if !self.talking_before && !self.voice.talking() {
            self.held[..span_end].fill(0.0);
        }
        out.append(&mut self.held);
    }
}

struct Listener {
    sender: Sender,
    /// Where its audio starts, counted in samples since the microphone opened.
    from: u64,
    /// How far it has been given audio, held cue included.
    sent: u64,
    cue: Option<Cue>,
    control: Arc<Control>,
}

impl Listener {
    /// Sends the part of `samples`, which start at `at`, that is its own;
    /// false once nobody reads it.
    fn send(&mut self, at: u64, samples: &[f32]) -> bool {
        let end = at + samples.len() as u64;
        let mut position = at.max(self.from);
        if position >= end {
            return true;
        }
        let mut out = Vec::with_capacity((end - position) as usize);
        while position < end {
            let rest = &samples[(position - at) as usize..];
            let Some(cue) = &mut self.cue else {
                out.extend_from_slice(rest);
                break;
            };
            let (taken, done) = cue.take(position, rest, &mut out);
            position += taken as u64;
            if done {
                self.cue = None;
            }
        }
        self.sent = end;
        out.is_empty() || self.sender.send(Ok(out.into())).is_ok()
    }

    /// Sends the cue it holds without waiting for the audio after it.
    fn flush(&mut self) {
        if let Some(mut cue) = self.cue.take() {
            let mut out = Vec::new();
            cue.release(&mut out);
            if !out.is_empty() {
                let _ = self.sender.send(Ok(out.into()));
            }
        }
    }

    /// Where the audio its recording took ends.
    fn read_until(&self) -> u64 {
        self.from + self.control.read.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct Ring {
    samples: VecDeque<f32>,
    /// Samples heard since the microphone opened.
    total: u64,
    /// The recording reading it; a newer one takes over, so no two ever
    /// share a sample.
    listener: Option<Listener>,
    /// Where the last kept recording stopped; no later one starts before it,
    /// so a quick re-press never repeats its last words.
    recorded_until: u64,
    /// The Settings level meter, which reads beside a recording.
    meter: Option<Sender>,
    open: bool,
}

impl Ring {
    fn start(&self) -> u64 {
        self.total - self.samples.len() as u64
    }

    /// Lets go of a listener whose recording stopped reading.
    fn forget(&mut self, listener: &Listener) {
        if !listener.control.discarded.load(Ordering::SeqCst) {
            self.recorded_until = self.recorded_until.max(listener.read_until());
        }
    }

    fn forget_closed(&mut self) {
        if let Some(listener) = self
            .listener
            .take_if(|listener| listener.sender.is_closed())
        {
            self.forget(&listener);
        }
    }

    fn in_use(&mut self) -> bool {
        self.forget_closed();
        self.listener.is_some()
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
        if let Some(listener) = self
            .listener
            .take_if(|listener| !listener.send(at, &samples))
        {
            self.forget(&listener);
        }
    }

    /// The live audio for the level meter, without taking it from a
    /// recording. A new meter replaces the last one.
    fn meter(&mut self) -> Option<Samples> {
        if !self.open {
            return None;
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        self.meter = Some(sender);
        Some(receive(receiver, None))
    }

    /// The live audio for one recording, from `window.lookback` before now
    /// but never before the last kept recording stopped. A recording still
    /// reading (its release tail) ends here with what it was given, and
    /// this one starts right after it.
    fn listen(&mut self, window: &Window) -> Option<(Samples, Arc<Control>)> {
        if !self.open {
            return None;
        }
        self.forget_closed();
        if let Some(mut previous) = self.listener.take() {
            previous.flush();
            if !previous.control.discarded.load(Ordering::SeqCst) {
                self.recorded_until = self.recorded_until.max(previous.sent);
            }
        }
        let now = self.total as i64;
        let cue = window
            .cue
            .as_ref()
            .map(|cue| (now + cue.start).max(0) as u64..(now + cue.end).max(0) as u64)
            .filter(|span| !span.is_empty())
            .map(|span| Cue {
                span,
                held: Vec::new(),
                held_from: 0,
                voice: Voice::default(),
                talking_before: false,
            });
        let (sender, receiver) = mpsc::unbounded_channel();
        let from = self
            .total
            .saturating_sub(samples_in(window.lookback))
            .max(self.start())
            .max(self.recorded_until);
        let control = Arc::new(Control::default());
        let mut listener = Listener {
            sender,
            from,
            sent: from,
            cue,
            control: control.clone(),
        };
        let backlog: Vec<f32> = self.samples.iter().copied().collect();
        listener.send(self.start(), &backlog);
        self.listener = Some(listener);
        Some((receive(receiver, Some(control.clone())), control))
    }

    /// The recording `control` belongs to was released: a cue it holds goes
    /// out now, ahead of its release tail.
    fn release(&mut self, control: &Arc<Control>) {
        if let Some(listener) = &mut self.listener
            && Arc::ptr_eq(&listener.control, control)
        {
            listener.flush();
        }
    }

    fn end(&mut self, error: Option<Error>) {
        self.open = false;
        self.meter = None;
        let listener = self.listener.take();
        if let (Some(listener), Some(error)) = (listener, error) {
            let _ = listener.sender.send(Err(Error::Capture(error.to_string())));
        }
    }
}

fn receive(
    mut receiver: mpsc::UnboundedReceiver<Result<Arc<[f32]>, Error>>,
    control: Option<Arc<Control>>,
) -> Samples {
    futures_util::stream::poll_fn(move |context| {
        let next = receiver.poll_recv(context);
        if let (Poll::Ready(Some(Ok(samples))), Some(control)) = (&next, &control) {
            control
                .read
                .fetch_add(samples.len() as u64, Ordering::SeqCst);
        }
        next
    })
    .boxed()
}

/// One recording's hold on the open microphone.
#[derive(Clone)]
pub(crate) struct Reading {
    ring: Weak<Mutex<Ring>>,
    control: Arc<Control>,
}

impl Reading {
    /// The recording was released: audio held back around the start cue is
    /// sent now, so it counts as said before the release, not as its tail.
    pub(crate) fn released(&self) {
        if let Some(ring) = self.ring.upgrade() {
            lock(&ring).release(&self.control);
        }
    }

    /// The recording is thrown away, so the next one may start before
    /// where it stopped.
    pub(crate) fn discard(&self) {
        self.control.discarded.store(true, Ordering::SeqCst);
    }
}

/// The open microphone; dropping it closes the microphone once its task
/// stops, shortly after. Nothing needs the old stream gone before a new one
/// opens: inputs are opened shared, so the two may overlap for a moment.
struct WarmMicrophone {
    device: Option<String>,
    /// The system default input it opened on, for a stream on the default.
    default: Option<String>,
    ring: Arc<Mutex<Ring>>,
    task: JoinHandle<()>,
}

impl WarmMicrophone {
    fn open(
        audio: &dyn anlg_audio::AudioProvider,
        device: Option<String>,
        default: Option<String>,
        chunk_size: usize,
        reopens: Arc<AtomicU32>,
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
            let mut heard = false;
            let error = loop {
                match stream.next().await {
                    Some(Ok(frame)) => {
                        if !heard {
                            // It works again, so a later failure starts a
                            // fresh round of attempts.
                            heard = true;
                            reopens.store(0, Ordering::SeqCst);
                        }
                        lock(&task_ring).push(frame.preferred_mic());
                    }
                    Some(Err(error)) => break Some(Error::Capture(error.to_string())),
                    None => break None,
                }
            };
            tracing::warn!(?error, "the open dictation microphone stopped");
            lock(&task_ring).end(error);
            drop(stream);
            ended();
        });
        Ok(Self {
            device,
            default,
            ring,
            task,
        })
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
    /// Counts the settles begun, so one that took long to open a microphone
    /// gives way to a later one.
    generation: u64,
    /// Attempts to open again since the microphone last heard a frame.
    reopens: Arc<AtomicU32>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    watch: Option<anlg_device_monitor::DeviceMonitorHandle>,
}

impl State {
    fn in_use(&self) -> bool {
        self.microphone
            .as_ref()
            .is_some_and(|microphone| lock(&microphone.ring).in_use())
    }
}

/// The open microphone and what it should be. A change while a recording
/// reads it waits for that recording to stop reading, so a dictation never
/// loses its microphone mid-sentence.
#[derive(Clone, Default)]
pub(crate) struct Warm(Arc<Mutex<State>>);

impl Warm {
    /// Opens the microphone as `wanted` says, or closes it for `None`.
    pub(crate) fn set(&self, wanted: Option<WarmRequest>) -> Result<(), Error> {
        {
            let mut state = lock(&self.0);
            state.wanted = wanted;
            state.generation += 1;
            state.reopens.store(0, Ordering::SeqCst);
        }
        settle(&self.0)
    }

    /// The open microphone's audio for one recording on `device`, or `None`
    /// when it is not open on it. A stream on the default input is only used
    /// while that is still the default, since Windows reports no change of
    /// default; otherwise it opens again on the new one for later dictations.
    /// Once the recording lets go of it, a change that waited for it applies.
    pub(crate) fn listen(
        &self,
        audio: &dyn anlg_audio::AudioProvider,
        device: &Option<String>,
        window: &Window,
    ) -> Option<(Samples, Reading)> {
        let default = device.is_none().then(|| audio.default_device_name());
        let (samples, reading) = {
            let state = lock(&self.0);
            let microphone = state.microphone.as_ref().filter(|m| &m.device == device)?;
            if device.is_none() && microphone.default != default {
                drop(state);
                settle_soon(Arc::downgrade(&self.0), |state| {
                    state.default_changed = true;
                    true
                });
                return None;
            }
            let (samples, control) = lock(&microphone.ring).listen(window)?;
            let reading = Reading {
                ring: Arc::downgrade(&microphone.ring),
                control,
            };
            (samples, reading)
        };
        let samples = Settling {
            samples: Some(samples),
            warm: Arc::downgrade(&self.0),
        }
        .boxed();
        Some((samples, reading))
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
/// recording reads it. Choosing and opening a microphone block, so they run
/// without the lock; a microphone opened meanwhile made obsolete by a later
/// change, or by a recording that started on the old one, is closed again.
fn settle(slot: &Arc<Mutex<State>>) -> Result<(), Error> {
    let (wanted, generation, current, default_changed, reopens) = {
        let mut state = lock(slot);
        if state.in_use() {
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
        state.generation += 1;
        let current = state
            .microphone
            .as_ref()
            .map(|m| (m.device.clone(), m.default.clone(), m.is_open()));
        (
            wanted,
            state.generation,
            current,
            std::mem::take(&mut state.default_changed),
            state.reopens.clone(),
        )
    };
    let device = crate::microphone::choose(
        wanted.audio.as_ref(),
        wanted.device.clone(),
        wanted.avoid_bluetooth,
    )
    .device;
    let default = device.is_none().then(|| wanted.audio.default_device_name());
    // A stream on the default input may stay on the input it opened with.
    let keep = current.is_some_and(|(current, opened_on, open)| {
        current == device
            && open
            && !(device.is_none() && (default_changed || opened_on != default))
    });
    if keep {
        return Ok(());
    }
    let weak = Arc::downgrade(slot);
    let microphone = WarmMicrophone::open(
        wanted.audio.as_ref(),
        device,
        default,
        wanted.chunk_size,
        reopens,
        move || reopen_later(weak),
    )?;
    let mut state = lock(slot);
    if state.generation != generation || state.in_use() {
        state.default_changed |= default_changed;
        drop(state);
        drop(microphone);
        return Ok(());
    }
    let previous = state.microphone.replace(microphone);
    drop(state);
    drop(previous);
    Ok(())
}

/// Settles off the async runtime: choosing and opening a microphone block.
fn settle_soon(slot: Weak<Mutex<State>>, change: impl FnOnce(&mut State) -> bool + Send + 'static) {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(slot) = slot.upgrade() else {
            return;
        };
        if !change(&mut lock(&slot)) {
            return;
        }
        if let Err(error) = settle(&slot) {
            tracing::warn!(%error, "the dictation microphone could not open again");
            let state = lock(&slot);
            if state.wanted.is_some() && state.reopens.load(Ordering::SeqCst) < MAX_REOPENS {
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
                || state.reopens.load(Ordering::SeqCst) >= MAX_REOPENS
            {
                return false;
            }
            state.reopens.fetch_add(1, Ordering::SeqCst);
            true
        });
    });
}

/// Opens the microphone again on the new default input. A change in the
/// devices only matters while a Bluetooth default gives way to a wired
/// input, which may change with it, or while the microphone is closed.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn follow_default_input(slot: Weak<Mutex<State>>) -> anlg_device_monitor::DeviceMonitorHandle {
    use anlg_device_monitor::{DeviceSwitch, DeviceSwitchMonitor};
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = DeviceSwitchMonitor::spawn_debounced(sender);
    std::thread::spawn(move || {
        for event in receiver {
            if slot.strong_count() == 0 {
                break;
            }
            match event {
                DeviceSwitch::DefaultOutputChanged { .. } => {}
                DeviceSwitch::DefaultInputChanged => settle_soon(slot.clone(), |state| {
                    state.default_changed = true;
                    state.reopens.store(0, Ordering::SeqCst);
                    true
                }),
                DeviceSwitch::DeviceListChanged => settle_soon(slot.clone(), |state| {
                    let closed = !state
                        .microphone
                        .as_ref()
                        .is_some_and(WarmMicrophone::is_open);
                    let swaps = state
                        .wanted
                        .as_ref()
                        .is_some_and(|wanted| wanted.avoid_bluetooth);
                    if closed {
                        state.reopens.store(0, Ordering::SeqCst);
                    }
                    closed || swaps
                }),
            }
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

    fn push_level(ring: &mut Ring, count: usize, level: f32) {
        ring.push(vec![level; count].into());
    }

    fn listen(ring: &mut Ring, window: Window) -> (Samples, Arc<Control>) {
        ring.listen(&window).unwrap()
    }

    /// What has arrived so far, and whether the stream ended.
    async fn heard(samples: &mut Samples) -> (Vec<f32>, bool) {
        let mut heard = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_millis(5), samples.next()).await {
                Ok(Some(next)) => heard.extend_from_slice(&next.unwrap()),
                Ok(None) => return (heard, true),
                Err(_) => return (heard, false),
            }
        }
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
        let (mut samples, _) = listen(&mut ring, back(PRE_ROLL));
        push(&mut ring, 320);

        let total = 4 * second + 320;
        assert_eq!(
            heard(&mut samples).await.0,
            positions(4 * second - samples_in(PRE_ROLL)..total)
        );
        assert_eq!(ring.samples.len() as u64, samples_in(RING));
        assert!(
            lookback(Duration::from_secs(10), None) <= RING,
            "the longest lookback outgrows the ring"
        );
    }

    /// A quick re-press while the last dictation still takes its release
    /// tail: the last one ends where the next one starts, with everything it
    /// was given, and the next one gets everything after that at once,
    /// without waiting for the tail.
    #[tokio::test]
    async fn a_quick_re_press_neither_cuts_the_last_dictation_nor_repeats_it() {
        let mut ring = ring();
        push(&mut ring, 16_000);
        let (mut first, _) = listen(&mut ring, back(Duration::ZERO));
        push(&mut ring, 3_200);
        let (mut second, _) = listen(&mut ring, back(Duration::from_secs(1)));
        push(&mut ring, 1_600);

        assert_eq!(heard(&mut first).await, (positions(16_000..19_200), true));
        assert_eq!(heard(&mut second).await, (positions(19_200..20_800), false));
    }

    /// However recordings overlap, stop and start again, no two kept ones
    /// ever hear the same sample.
    #[tokio::test]
    async fn no_two_recordings_hear_the_same_sample() {
        let mut ring = ring();
        push(&mut ring, 16_000);
        let mut all = Vec::new();
        let mut current = listen(&mut ring, back(PRE_ROLL)).0;
        for round in 0..6_u64 {
            push(&mut ring, 1_000 + round * 300);
            if round % 2 == 0 {
                // Stopped on its own before the next press.
                all.extend(heard(&mut current).await.0);
                drop(current);
                push(&mut ring, 500);
                current = listen(&mut ring, back(Duration::from_secs(1))).0;
            } else {
                let next = listen(&mut ring, back(Duration::from_secs(1))).0;
                all.extend(heard(&mut current).await.0);
                current = next;
            }
        }
        all.extend(heard(&mut current).await.0);
        let mut unique = all.clone();
        unique.sort_by(f32::total_cmp);
        unique.dedup();
        assert_eq!(unique.len(), all.len());
    }

    /// Escape throws a dictation away, so pressing again at once still gets
    /// the pre-roll it would have had.
    #[tokio::test]
    async fn a_cancelled_dictation_leaves_the_next_one_its_pre_roll() {
        let mut ring = ring();
        push(&mut ring, 16_000);
        let (mut first, control) = listen(&mut ring, back(Duration::ZERO));
        push(&mut ring, 1_600);
        heard(&mut first).await;
        control.discarded.store(true, Ordering::SeqCst);
        drop(first);
        push(&mut ring, 320);
        let (mut second, _) = listen(&mut ring, back(PRE_ROLL));

        let total = 16_000 + 1_600 + 320;
        assert_eq!(
            heard(&mut second).await.0,
            positions(total - samples_in(PRE_ROLL)..total)
        );
    }

    const ROOM: f32 = 0.0005;
    const CUE: f32 = 0.3;
    const SPEECH: f32 = 0.1;
    /// The pre-roll before the cue, 300 ms, in the 500 ms a recording keeps.
    const AHEAD: usize = 4_800;

    /// A start cue that played for 200 ms until now, widened by the
    /// latency, in a recording that starts 500 ms back.
    fn cue_window() -> Window {
        Window {
            lookback: Duration::from_millis(500),
            cue: Some(-3_200..samples_in(CUE_LATENCY) as i64),
        }
    }

    /// The room, `before` just ahead of the cue, the cue as the microphone
    /// heard it, then `after` until the lookahead is over.
    async fn record_cue(before: f32, after: f32) -> Vec<f32> {
        let mut ring = ring();
        push_level(&mut ring, 16_000 - 3_200 - 2_400, ROOM);
        push_level(&mut ring, 2_400, before);
        push_level(&mut ring, 3_200, CUE);
        let (mut samples, _) = listen(&mut ring, cue_window());
        let rest = samples_in(CUE_LATENCY + CUE_LOOKAHEAD) as usize;
        for _ in 0..rest.div_ceil(320) {
            push_level(&mut ring, 320, after);
        }
        heard(&mut samples).await.0
    }

    /// The start cue sounding into the microphone is silenced, keeping the
    /// recording's timeline, when nobody talks into or right after it.
    #[tokio::test]
    async fn the_start_cue_is_silenced_when_nobody_talks_over_it() {
        let heard = record_cue(ROOM, ROOM).await;
        let span = 3_200 + samples_in(CUE_LATENCY) as usize;
        assert!(heard[..AHEAD].iter().all(|&sample| sample == ROOM));
        assert!(
            heard[AHEAD..AHEAD + span]
                .iter()
                .all(|&sample| sample == 0.0)
        );
        assert!(heard[AHEAD + span..].iter().all(|&sample| sample == ROOM));
        assert!(heard.len() >= AHEAD + span + samples_in(CUE_LOOKAHEAD) as usize);
    }

    /// Words said into the cue or just after it are kept as heard, cue and
    /// all.
    #[tokio::test]
    async fn speech_over_the_start_cue_is_kept() {
        for (before, after) in [(SPEECH, ROOM), (ROOM, SPEECH)] {
            let heard = record_cue(before, after).await;
            assert!(
                heard[AHEAD..AHEAD + 3_200]
                    .iter()
                    .all(|&sample| sample == CUE)
            );
            assert!(!heard.contains(&0.0));
        }
    }

    /// A dictation released while its cue is held gets it at once, so it
    /// counts as said before the release and not as release tail.
    #[tokio::test]
    async fn a_release_sends_the_held_cue_at_once() {
        let mut ring = ring();
        push_level(&mut ring, 16_000 - 3_200, ROOM);
        push_level(&mut ring, 3_200, CUE);
        let (mut samples, control) = listen(&mut ring, cue_window());
        assert_eq!(heard(&mut samples).await.0.len(), AHEAD);
        ring.release(&control);
        assert_eq!(heard(&mut samples).await.0.len(), 3_200);
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
