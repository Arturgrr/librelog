//! An optional microphone kept open between dictations, so a dictation can
//! include what was said just before its shortcut: Hex's warm capture and its
//! 450 ms pre-roll (MIT, Christopher Langton).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use tauri::async_runtime::JoinHandle;
use tokio::sync::mpsc;

use crate::Error;
use crate::recorder::SAMPLE_RATE;

/// Audio a dictation keeps from before its shortcut was pressed.
pub(crate) const PRE_ROLL: Duration = Duration::from_millis(450);
/// What the open microphone remembers: the pre-roll, plus up to a second for
/// the start to reach the recorder after the press (HUD, permission checks).
const RING: Duration = Duration::from_millis(1_500);

pub(crate) type Samples = BoxStream<'static, Result<Arc<[f32]>, Error>>;

fn samples_in(duration: Duration) -> usize {
    (u128::from(SAMPLE_RATE) * duration.as_millis() / 1_000) as usize
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
            since_press.min(RING) + PRE_ROLL
        }
        Some(_) => Duration::ZERO,
    }
}

#[derive(Default)]
struct Ring {
    samples: VecDeque<f32>,
    listener: Option<mpsc::UnboundedSender<Result<Arc<[f32]>, Error>>>,
    open: bool,
}

impl Ring {
    fn push(&mut self, samples: Arc<[f32]>) {
        let capacity = samples_in(RING);
        self.samples.extend(samples.iter().copied());
        let excess = self.samples.len().saturating_sub(capacity);
        self.samples.drain(..excess);
        if let Some(listener) = &self.listener
            && listener.send(Ok(samples)).is_err()
        {
            self.listener = None;
        }
    }

    /// The live audio from now on, led by the last `lookback` of it.
    fn listen(&mut self, lookback: Duration) -> Option<Samples> {
        if !self.open {
            return None;
        }
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let earlier = samples_in(lookback).min(self.samples.len());
        if earlier > 0 {
            let start = self.samples.len() - earlier;
            let pre_roll: Arc<[f32]> = self.samples.range(start..).copied().collect();
            let _ = sender.send(Ok(pre_roll));
        }
        self.listener = Some(sender);
        Some(futures_util::stream::poll_fn(move |context| receiver.poll_recv(context)).boxed())
    }
}

/// The open microphone; dropping it closes the microphone.
pub(crate) struct WarmMicrophone {
    pub(crate) device: Option<String>,
    ring: Arc<Mutex<Ring>>,
    task: JoinHandle<()>,
}

impl WarmMicrophone {
    pub(crate) fn open(
        audio: &dyn anlg_audio::AudioProvider,
        device: Option<String>,
        chunk_size: usize,
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
            let ended = loop {
                match stream.next().await {
                    Some(Ok(frame)) => lock(&task_ring).push(frame.preferred_mic()),
                    Some(Err(error)) => break Some(Error::Capture(error.to_string())),
                    None => break None,
                }
            };
            tracing::warn!(error = ?ended, "the open dictation microphone stopped");
            let mut ring = lock(&task_ring);
            ring.open = false;
            if let (Some(listener), Some(error)) = (ring.listener.take(), ended) {
                let _ = listener.send(Err(error));
            }
        });
        Ok(Self { device, ring, task })
    }

    pub(crate) fn is_open(&self) -> bool {
        lock(&self.ring).open
    }

    /// The microphone's audio for one recording, `lookback` of it from
    /// before now included; `None` once the microphone has stopped. A new
    /// listener replaces the last one.
    pub(crate) fn listen(&self, lookback: Duration) -> Option<Samples> {
        lock(&self.ring).listen(lookback)
    }
}

impl Drop for WarmMicrophone {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn lock(ring: &Mutex<Ring>) -> std::sync::MutexGuard<'_, Ring> {
    ring.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dictation from the open microphone starts with the audio from just
    /// before its press, then carries on live; what the ring forgot is gone.
    #[tokio::test]
    async fn a_listener_gets_the_pre_roll_then_live_audio() {
        let mut ring = Ring {
            open: true,
            ..Ring::default()
        };
        let second = SAMPLE_RATE as usize;
        for index in 0..4 {
            ring.push(vec![index as f32; second].into());
        }
        let mut heard = ring.listen(PRE_ROLL).unwrap();
        ring.push(vec![4.0; 320].into());

        let pre_roll = heard.next().await.unwrap().unwrap();
        assert_eq!(pre_roll.len(), samples_in(PRE_ROLL));
        assert!(pre_roll.iter().all(|sample| *sample == 3.0));
        let live = heard.next().await.unwrap().unwrap();
        assert_eq!(&*live, &[4.0; 320][..]);
        assert_eq!(ring.samples.len(), samples_in(RING));
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
