//! The narration: the microphone for the whole capture, written to
//! `audio.wav` (16 kHz mono 16-bit) so that its sample 0 is the capture's
//! start.
//!
//! The microphone opens just before the screen recording starts, and its
//! frames carry no timestamps. Each frame's arrival on the host clock bounds
//! when its first sample was heard: `arrival − samples so far / rate` is never
//! earlier than the true start of sample 0, and is closest to it for the frame
//! delivered with the least delay. [`Aligner`] keeps the lowest such bound
//! over the first second of frames, then pads with silence or drops samples so
//! that the file starts at the capture's `started_host_ns`. What it cannot see
//! is the input latency before a frame reaches the process (the device's and
//! the resampler's), which makes the speech land that much late, typically
//! 10–30 ms, plus the drift between the microphone's clock and the host's over
//! a long recording (tens of ms over 20 minutes).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::Error;

pub const SAMPLE_RATE: u32 = 16_000;
const CHUNK_SIZE: usize = 320;
/// The native layer stops at 20 minutes from the capture's start; this only
/// bounds a microphone left open by a host that lost track. Counted from the
/// start once it is known, from opening the microphone until then.
const MAX_SECONDS: u64 = 20 * 60 + 30;
/// Frames observed before the start bound is trusted: a second of 20 ms frames.
const OBSERVATIONS: usize = 50;

/// The host clock: `mach_absolute_time` in nanoseconds on macOS, the clock of
/// the native layer's `started_host_ns`.
pub trait Clock: Send + Sync + 'static {
    fn now_ns(&self) -> u64;
}

pub struct HostClock;

impl Clock for HostClock {
    #[cfg(target_os = "macos")]
    fn now_ns(&self) -> u64 {
        let mut now = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // CLOCK_UPTIME_RAW is mach_absolute_time in nanoseconds.
        unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut now) };
        (now.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(now.tv_nsec as u64)
    }

    #[cfg(not(target_os = "macos"))]
    fn now_ns(&self) -> u64 {
        use std::sync::OnceLock;
        static EPOCH: OnceLock<std::time::Instant> = OnceLock::new();
        EPOCH
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_nanos() as u64
    }
}

/// How many samples to put before or take off the recording's head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Pad(u64),
    Trim(u64),
}

/// The shift that makes a recording whose sample 0 was heard at `first_ns`
/// start at `start_ns`.
pub fn shift(first_ns: u64, start_ns: u64, rate: u32) -> Shift {
    let samples = |ns: u64| (u128::from(ns) * u128::from(rate) / 1_000_000_000) as u64;
    if first_ns <= start_ns {
        Shift::Trim(samples(start_ns - first_ns))
    } else {
        Shift::Pad(samples(first_ns - start_ns))
    }
}

/// Turns the microphone's frames into samples aligned to the capture's
/// start. Pure: the caller passes each frame's arrival time.
pub struct Aligner {
    rate: u32,
    received: u64,
    observations: usize,
    first_ns: Option<u64>,
    start_ns: Option<u64>,
    state: State,
    /// The silence put before the first sample, once aligned.
    padded: Option<u64>,
}

enum State {
    /// Until the start and the bound are known.
    Buffering(Vec<f32>),
    /// Dropping the head that came before the start.
    Skipping(u64),
    Passing,
}

