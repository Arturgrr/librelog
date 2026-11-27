use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use futures_util::{FutureExt, StreamExt};
use tauri::async_runtime::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::Error;
use crate::environment::{Quiet, RecordingEnvironment};
use crate::level::{LevelMonitor, LevelSource, LevelUpdate};
use crate::preview::{Preview, PreviewConfig, RecordingUpdate};
use crate::tail::{MAX_RELEASE_TAIL, RELEASE_TAIL, ReleaseTail, TAIL_DELIVERY_GRACE, Voice};
use crate::warm::{Reading, Samples, Warm, WarmRequest, Window};
use tauri::ipc::Channel;

pub(crate) const SAMPLE_RATE: u32 = 16_000;
/// 20 ms buffers, so the HUD meters the voice as often as Hex does.
const CHUNK_SIZE: usize = 320;
/// The live preview streams 100 ms packets, as it always has.
const PREVIEW_CHUNK_SIZE: usize = 1_600;
/// Only a safety net for a capture nobody stops: the WAV streams to disk at
/// 64 KB/s (about 460 MB in two hours) and nothing grows in memory, and long
/// clips are transcribed in chunks.
const MAX_RECORDING: std::time::Duration = std::time::Duration::from_secs(2 * 60 * 60);

/// Receives the RMS of each captured buffer.
pub type Meter = Arc<dyn Fn(f32) + Send + Sync>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecordedAudio {
    pub file_path: String,
    pub duration_ms: u64,
    pub transcript: Option<String>,
}

/// What a capture reports while it records, and what it asks of the
/// computer's other audio.
#[derive(Default)]
pub struct Feedback {
    pub preview: Option<PreviewConfig>,
    pub updates: Option<Channel<RecordingUpdate>>,
    pub meter: Option<Meter>,
    pub environment: Option<RecordingEnvironment>,
    /// When the shortcut that started this capture went down. From the open
    /// microphone, the recording then starts a pre-roll before it; without
    /// one it starts now.
    pub pressed_at: Option<std::time::SystemTime>,
    /// When the start cue began and stopped playing; the open microphone
    /// silences it unless someone talks over it.
    pub start_cue: Option<(std::time::SystemTime, std::time::SystemTime)>,
}

struct ActiveRecording {
    owner: String,
    cancellation: CancellationToken,
    /// Ends the recording after its release tail.
    release: CancellationToken,
    task: JoinHandle<Result<RecordedAudio, Error>>,
    finalize: Arc<AtomicBool>,
    /// Its hold on the open microphone, when it reads it.
    reading: Option<Reading>,
}

