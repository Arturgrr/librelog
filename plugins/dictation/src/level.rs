//! The microphone's live level for Settings, so the person can check that it
//! hears them. Nothing is recorded: the audio is only measured. Measured
//! around each window's mean, as Hex's input level (MIT, Christopher
//! Langton), so a microphone with a DC offset never reads as loud.

use std::time::Duration;

use futures_util::StreamExt;
use tauri::async_runtime::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::warm::Samples;

/// 25 levels a second.
pub(crate) const LEVEL_INTERVAL: Duration = Duration::from_millis(40);
/// A meter nobody stops (a closed window) still lets go of the microphone.
pub(crate) const MAX_LEVEL_MONITOR: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LevelUpdate {
    /// The last window's RMS and peak, 0 to 1.
    Level { rms: f32, peak: f32 },
    /// The meter let go of the microphone: stopped, timed out, a dictation
    /// took the microphone, or the microphone stopped.
    Ended,
}

/// Where the meter reads its level from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LevelSource {
    /// A stream opened for the meter.
    Microphone,
    /// The microphone kept open for dictation.
    OpenMicrophone,
    /// A dictation is recording, so no meter opened.
    InUse,
}

/// RMS and peak of `samples` around their mean; samples that are not finite
/// are left out.
pub(crate) fn measure(samples: &[f32]) -> (f32, f32) {
    let finite = || samples.iter().copied().filter(|sample| sample.is_finite());
    let count = finite().count();
    if count == 0 {
        return (0.0, 0.0);
    }
    let mean = finite().map(f64::from).sum::<f64>() / count as f64;
    let (squares, peak) = finite().fold((0.0_f64, 0.0_f64), |(squares, peak), sample| {
        let centered = f64::from(sample) - mean;
        (squares + centered * centered, peak.max(centered.abs()))
    });
    let rms = (squares / count as f64).sqrt();
    (rms.min(1.0) as f32, peak.min(1.0) as f32)
}

/// A running meter; dropping it stops it.
pub(crate) struct LevelMonitor {
    stop: CancellationToken,
    _task: JoinHandle<()>,
    /// Whether it opened its own stream, which a dictation must not share.
    pub(crate) owns_stream: bool,
}

impl LevelMonitor {
    /// Measures `stream` every [`LEVEL_INTERVAL`] until stopped, `max` has
    /// passed, the stream ends, or `emit` reports nobody is listening.
    pub(crate) fn start(
        mut stream: Samples,
        owns_stream: bool,
        emit: impl Fn(LevelUpdate) -> bool + Send + 'static,
        max: Duration,
    ) -> Self {
        let stop = CancellationToken::new();
        let task_stop = stop.clone();
        let task = tauri::async_runtime::spawn(async move {
            let deadline = tokio::time::sleep(max);
            tokio::pin!(deadline);
            let mut tick = tokio::time::interval(LEVEL_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut window = Vec::new();
            loop {
                tokio::select! {
                    _ = task_stop.cancelled() => break,
                    _ = &mut deadline => break,
                    _ = tick.tick() => {
                        if window.is_empty() {
                            continue;
                        }
                        let (rms, peak) = measure(&window);
                        window.clear();
                        if !emit(LevelUpdate::Level { rms, peak }) {
                            break;
                        }
                    }
                    next = stream.next() => match next {
                        Some(Ok(samples)) => window.extend_from_slice(&samples),
                        Some(Err(error)) => {
                            tracing::warn!(%error, "the microphone level meter stopped");
                            break;
                        }
                        None => break,
                    },
                }
            }
            drop(stream);
            emit(LevelUpdate::Ended);
        });
        Self {
            stop,
            _task: task,
            owns_stream,
        }
    }
}

impl Drop for LevelMonitor {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_measured_around_the_mean() {
        let (rms, peak) = measure(&[0.7, -0.3]);
        assert!((rms - 0.5).abs() < 1e-6, "{rms}");
        assert!((peak - 0.5).abs() < 1e-6, "{peak}");
        let (rms, peak) = measure(&[0.2; 512]);
        assert!(rms < 1e-6 && peak < 1e-6, "{rms} {peak}");
        assert_eq!(measure(&[f32::NAN]), (0.0, 0.0));
    }
}