impl Aligner {
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            received: 0,
            observations: 0,
            first_ns: None,
            start_ns: None,
            state: State::Buffering(Vec::new()),
            padded: None,
        }
    }

    /// The silence put before the first sample heard, once it is aligned.
    pub fn padded(&self) -> Option<u64> {
        self.padded
    }

    /// Whether any frame arrived.
    pub fn heard(&self) -> bool {
        self.received > 0
    }

    /// The capture started at `ns` on the host clock.
    pub fn set_start(&mut self, ns: u64) {
        self.start_ns.get_or_insert(ns);
    }

    /// A frame arrived at `arrival_ns`; returns the samples ready to write.
    pub fn push(&mut self, arrival_ns: u64, samples: &[f32]) -> Vec<f32> {
        self.received += samples.len() as u64;
        if self.observations < OBSERVATIONS {
            let heard = u128::from(self.received) * 1_000_000_000 / u128::from(self.rate);
            let bound = u64::try_from(u128::from(arrival_ns).saturating_sub(heard)).unwrap_or(0);
            self.first_ns = Some(self.first_ns.map_or(bound, |first| first.min(bound)));
            self.observations += 1;
        }
        match &mut self.state {
            State::Buffering(buffer) => {
                buffer.extend_from_slice(samples);
                if self.observations >= OBSERVATIONS && self.start_ns.is_some() {
                    self.align()
                } else {
                    Vec::new()
                }
            }
            State::Skipping(_) | State::Passing => self.pass(samples),
        }
    }

    /// The end of the recording: what is still held, aligned as well as the
    /// frames so far allow. `None` when the capture never started.
    pub fn finish(&mut self) -> Option<Vec<f32>> {
        self.start_ns?;
        Some(match self.state {
            State::Buffering(_) => self.align(),
            _ => Vec::new(),
        })
    }

    fn align(&mut self) -> Vec<f32> {
        let (Some(start), State::Buffering(buffer)) = (
            self.start_ns,
            std::mem::replace(&mut self.state, State::Passing),
        ) else {
            return Vec::new();
        };
        let first = self.first_ns.unwrap_or(start);
        let shifted = shift(first, start, self.rate);
        self.padded = Some(match shifted {
            Shift::Pad(n) => n,
            Shift::Trim(_) => 0,
        });
        match shifted {
            Shift::Pad(n) => {
                let mut out = vec![0.0; n as usize];
                out.extend_from_slice(&buffer);
                out
            }
            Shift::Trim(n) => {
                self.state = State::Skipping(n);
                self.pass(&buffer)
            }
        }
    }

    fn pass(&mut self, samples: &[f32]) -> Vec<f32> {
        if let State::Skipping(left) = &mut self.state {
            let skip = (*left).min(samples.len() as u64);
            *left -= skip;
            if *left == 0 {
                self.state = State::Passing;
            }
            return samples[skip as usize..].to_vec();
        }
        samples.to_vec()
    }
}

/// The microphone the narration opens, chosen afresh each time it opens: a
/// Bluetooth default gives way to the built-in or wired microphone unless
/// one was chosen, as dictation's does.
pub struct Microphone {
    pub audio: Arc<dyn anlg_audio::AudioProvider>,
    /// The microphone the person chose, or the one a meeting records.
    pub explicit: Option<String>,
    pub avoid_bluetooth: bool,
    /// The Bluetooth default and its wired replacement, by name.
    pub stand_in: fn() -> Option<(String, String)>,
}

impl Microphone {
    pub fn new(
        audio: Arc<dyn anlg_audio::AudioProvider>,
        explicit: Option<String>,
        avoid_bluetooth: bool,
    ) -> Self {
        Self {
            audio,
            explicit,
            avoid_bluetooth,
            stand_in: anlg_audio_device::bluetooth_default_names,
        }
    }

    /// Blocks while the device opens.
    fn open(&self) -> Result<anlg_audio::CaptureStream, Error> {
        let choice = anlg_audio_device::choose_input(
            self.explicit.clone(),
            self.avoid_bluetooth,
            self.stand_in,
            || self.audio.list_mic_devices(),
        );
        self.audio
            .open_mic_capture(choice.device, SAMPLE_RATE, CHUNK_SIZE)
            .map_err(|error| Error::Microphone(error.to_string()))
    }
}

/// What the narration tells the capsule while it records.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Heard {
    /// The last 40 ms on a meter, 0 to 1.
    Level(f32),
    /// The microphone stopped: nothing is recorded until it is back.
    Lost,
    /// It opened again after stopping or failing to open.
    Back,
}

pub type Hear = Arc<dyn Fn(Heard) + Send + Sync>;