pub struct Recorder {
    active: Mutex<Option<ActiveRecording>>,
    completed: Mutex<HashSet<PathBuf>>,
    warm: Warm,
    level: Mutex<Option<LevelMonitor>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
            completed: Mutex::new(HashSet::new()),
            warm: Warm::default(),
            level: Mutex::new(None),
        }
    }

    /// Keeps the chosen microphone open between dictations, or closes it.
    /// A change waits for a recording reading the open microphone to end.
    pub fn set_warm_microphone(
        &self,
        audio: Arc<dyn anlg_audio::AudioProvider>,
        microphone_device: Option<String>,
        avoid_bluetooth: bool,
        enabled: bool,
    ) -> Result<(), Error> {
        self.warm.set(enabled.then_some(WarmRequest {
            audio,
            device: microphone_device,
            avoid_bluetooth,
            chunk_size: CHUNK_SIZE,
        }))
    }

    /// The open microphone's audio through `window`, when it is open on
    /// `microphone_device`; otherwise the microphone opened now.
    fn open_source(
        &self,
        audio: &dyn anlg_audio::AudioProvider,
        microphone_device: Option<String>,
        window: &Window,
    ) -> Result<(Samples, Option<Reading>), Error> {
        if let Some((samples, reading)) = self.warm.listen(audio, &microphone_device, window) {
            return Ok((samples, Some(reading)));
        }
        Ok((open_cold(audio, microphone_device)?, None))
    }

    /// Measures `microphone_device` for the Settings level meter until
    /// [`Self::stop_level`], `max`, or a dictation that needs its stream.
    /// Reads the open microphone when it is open on that device, and opens
    /// nothing while a dictation records. Opening a microphone blocks, so
    /// call it off the async runtime.
    pub fn monitor_level(
        &self,
        audio: &dyn anlg_audio::AudioProvider,
        microphone_device: Option<String>,
        emit: impl Fn(LevelUpdate) -> bool + Send + 'static,
        max: std::time::Duration,
    ) -> Result<LevelSource, Error> {
        self.stop_level();
        {
            // Held until the meter is in place, so a dictation starting
            // meanwhile still finds and closes it.
            let active = self
                .active
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if active.is_some() {
                emit(LevelUpdate::Ended);
                return Ok(LevelSource::InUse);
            }
            if let Some(stream) = self.warm.meter(&microphone_device) {
                self.set_level(LevelMonitor::start(stream, false, emit, max));
                return Ok(LevelSource::OpenMicrophone);
            }
        }
        // Opened without the lock, so a dictation never waits for it; one
        // that started meanwhile keeps the microphone.
        let stream = open_cold(audio, microphone_device)?;
        let active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if active.is_some() {
            drop(stream);
            emit(LevelUpdate::Ended);
            return Ok(LevelSource::InUse);
        }
        self.set_level(LevelMonitor::start(stream, true, emit, max));
        Ok(LevelSource::Microphone)
    }

    fn set_level(&self, monitor: LevelMonitor) {
        *self.level.lock().unwrap_or_else(|error| error.into_inner()) = Some(monitor);
    }

    pub fn stop_level(&self) {
        self.level
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }

    pub fn start(
        &self,
        audio: Arc<dyn anlg_audio::AudioProvider>,
        microphone_device: Option<String>,
        owner: String,
    ) -> Result<(), Error> {
        self.start_with_feedback(audio, microphone_device, owner, Feedback::default())
    }

    pub fn start_with_feedback(
        &self,
        audio: Arc<dyn anlg_audio::AudioProvider>,
        microphone_device: Option<String>,
        owner: String,
        Feedback {
            preview,
            updates,
            meter,
            environment,
            pressed_at,
            start_cue,
        }: Feedback,
    ) -> Result<(), Error> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if active.is_some() {
            return Err(Error::AlreadyRecording);
        }
        // A meter on its own stream gives the microphone back to dictation.
        {
            let mut level = self.level.lock().unwrap_or_else(|error| error.into_inner());
            if level.as_ref().is_some_and(|level| level.owns_stream) {
                *level = None;
            }
        }

        let lookback = pressed_at.map_or(std::time::Duration::ZERO, |pressed_at| {
            crate::warm::lookback(
                pressed_at.elapsed().unwrap_or_default(),
                environment
                    .as_ref()
                    .map(|environment| &environment.config.behavior),
            )
        });
        let window = Window {
            lookback,
            cue: start_cue.map(|(start, end)| Window::cue_between(start, end)),
        };
        let quiet = Quiet::new(environment, updates.clone());
        let (stream, reading) = self.open_source(audio.as_ref(), microphone_device, &window)?;
        let temporary_file = tempfile::Builder::new()
            .prefix("librelog-dictation-")
            .suffix(".wav")
            .tempfile()
            .map_err(|error| Error::Recording(error.to_string()))?;
        let (_, path) = temporary_file
            .keep()
            .map_err(|error| Error::Recording(error.error.to_string()))?;

        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let release = CancellationToken::new();
        let task_release = release.clone();
        let task_path = path.clone();
        let finalize = Arc::new(AtomicBool::new(false));
        let task_finalize = finalize.clone();
        let task_reading = reading.clone();
        let task = tauri::async_runtime::spawn(async move {
            let preview = preview
                .zip(updates)
                .map(|(config, updates)| Preview::start(config, updates));
            let result = record_to_file(
                stream,
                Stop {
                    cancel: task_cancellation,
                    release: task_release,
                    reading: task_reading,
                },
                &task_path,
                preview,
                meter,
                task_finalize,
                quiet,
            )
            .await;
            if result.is_err() {
                let _ = std::fs::remove_file(&task_path);
            }
            result
        });

        *active = Some(ActiveRecording {
            owner,
            cancellation,
            release,
            task,
            finalize,
            reading,
        });
        Ok(())
    }

    pub async fn stop(&self, owner: &str) -> Result<RecordedAudio, Error> {
        let active = self.take_active(owner, false)?.ok_or(Error::NotRecording)?;
        active.finalize.store(true, Ordering::SeqCst);
        active.release.cancel();
        let recorded = active
            .task
            .await
            .map_err(|error| Error::Task(error.to_string()))??;
        self.completed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(PathBuf::from(&recorded.file_path));
        Ok(recorded)
    }

    pub async fn cancel(&self, owner: &str) -> Result<(), Error> {
        let Some(active) = self.take_active(owner, true)? else {
            return Ok(());
        };

        active.cancellation.cancel();
        match active.task.await {
            Ok(Ok(recorded)) => {
                let _ = std::fs::remove_file(recorded.file_path);
            }
            Ok(Err(_)) => {}
            Err(error) => return Err(Error::Task(error.to_string())),
        }
        Ok(())
    }

    pub fn discard(&self, file_path: String) -> Result<(), Error> {
        let path = PathBuf::from(file_path);
        let removed = self
            .completed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&path);
        if !removed {
            return Err(Error::UnknownRecording);
        }

        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::Recording(error.to_string())),
        }
    }

    /// The recording `owner` runs; a `discard`ed one is thrown away before
    /// another can start, so that one may reuse its audio.
    fn take_active(&self, owner: &str, discard: bool) -> Result<Option<ActiveRecording>, Error> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if active
            .as_ref()
            .is_some_and(|recording| recording.owner != owner)
        {
            return Err(Error::AlreadyRecording);
        }
        if discard && let Some(reading) = active.as_ref().and_then(|active| active.reading.as_ref())
        {
            reading.discard();
        }
        Ok(active.take())
    }
}