/// How a microphone that stopped or failed to open is opened again: after
/// `first`, doubling up to `most`, at most `tries` times in a row. One that
/// then hears for a second gets every try again when it next stops.
#[derive(Clone, Copy, Debug)]
pub struct Reopen {
    pub first: Duration,
    pub most: Duration,
    pub tries: u32,
}

impl Default for Reopen {
    /// About 20 s of tries, for a headset reconnecting or a cable put back.
    fn default() -> Self {
        Self {
            first: Duration::from_millis(250),
            most: Duration::from_secs(2),
            tries: 12,
        }
    }
}

impl Reopen {
    fn delay(&self, tried: u32) -> Duration {
        self.first.saturating_mul(1 << tried.min(16)).min(self.most)
    }
}

/// 25 levels a second.
const LEVEL_SAMPLES: usize = SAMPLE_RATE as usize / 25;
/// A reopened microphone heard this long counts as working again.
const STEADY_SAMPLES: u64 = SAMPLE_RATE as u64;

/// Part of the narration the microphone did not hear: silence in the file,
/// in ms from the capture's start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gap {
    pub from_ms: u64,
    /// `None` when it never came back.
    pub to_ms: Option<u64>,
}

/// What a finished narration left.
#[derive(Clone, Debug)]
pub struct Narrated {
    pub path: PathBuf,
    /// Samples in the file, silence padding included.
    pub samples: u32,
    pub gaps: Vec<Gap>,
}

/// The microphone recording to `audio.wav` while a capture runs. A
/// microphone that stops or fails to open is opened again, the time it was
/// away kept as silence so that the narration stays on the screen's
/// timeline; what it heard before is always kept.
pub struct Narration {
    cancellation: CancellationToken,
    start: watch::Sender<Option<u64>>,
    keep: Arc<std::sync::atomic::AtomicBool>,
    task: tauri::async_runtime::JoinHandle<Result<Narrated, Error>>,
    hearing: Arc<std::sync::atomic::AtomicBool>,
    /// Whether the microphone opened at the start.
    pub opened: bool,
}

impl Narration {
    /// Opens the microphone, blocking while it does. One that fails to open
    /// is tried again in the background.
    pub fn start(
        microphone: Microphone,
        path: PathBuf,
        clock: Arc<dyn Clock>,
        hear: Hear,
        reopen: Reopen,
    ) -> Self {
        let first = microphone
            .open()
            .inspect_err(|error| tracing::warn!(%error, "annotate microphone did not open"))
            .ok();
        let opened = first.is_some();
        let cancellation = CancellationToken::new();
        let (start, starts) = watch::channel(None);
        let keep = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hearing = Arc::new(std::sync::atomic::AtomicBool::new(opened));
        let task = tauri::async_runtime::spawn({
            let cancellation = cancellation.clone();
            let keep = keep.clone();
            let mut recording = Recording {
                microphone: Arc::new(microphone),
                path,
                cancellation: cancellation.clone(),
                start: starts,
                clock,
                hear,
                hearing: hearing.clone(),
                reopen,
                writer: None,
                gaps: Vec::new(),
                gap_from: (!opened).then_some(0),
            };
            async move {
                let result = recording.run(first).await;
                // A microphone that gave up, or the deadline, leaves the
                // file finished; only `stop` or `cancel` decides whether it
                // stays.
                cancellation.cancelled().await;
                let keep = keep.load(std::sync::atomic::Ordering::SeqCst);
                if result.is_err() || !keep {
                    let _ = std::fs::remove_file(&recording.path);
                }
                result.and_then(|narrated| {
                    if keep {
                        Ok(narrated)
                    } else {
                        Err(Error::Cancelled)
                    }
                })
            }
        });
        Self {
            cancellation,
            start,
            keep,
            task,
            hearing,
            opened,
        }
    }

    /// Whether the microphone records now.
    pub fn hearing(&self) -> bool {
        self.hearing.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The capture started at `ns` on the host clock: sample 0 of the file.
    pub fn begin_at(&self, ns: u64) {
        self.start.send_replace(Some(ns));
    }

    pub async fn stop(self) -> Result<Narrated, Error> {
        self.keep.store(true, std::sync::atomic::Ordering::SeqCst);
        self.cancellation.cancel();
        self.task
            .await
            .map_err(|error| Error::Microphone(error.to_string()))?
    }

    /// Stops and deletes the file.
    pub async fn cancel(self) {
        self.cancellation.cancel();
        let _ = self.task.await;
    }
}

type Writer = hound::WavWriter<std::io::BufWriter<std::fs::File>>;

/// How one stream of the microphone ended.
enum Ended {
    /// Stopped, cancelled, or the deadline.
    Done,
    /// It failed or ended by itself.
    Lost,
}

struct Recording {
    microphone: Arc<Microphone>,
    path: PathBuf,
    cancellation: CancellationToken,
    start: watch::Receiver<Option<u64>>,
    clock: Arc<dyn Clock>,
    hear: Hear,
    hearing: Arc<std::sync::atomic::AtomicBool>,
    reopen: Reopen,
    /// Made once the microphone opens, so one never heard leaves no file.
    writer: Option<Writer>,
    gaps: Vec<Gap>,
    /// Where the gap still open began, in samples.
    gap_from: Option<u64>,
}

impl Recording {
    fn written(&self) -> u64 {
        self.writer
            .as_ref()
            .map_or(0, |writer| u64::from(writer.len()))
    }

    fn start_ns(&self) -> Option<u64> {
        *self.start.borrow()
    }

    fn writer(&mut self) -> Result<&mut Writer, Error> {
        if self.writer.is_none() {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            self.writer = Some(hound::WavWriter::create(&self.path, spec).map_err(wav)?);
        }
        self.writer
            .as_mut()
            .ok_or_else(|| Error::Microphone("no narration file".into()))
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), Error> {
        if samples.is_empty() {
            return Ok(());
        }
        write(self.writer()?, samples)
    }

    /// Waits for the capture's start; false when it was cancelled first.
    async fn started(&mut self) -> bool {
        if self.start_ns().is_some() {
            return true;
        }
        tokio::select! {
            _ = self.cancellation.cancelled() => {}
            _ = self.start.wait_for(Option::is_some) => {}
        }
        self.start_ns().is_some()
    }