fn open_cold(
    audio: &dyn anlg_audio::AudioProvider,
    microphone_device: Option<String>,
) -> Result<Samples, Error> {
    let stream = audio
        .open_mic_capture(microphone_device, SAMPLE_RATE, CHUNK_SIZE)
        .map_err(|error| Error::Capture(error.to_string()))?;
    Ok(stream
        .map(|frame| {
            frame
                .map(|frame| frame.preferred_mic())
                .map_err(|error| Error::Capture(error.to_string()))
        })
        .boxed())
}

/// How a recording ends: cancelled at once, or released, keeping its tail.
struct Stop {
    cancel: CancellationToken,
    release: CancellationToken,
    reading: Option<Reading>,
}

/// Where a recording's audio goes: the WAV, the HUD meter and the live
/// preview.
struct Output {
    writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    sample_count: u64,
    meter: Option<Meter>,
    preview: Option<Preview>,
    pending: Vec<f32>,
}

impl Output {
    fn write(&mut self, samples: &[f32]) -> Result<(), Error> {
        for sample in samples {
            self.writer
                .write_sample(*sample)
                .map_err(|error| Error::Recording(error.to_string()))?;
        }
        self.sample_count += samples.len() as u64;
        if let Some(meter) = &self.meter {
            meter(crate::events::rms(samples));
        }
        if self.preview.is_some() {
            self.pending.extend_from_slice(samples);
            if self.pending.len() >= PREVIEW_CHUNK_SIZE {
                if self
                    .preview
                    .as_ref()
                    .is_some_and(|preview| !preview.send(&self.pending))
                {
                    self.preview = None;
                }
                self.pending.clear();
            }
        }
        Ok(())
    }
}