    async fn run(&mut self, first: Option<anlg_audio::CaptureStream>) -> Result<Narrated, Error> {
        let max = Duration::from_secs(MAX_SECONDS);
        let deadline = tokio::time::sleep(max);
        tokio::pin!(deadline);
        let mut deadline_from_start = false;
        if first.is_some() {
            self.writer()?;
        }
        let mut stream = first;
        let mut tried = 0;
        let mut lost = stream.is_none();
        let mut level = Vec::with_capacity(LEVEL_SAMPLES);
        loop {
            if let Some(mut current) = stream.take() {
                // This stream's sample 0 lands after what is written.
                let base = self.written();
                let mut aligner = Aligner::new(SAMPLE_RATE);
                let mut heard = 0;
                let ended = loop {
                    let next = tokio::select! {
                        _ = self.cancellation.cancelled() => break Ended::Done,
                        _ = &mut deadline => break Ended::Done,
                        next = current.next() => next,
                    };
                    let frame = match next {
                        Some(Ok(frame)) => frame,
                        Some(Err(error)) => {
                            tracing::warn!(%error, "annotate microphone stream failed");
                            break Ended::Lost;
                        }
                        None => {
                            tracing::warn!("annotate microphone stream ended");
                            break Ended::Lost;
                        }
                    };
                    let arrival = self.clock.now_ns();
                    if let Some(ns) = self.start_ns() {
                        aligner.set_start(at(ns, base));
                        if !deadline_from_start {
                            deadline_from_start = true;
                            deadline.as_mut().reset(tokio::time::Instant::now() + max);
                        }
                    }
                    if lost {
                        lost = false;
                        self.hearing
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        (self.hear)(Heard::Back);
                    }
                    let samples = frame.preferred_mic();
                    heard += samples.len() as u64;
                    if heard >= STEADY_SAMPLES {
                        tried = 0;
                    }
                    level.extend_from_slice(&samples);
                    if level.len() >= LEVEL_SAMPLES {
                        let (_, peak) = anlg_audio::level::measure(&level);
                        (self.hear)(Heard::Level(anlg_audio::level::meter(peak)));
                        level.clear();
                    }
                    let ready = aligner.push(arrival, &samples);
                    self.write(&ready)?;
                    self.close_gap(&aligner, base);
                };
                drop(current);
                level.clear();
                // What this stream still holds, aligned once the capture's
                // start is known.
                if self.started().await
                    && let Some(ns) = self.start_ns()
                {
                    aligner.set_start(at(ns, base));
                    if let Some(rest) = aligner.finish() {
                        self.write(&rest)?;
                    }
                    self.close_gap(&aligner, base);
                }
                if matches!(ended, Ended::Done) || self.cancellation.is_cancelled() {
                    break;
                }
                if !lost {
                    lost = true;
                    self.hearing
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    (self.hear)(Heard::Lost);
                }
                if self.gap_from.is_none() {
                    self.gap_from = Some(self.written());
                }
            }
            if tried >= self.reopen.tries {
                tracing::warn!("annotate microphone gave up");
                break;
            }
            let delay = self.reopen.delay(tried);
            tried += 1;
            tokio::select! {
                _ = self.cancellation.cancelled() => break,
                _ = &mut deadline => break,
                _ = tokio::time::sleep(delay) => {}
            }
            // A stream opened before the capture's start could not be
            // placed after what the last one heard.
            if !self.started().await {
                break;
            }
            let microphone = self.microphone.clone();
            match tauri::async_runtime::spawn_blocking(move || microphone.open()).await {
                Ok(Ok(opened)) => stream = Some(opened),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "annotate microphone did not open again");
                }
                Err(error) => tracing::warn!(%error, "annotate microphone did not open again"),
            }
        }
        if let Some(from) = self.gap_from.take() {
            self.gaps.push(Gap {
                from_ms: ms(from),
                to_ms: None,
            });
        }
        // The capture never started: nothing to align to.
        if !self.started().await {
            return Err(Error::Cancelled);
        }
        let Some(writer) = self.writer.take() else {
            return Err(Error::Microphone("the microphone heard nothing".into()));
        };
        let samples = writer.len();
        writer.finalize().map_err(wav)?;
        Ok(Narrated {
            path: self.path.clone(),
            samples,
            gaps: std::mem::take(&mut self.gaps),
        })
    }

    /// The open gap ends where a stream that heard something was placed.
    fn close_gap(&mut self, aligner: &Aligner, base: u64) {
        if !aligner.heard() {
            return;
        }
        let (Some(from), Some(padded)) = (self.gap_from, aligner.padded()) else {
            return;
        };
        self.gap_from = None;
        self.gaps.push(Gap {
            from_ms: ms(from),
            to_ms: Some(ms(base + padded)),
        });
    }
}

/// The host time of sample `sample` of a file whose sample 0 is `start_ns`.
fn at(start_ns: u64, sample: u64) -> u64 {
    start_ns.saturating_add(sample.saturating_mul(1_000_000_000) / u64::from(SAMPLE_RATE))
}

fn ms(samples: u64) -> u64 {
    samples * 1_000 / u64::from(SAMPLE_RATE)
}

fn wav(error: hound::Error) -> Error {
    Error::Microphone(error.to_string())
}

fn write<W: std::io::Write + std::io::Seek>(
    writer: &mut hound::WavWriter<W>,
    samples: &[f32],
) -> Result<(), Error> {
    for sample in samples {
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
            .map_err(wav)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 20 ms frames of `value`, the first `jitter` late by the given ns.
    fn feed(aligner: &mut Aligner, heard_at: u64, frames: usize, late: &[u64]) -> Vec<f32> {
        let mut out = Vec::new();
        for i in 0..frames {
            let ends = heard_at + (i as u64 + 1) * 20_000_000;
            let delay = late.get(i).copied().unwrap_or(3_000_000);
            out.extend(aligner.push(ends + delay, &[i as f32; CHUNK_SIZE]));
        }
        out
    }

    #[test]
    fn audio_heard_before_the_capture_started_is_trimmed_to_its_start() {
        // Sample 0 heard at 1.000 s; the capture starts at 1.250 s.
        let mut aligner = Aligner::new(SAMPLE_RATE);
        aligner.set_start(1_250_000_000);
        let mut out = feed(
            &mut aligner,
            1_000_000_000,
            60,
            &[9_000_000, 2_000_000, 7_000_000],
        );
        out.extend(aligner.finish().unwrap());
        // 250 ms = 4,000 samples dropped, give or take the 2 ms best delivery.
        let dropped = 60 * CHUNK_SIZE - out.len();
        assert!((3_960..=4_000).contains(&dropped), "dropped {dropped}");
        // What remains starts inside frame 12 (240–260 ms).
        assert_eq!(out[0], 12.0);
    }

    #[test]
    fn audio_that_began_after_the_capture_is_padded_with_silence() {
        // The capture started at 1.000 s; the microphone's sample 0 at 1.100 s.
        let mut aligner = Aligner::new(SAMPLE_RATE);
        aligner.set_start(1_000_000_000);
        let mut out = feed(&mut aligner, 1_100_000_000, 60, &[]);
        out.extend(aligner.finish().unwrap());
        let silence = out.iter().take_while(|s| **s == 0.0).count() - CHUNK_SIZE;
        // 100 ms is 1,600 samples, plus the 3 ms delivery delay counted as late.
        assert!((1_600..=1_650).contains(&silence), "silence {silence}");
        assert_eq!(out.len() - silence, 60 * CHUNK_SIZE);
    }

    #[test]
    fn a_short_recording_still_aligns_and_a_capture_that_never_started_writes_nothing() {
        let mut aligner = Aligner::new(SAMPLE_RATE);
        assert!(feed(&mut aligner, 0, 5, &[]).is_empty());
        aligner.set_start(0);
        let out = aligner.finish().unwrap();
        assert_eq!(out.len(), 5 * CHUNK_SIZE + 48);

        let mut never = Aligner::new(SAMPLE_RATE);
        feed(&mut never, 0, 5, &[]);
        assert!(never.finish().is_none());
    }

    #[test]
    fn the_shift_is_counted_in_samples() {
        assert_eq!(
            shift(1_000_000_000, 1_500_000_000, 16_000),
            Shift::Trim(8_000)
        );
        assert_eq!(
            shift(1_500_000_000, 1_000_000_000, 16_000),
            Shift::Pad(8_000)
        );
        assert_eq!(shift(5, 5, 16_000), Shift::Trim(0));
    }

    /// A microphone that, on each opening in turn, fails to open (`None`) or
    /// delivers that many 20 ms frames and then ends; past the script it no
    /// longer opens. It keeps the device each opening asked for.
    struct Script {
        opens: std::sync::Mutex<std::collections::VecDeque<Option<usize>>>,
        devices: std::sync::Mutex<Vec<Option<String>>>,
        listed: Vec<String>,
    }

    impl Script {
        fn new(opens: &[Option<usize>]) -> Arc<Self> {
            Arc::new(Self {
                opens: std::sync::Mutex::new(opens.iter().copied().collect()),
                devices: std::sync::Mutex::default(),
                listed: vec!["AirPods".into(), "MacBook Pro Microphone".into()],
            })
        }
    }

    /// A tone of ±0.1, so the meter reads it.
    const TONE: f32 = 0.1;

    impl anlg_audio::AudioProvider for Script {
        fn open_capture(
            &self,
            _: anlg_audio::CaptureConfig,
        ) -> Result<anlg_audio::CaptureStream, anlg_audio::Error> {
            unreachable!()
        }
        fn open_speaker_capture(
            &self,
            _: u32,
            _: usize,
        ) -> Result<anlg_audio::CaptureStream, anlg_audio::Error> {
            unreachable!()
        }
        fn open_mic_capture(
            &self,
            device: Option<String>,
            _: u32,
            chunk: usize,
        ) -> Result<anlg_audio::CaptureStream, anlg_audio::Error> {
            self.devices.lock().unwrap().push(device);
            let Some(Some(frames)) = self.opens.lock().unwrap().pop_front() else {
                return Err(anlg_audio::Error::NoInputDevice);
            };
            let tone: Vec<f32> = (0..chunk)
                .map(|i| if i % 2 == 0 { TONE } else { -TONE })
                .collect();
            let frame = anlg_audio::CaptureFrame {
                raw_mic: Arc::from(tone),
                raw_speaker: Arc::from([]),
                aec_mic: None,
            };
            Ok(anlg_audio::CaptureStream::new(
                futures_util::stream::unfold(frames, move |left| {
                    let frame = frame.clone();
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        (left > 0).then(|| (Ok(frame), left - 1))
                    }
                }),
            ))
        }
        fn default_device_name(&self) -> String {
            "mic".into()
        }
        fn list_mic_devices(&self) -> Vec<String> {
            self.listed.clone()
        }
        fn play_silence(&self) -> std::sync::mpsc::Sender<()> {
            std::sync::mpsc::channel().0
        }
        fn play_bytes(&self, _: &'static [u8]) -> std::sync::mpsc::Sender<()> {
            std::sync::mpsc::channel().0
        }
        fn probe_mic(&self, _: Option<String>) -> Result<(), anlg_audio::Error> {
            Ok(())
        }
        fn probe_speaker(&self) -> Result<(), anlg_audio::Error> {
            Ok(())
        }
    }

    fn airpods() -> Option<(String, String)> {
        Some(("AirPods".into(), "MacBook Pro Microphone".into()))
    }

    fn microphone(script: &Arc<Script>) -> Microphone {
        Microphone {
            stand_in: airpods,
            ..Microphone::new(script.clone(), None, true)
        }
    }

    /// What the narration told the capsule.
    fn ears() -> (Hear, Arc<std::sync::Mutex<Vec<Heard>>>) {
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = heard.clone();
        (
            Arc::new(move |event| sink.lock().unwrap().push(event)),
            heard,
        )
    }

    fn quick(tries: u32) -> Reopen {
        Reopen {
            first: Duration::from_millis(100),
            most: Duration::from_millis(100),
            tries,
        }
    }

    #[test]
    fn the_narration_records_from_the_wired_microphone_while_the_default_is_a_bluetooth_headset() {
        let script = Script::new(&[Some(0), Some(0), Some(0)]);
        microphone(&script).open().unwrap();
        Microphone {
            explicit: Some("AirPods".into()),
            ..microphone(&script)
        }
        .open()
        .unwrap();
        Microphone {
            avoid_bluetooth: false,
            ..microphone(&script)
        }
        .open()
        .unwrap();
        assert_eq!(
            *script.devices.lock().unwrap(),
            [
                Some("MacBook Pro Microphone".to_string()),
                Some("AirPods".to_string()),
                None
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_microphone_that_ends_mid_recording_keeps_what_it_heard_for_stop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.wav");
        let started = HostClock.now_ns();
        let script = Script::new(&[Some(10)]);
        let (hear, heard) = ears();
        let narration = Narration::start(
            microphone(&script),
            path.clone(),
            Arc::new(HostClock),
            hear,
            quick(2),
        );
        assert!(narration.opened);
        narration.begin_at(started);
        // The ten frames, the end of the stream and both tries to open it
        // again come well before the stop.
        tokio::time::sleep(Duration::from_millis(700)).await;
        let narrated = narration.stop().await.unwrap();
        let wav = hound::WavReader::open(&narrated.path).unwrap();
        assert!(
            wav.duration() >= 10 * CHUNK_SIZE as u32 / 2,
            "{}",
            wav.duration()
        );
        assert_eq!(wav.duration(), narrated.samples);
        assert_eq!(narrated.gaps.len(), 1);
        assert!(narrated.gaps[0].to_ms.is_none());
        assert!(heard.lock().unwrap().contains(&Heard::Lost));
        assert_eq!(script.devices.lock().unwrap().len(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_microphone_that_comes_back_leaves_silence_for_the_time_it_was_away() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.wav");
        let started = HostClock.now_ns();
        // 200 ms heard, a failed try, then 1.2 s heard again.
        let script = Script::new(&[Some(10), None, Some(60)]);
        let (hear, heard) = ears();
        let narration = Narration::start(
            microphone(&script),
            path.clone(),
            Arc::new(HostClock),
            hear,
            quick(5),
        );
        narration.begin_at(started);
        tokio::time::sleep(Duration::from_millis(2_200)).await;
        let narrated = narration.stop().await.unwrap();

        let samples: Vec<i16> = hound::WavReader::open(&narrated.path)
            .unwrap()
            .into_samples()
            .map(Result::unwrap)
            .collect();
        let heard_at = |sample: &i16| *sample != 0;
        let first = samples.iter().position(heard_at).unwrap();
        let until = first + samples[first..].iter().position(|s| !heard_at(s)).unwrap();
        let back = until + samples[until..].iter().position(heard_at).unwrap();
        let silence = back - until;
        // Two tries 100 ms apart: at least 200 ms away, at most a little more.
        assert!((3_200..=8_000).contains(&silence), "silence {silence}");
        // Both stretches are kept whole.
        assert!((until - first) >= 9 * CHUNK_SIZE, "{}", until - first);
        assert!(
            samples.len() - back >= 59 * CHUNK_SIZE,
            "{}",
            samples.len() - back
        );

        // It stopped again at the end, past every try.
        let [gap, end] = narrated.gaps[..] else {
            panic!("{:?}", narrated.gaps);
        };
        assert!(end.to_ms.is_none());
        let to_ms = gap.to_ms.unwrap();
        assert_eq!(gap.from_ms, ms(until as u64));
        assert!(to_ms.abs_diff(ms(back as u64)) <= 25, "{gap:?} {back}");

        let heard = heard.lock().unwrap();
        let lost = heard.iter().position(|h| *h == Heard::Lost).unwrap();
        let again = heard.iter().position(|h| *h == Heard::Back).unwrap();
        assert!(lost < again);
        assert!(
            heard
                .iter()
                .any(|h| matches!(h, Heard::Level(level) if *level > 0.5))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_microphone_that_opens_late_is_placed_on_the_capture_timeline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.wav");
        let script = Script::new(&[None, Some(60)]);
        let (hear, heard) = ears();
        let narration = Narration::start(
            microphone(&script),
            path.clone(),
            Arc::new(HostClock),
            hear,
            quick(3),
        );
        assert!(!narration.opened);
        narration.begin_at(HostClock.now_ns());
        tokio::time::sleep(Duration::from_millis(1_600)).await;
        let narrated = narration.stop().await.unwrap();
        let samples: Vec<i16> = hound::WavReader::open(&narrated.path)
            .unwrap()
            .into_samples()
            .map(Result::unwrap)
            .collect();
        let silence = samples.iter().position(|s| *s != 0).unwrap();
        // The try 100 ms in opened it.
        assert!((1_600..=4_000).contains(&silence), "silence {silence}");
        assert_eq!(narrated.gaps[0].from_ms, 0);
        assert_eq!(*heard.lock().unwrap().first().unwrap(), Heard::Back);
    }
}