async fn record_to_file(
    mut stream: Samples,
    stop: Stop,
    path: &Path,
    preview: Option<Preview>,
    meter: Option<Meter>,
    finalize: Arc<AtomicBool>,
    quiet: Quiet,
) -> Result<RecordedAudio, Error> {
    let specification = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let writer = hound::WavWriter::create(path, specification)
        .map_err(|error| Error::Recording(error.to_string()))?;
    let mut output = Output {
        writer,
        sample_count: 0,
        meter,
        preview,
        pending: Vec::with_capacity(PREVIEW_CHUNK_SIZE),
    };
    let deadline = tokio::time::sleep(MAX_RECORDING);
    tokio::pin!(deadline);
    // A microphone that stalls during the tail still ends the recording.
    let tail_deadline = tokio::time::sleep(MAX_RECORDING);
    tokio::pin!(tail_deadline);
    let mut released: Option<(tokio::time::Instant, ReleaseTail)> = None;
    let mut voice = Voice::default();

    loop {
        tokio::select! {
            _ = stop.cancel.cancelled() => break,
            _ = stop.release.cancelled(), if released.is_none() => {
                if let Some(reading) = &stop.reading {
                    reading.released();
                }
                // What was heard before the release and is still on its way
                // belongs to the recording, not to its tail.
                let mut ended = false;
                while let Some(next) = stream.next().now_or_never() {
                    let Some(samples) = next else {
                        ended = true;
                        break;
                    };
                    let samples = samples?;
                    voice.extend(&samples);
                    output.write(&samples)?;
                }
                if ended {
                    break;
                }
                let now = tokio::time::Instant::now();
                tail_deadline.as_mut().reset(now + RELEASE_TAIL + TAIL_DELIVERY_GRACE);
                released = Some((now, ReleaseTail::default()));
            }
            _ = &mut tail_deadline, if released.is_some() => break,
            _ = &mut deadline => break,
            next = stream.next() => {
                let Some(samples) = next else {
                    break;
                };
                let mut samples = samples?;
                let mut done = false;
                match &mut released {
                    None => {
                        voice.extend(&samples);
                    }
                    Some((at, tail)) => {
                        let extending = tail.extending();
                        let fed = tail.feed(&samples, &mut voice);
                        if !extending && tail.extending() {
                            tail_deadline
                                .as_mut()
                                .reset(*at + MAX_RELEASE_TAIL + TAIL_DELIVERY_GRACE);
                        }
                        if fed.keep < samples.len() {
                            samples = samples[..fed.keep].into();
                        }
                        done = fed.done;
                    }
                }
                output.write(&samples)?;
                if done {
                    break;
                }
            }
        }
    }
    // The live transcript can take seconds to finish; the next dictation may
    // open the microphone meanwhile, and other audio comes back now.
    drop(stream);
    drop(quiet);
    let Output {
        writer,
        sample_count,
        mut preview,
        pending,
        ..
    } = output;
    if !pending.is_empty()
        && preview
            .as_ref()
            .is_some_and(|preview| !preview.send(&pending))
    {
        preview = None;
    }

    writer
        .finalize()
        .map_err(|error| Error::Recording(error.to_string()))?;

    let transcript = if finalize.load(Ordering::SeqCst) {
        if let Some(preview) = preview {
            preview.finish().await
        } else {
            None
        }
    } else {
        None
    };
    Ok(RecordedAudio {
        transcript,
        file_path: path.to_string_lossy().into_owned(),
        duration_ms: sample_count.saturating_mul(1_000) / u64::from(SAMPLE_RATE),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use anlg_audio::{AudioProvider, CaptureConfig, CaptureFrame, CaptureStream};
    use futures_util::{SinkExt, stream};
    use tokio_tungstenite::tungstenite::Message;

    use super::*;

    /// A microphone of steady frames that counts its open streams.
    #[derive(Default)]
    struct TestAudio {
        open: Arc<AtomicUsize>,
        opened: AtomicUsize,
        /// The first stream fails after its first frame.
        fail_first: AtomicBool,
        /// Every stream fails after its first frame.
        fail_every: AtomicBool,
        /// The system default input, `test-mic` unless set.
        default: Mutex<Option<String>>,
    }

    struct OpenStream(Arc<AtomicUsize>);

    impl Drop for OpenStream {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl AudioProvider for TestAudio {
        fn open_capture(&self, _config: CaptureConfig) -> Result<CaptureStream, anlg_audio::Error> {
            unreachable!()
        }

        fn open_speaker_capture(
            &self,
            _sample_rate: u32,
            _chunk_size: usize,
        ) -> Result<CaptureStream, anlg_audio::Error> {
            unreachable!()
        }

        fn open_mic_capture(
            &self,
            _device: Option<String>,
            _sample_rate: u32,
            _chunk_size: usize,
        ) -> Result<CaptureStream, anlg_audio::Error> {
            self.open.fetch_add(1, Ordering::SeqCst);
            self.opened.fetch_add(1, Ordering::SeqCst);
            let open = OpenStream(self.open.clone());
            if self.fail_first.swap(false, Ordering::SeqCst)
                || self.fail_every.load(Ordering::SeqCst)
            {
                let frame = CaptureFrame {
                    raw_mic: Arc::from([0.25_f32; CHUNK_SIZE]),
                    raw_speaker: Arc::from([]),
                    aec_mic: None,
                };
                return Ok(CaptureStream::new(
                    stream::iter([Ok(frame), Err(anlg_audio::Error::MicOpenFailed)]).map(
                        move |item| {
                            let _open = &open;
                            item
                        },
                    ),
                ));
            }
            let frame = CaptureFrame {
                raw_mic: Arc::from([0.25_f32; CHUNK_SIZE]),
                raw_speaker: Arc::from([]),
                aec_mic: None,
            };
            Ok(CaptureStream::new(stream::unfold(
                (frame, open),
                |(frame, open)| async move {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    Some((Ok(frame.clone()), (frame, open)))
                },
            )))
        }

        fn default_device_name(&self) -> String {
            self.default
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "test-mic".to_string())
        }

        fn list_mic_devices(&self) -> Vec<String> {
            vec![self.default_device_name()]
        }

        fn play_silence(&self) -> std::sync::mpsc::Sender<()> {
            std::sync::mpsc::channel().0
        }

        fn play_bytes(&self, _bytes: &'static [u8]) -> std::sync::mpsc::Sender<()> {
            std::sync::mpsc::channel().0
        }

        fn probe_mic(&self, _device: Option<String>) -> Result<(), anlg_audio::Error> {
            Ok(())
        }

        fn probe_speaker(&self) -> Result<(), anlg_audio::Error> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn records_and_discards_a_temporary_wav() {
        let recorder = Recorder::new();
        recorder
            .start(Arc::new(TestAudio::default()), None, "test".into())
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let recorded = recorder.stop("test").await.unwrap();
        let path = PathBuf::from(&recorded.file_path);
        let reader = hound::WavReader::open(&path).unwrap();

        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
        assert!(recorded.duration_ms > 0);
        assert!(path.exists());

        recorder.discard(recorded.file_path).unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn another_owner_cannot_stop_or_cancel_a_recording() {
        let recorder = Recorder::new();
        recorder
            .start(Arc::new(TestAudio::default()), None, "chat".into())
            .unwrap();
        assert!(matches!(
            recorder.stop("system").await,
            Err(Error::AlreadyRecording)
        ));
        assert!(matches!(
            recorder.cancel("system").await,
            Err(Error::AlreadyRecording)
        ));
        recorder.cancel("chat").await.unwrap();
        recorder
            .start(Arc::new(TestAudio::default()), None, "system".into())
            .unwrap();
        recorder.cancel("system").await.unwrap();
    }

    #[tokio::test]
    async fn preview_failure_does_not_interrupt_wav_capture() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let failed = Arc::new(AtomicBool::new(false));
        let received = failed.clone();
        let updates = Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(json) = body
                && json.contains("previewUnavailable")
            {
                received.store(true, Ordering::SeqCst);
            }
            Ok(())
        });
        let recorder = Recorder::new();
        recorder
            .start_with_feedback(
                Arc::new(TestAudio::default()),
                None,
                "test".into(),
                Feedback {
                    preview: Some(PreviewConfig {
                        provider: "unsupported".into(),
                        base_url: "http://localhost".into(),
                        api_key: String::new(),
                        params: Default::default(),
                    }),
                    updates: Some(updates),
                    ..Feedback::default()
                },
            )
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !failed.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let recorded = recorder.stop("test").await.unwrap();
        assert!(recorded.duration_ms > 0);
        recorder.discard(recorded.file_path).unwrap();
    }

    /// The HUD's auto level reads raw loudness; the old feed scaled the RMS
    /// by 8 and pinned ordinary speech at full.
    #[tokio::test]
    async fn the_hud_meter_gets_each_buffers_raw_rms() {
        let levels = Arc::new(Mutex::new(Vec::new()));
        let heard = levels.clone();
        let recorder = Recorder::new();
        recorder
            .start_with_feedback(
                Arc::new(TestAudio::default()),
                None,
                "test".into(),
                Feedback {
                    meter: Some(Arc::new(move |rms| heard.lock().unwrap().push(rms))),
                    ..Feedback::default()
                },
            )
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        recorder.cancel("test").await.unwrap();
        let levels = levels.lock().unwrap();
        assert!(!levels.is_empty());
        assert!(levels.iter().all(|rms| (rms - 0.25).abs() < 1e-6));
    }

    /// Dictations overlap: the next capture opens the microphone while the
    /// last one still waits on its live transcript, other audio comes back
    /// before that transcript does, and no capture touches a recording
    /// another one left for transcription.
    #[tokio::test]
    async fn a_new_capture_starts_while_earlier_recordings_are_pending() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (reply, replied) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            ws.next().await.unwrap().unwrap();
            ws.send(Message::Text(r#"{"status":"auth"}"#.into()))
                .await
                .unwrap();
            while let Some(Ok(message)) = ws.next().await {
                if message
                    .into_text()
                    .is_ok_and(|text| text.contains("commit"))
                {
                    break;
                }
            }
            replied.await.unwrap();
            ws.send(Message::Text(
                r#"{"status":"text","final":true,"body":{"text":"First."}}"#.into(),
            ))
            .await
            .unwrap();
        });
        let audio = Arc::new(TestAudio::default());
        let recorder = Arc::new(Recorder::new());
        let ended = Arc::new(AtomicBool::new(false));
        let heard = ended.clone();
        recorder
            .start_with_feedback(
                audio.clone(),
                None,
                "system".into(),
                Feedback {
                    preview: Some(PreviewConfig {
                        provider: "wisprflow".into(),
                        base_url: format!("http://{address}"),
                        api_key: "test".into(),
                        params: Default::default(),
                    }),
                    updates: Some(Channel::new(move |body| {
                        if let tauri::ipc::InvokeResponseBody::Json(json) = body
                            && json.contains("ended")
                        {
                            heard.store(true, Ordering::SeqCst);
                        }
                        Ok(())
                    })),
                    environment: Some(RecordingEnvironment {
                        config: anlg_recording_environment::Config {
                            behavior: anlg_recording_environment::AudioBehavior::Nothing,
                            prevent_sleep: false,
                        },
                        delay_ms: 0,
                    }),
                    ..Feedback::default()
                },
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let first = tokio::spawn({
            let recorder = recorder.clone();
            async move { recorder.stop("system").await }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while audio.open.load(Ordering::SeqCst) > 0 || !ended.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert!(!first.is_finished());

        recorder
            .start(audio.clone(), None, "system".into())
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let second = recorder.stop("system").await.unwrap();
        recorder
            .start(audio.clone(), None, "system".into())
            .unwrap();
        recorder.cancel("system").await.unwrap();

        reply.send(()).unwrap();
        let first = tokio::time::timeout(Duration::from_secs(3), first)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(first.transcript.as_deref(), Some("First."));
        assert_ne!(first.file_path, second.file_path);
        recorder.discard(first.file_path.clone()).unwrap();
        assert!(!Path::new(&first.file_path).exists());
        assert!(
            hound::WavReader::open(&second.file_path)
                .unwrap()
                .duration()
                > 0
        );
        recorder.discard(second.file_path).unwrap();
        server.await.unwrap();
    }

    async fn until(condition: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }

    fn meter() -> (
        Arc<Mutex<Vec<LevelUpdate>>>,
        impl Fn(LevelUpdate) -> bool + Send + 'static,
    ) {
        let updates = Arc::new(Mutex::new(Vec::new()));
        let heard = updates.clone();
        (updates, move |update| {
            heard.lock().unwrap().push(update);
            true
        })
    }

    /// The Settings meter measures the microphone and lets go of it when
    /// stopped, when nobody stops it, and when a dictation needs it.
    #[tokio::test]
    async fn the_level_meter_releases_the_microphone() {
        let audio = Arc::new(TestAudio::default());
        let recorder = Recorder::new();
        let ended = |updates: &Arc<Mutex<Vec<LevelUpdate>>>| {
            updates.lock().unwrap().last() == Some(&LevelUpdate::Ended)
        };

        let (updates, emit) = meter();
        let source = recorder
            .monitor_level(audio.as_ref(), None, emit, Duration::from_secs(60))
            .unwrap();
        assert_eq!(source, LevelSource::Microphone);
        until(|| {
            updates
                .lock()
                .unwrap()
                .iter()
                .any(|update| matches!(update, LevelUpdate::Level { .. }))
        })
        .await;
        recorder.stop_level();
        until(|| ended(&updates) && audio.open.load(Ordering::SeqCst) == 0).await;

        let (updates, emit) = meter();
        recorder
            .monitor_level(audio.as_ref(), None, emit, Duration::from_millis(20))
            .unwrap();
        until(|| ended(&updates) && audio.open.load(Ordering::SeqCst) == 0).await;

        let (updates, emit) = meter();
        recorder
            .monitor_level(audio.as_ref(), None, emit, Duration::from_secs(60))
            .unwrap();
        recorder
            .start(audio.clone(), None, "system".into())
            .unwrap();
        until(|| ended(&updates) && audio.open.load(Ordering::SeqCst) == 1).await;
        let (updates, emit) = meter();
        assert_eq!(
            recorder
                .monitor_level(audio.as_ref(), None, emit, Duration::from_secs(60))
                .unwrap(),
            LevelSource::InUse
        );
        assert!(ended(&updates));
        recorder.cancel("system").await.unwrap();

        recorder
            .set_warm_microphone(audio.clone(), None, true, true)
            .unwrap();
        let (_, emit) = meter();
        assert_eq!(
            recorder
                .monitor_level(audio.as_ref(), None, emit, Duration::from_secs(60))
                .unwrap(),
            LevelSource::OpenMicrophone
        );
        assert_eq!(audio.open.load(Ordering::SeqCst), 1);
        recorder.stop_level();
    }

    /// Turning "Keep microphone ready" off or changing the microphone while
    /// dictating waits for the dictation to end instead of cutting it off.
    #[tokio::test]
    async fn closing_the_open_microphone_mid_dictation_waits_for_it_to_end() {
        let audio = Arc::new(TestAudio::default());
        let recorder = Recorder::new();
        recorder
            .set_warm_microphone(audio.clone(), None, true, true)
            .unwrap();
        recorder
            .start(audio.clone(), None, "system".into())
            .unwrap();
        recorder
            .set_warm_microphone(audio.clone(), None, true, false)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(audio.open.load(Ordering::SeqCst), 1);
        let recorded = recorder.stop("system").await.unwrap();
        assert!(recorded.duration_ms > 0);
        recorder.discard(recorded.file_path).unwrap();
        until(|| audio.open.load(Ordering::SeqCst) == 0).await;
    }

    /// An open microphone whose stream fails opens again on its own.
    #[tokio::test]
    async fn the_open_microphone_opens_again_after_a_stream_error() {
        let audio = Arc::new(TestAudio::default());
        audio.fail_first.store(true, Ordering::SeqCst);
        let recorder = Recorder::new();
        recorder
            .set_warm_microphone(audio.clone(), None, true, true)
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while audio.opened.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        recorder
            .set_warm_microphone(audio.clone(), None, true, false)
            .unwrap();
        until(|| audio.open.load(Ordering::SeqCst) == 0).await;
    }

    /// A microphone that keeps failing but works again in between keeps
    /// opening again: only failures in a row give up.
    #[tokio::test]
    async fn the_open_microphone_keeps_recovering_from_stream_errors() {
        let audio = Arc::new(TestAudio::default());
        audio.fail_every.store(true, Ordering::SeqCst);
        let recorder = Recorder::new();
        recorder
            .set_warm_microphone(audio.clone(), None, true, true)
            .unwrap();
        let attempts = crate::warm::MAX_REOPENS as usize + 3;
        tokio::time::timeout(Duration::from_secs(3), async {
            while audio.opened.load(Ordering::SeqCst) < attempts {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        recorder
            .set_warm_microphone(audio.clone(), None, true, false)
            .unwrap();
        until(|| audio.open.load(Ordering::SeqCst) == 0).await;
    }

    /// Windows reports no change of default input, so a dictation finds it
    /// itself: it records from the new default and the open microphone
    /// moves there for the next one.
    #[tokio::test]
    async fn the_open_microphone_follows_a_new_default_input() {
        let audio = Arc::new(TestAudio::default());
        let recorder = Recorder::new();
        recorder
            .set_warm_microphone(audio.clone(), None, true, true)
            .unwrap();
        *audio.default.lock().unwrap() = Some("headset".into());
        recorder
            .start(audio.clone(), None, "system".into())
            .unwrap();
        until(|| audio.opened.load(Ordering::SeqCst) == 3).await;
        recorder.cancel("system").await.unwrap();
        until(|| audio.open.load(Ordering::SeqCst) == 1).await;

        recorder
            .start(audio.clone(), None, "system".into())
            .unwrap();
        assert_eq!(audio.opened.load(Ordering::SeqCst), 3);
        recorder.cancel("system").await.unwrap();
    }

    #[tokio::test]
    async fn rejects_parallel_recordings() {
        let recorder = Recorder::new();
        recorder
            .start(Arc::new(TestAudio::default()), None, "test".into())
            .unwrap();

        assert!(matches!(
            recorder.start(Arc::new(TestAudio::default()), None, "test".into()),
            Err(Error::AlreadyRecording)
        ));

        recorder.cancel("test").await.unwrap();
    }
}
