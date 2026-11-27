//! The release tail: audio kept after a dictation is stopped, because the
//! shortcut often goes up while the last word is still sounding. Adapted from
//! Hex's `POST_RELEASE_TAIL`, `MAX_RELEASE_TAIL` and `VoiceTrack` (MIT,
//! Christopher Langton), with a plain RMS detector in place of Hex's.

use std::collections::VecDeque;
use std::time::Duration;

use crate::recorder::SAMPLE_RATE;

/// Audio kept after every stop.
pub(crate) const RELEASE_TAIL: Duration = Duration::from_millis(200);
/// The longest the tail grows while speech carries on past [`RELEASE_TAIL`].
pub(crate) const MAX_RELEASE_TAIL: Duration = Duration::from_millis(600);
/// How long past the tail a stop waits for a stalled microphone to deliver it.
pub(crate) const TAIL_DELIVERY_GRACE: Duration = Duration::from_millis(100);

/// 20 ms at 16 kHz, the frame speech is judged on.
const FRAME: usize = 320;
/// Speech frames that count as still talking: a key click is far shorter.
const SPEECH_FRAMES: usize = 5;
/// Quiet frames a run of speech may skip over, as between syllables.
const SPEECH_GAP_FRAMES: usize = 2;
/// Quiet frames in a row that end a growing tail (160 ms).
const QUIET_FRAMES: usize = 8;
/// A frame quieter than this (-60 dBFS) is never speech.
const SPEECH_FLOOR: f32 = 0.001;
/// How far above the room's noise a frame must be to be speech (12 dB).
const SPEECH_OVER_NOISE: f32 = 4.0;
const HISTORY_FRAMES: usize = 32;

fn samples_in(duration: Duration) -> usize {
    (u128::from(SAMPLE_RATE) * duration.as_millis() / 1_000) as usize
}

/// Which recent 20 ms frames of a recording sound like speech, judged against
/// the recording's own noise floor.
#[derive(Default)]
pub(crate) struct Voice {
    frame: Vec<f32>,
    noise: Option<f32>,
    flags: VecDeque<bool>,
}

impl Voice {
    /// Classifies `samples`; returns whether the last frame they complete is
    /// speech, when they complete one.
    pub(crate) fn extend(&mut self, mut samples: &[f32]) -> Option<bool> {
        let mut completed = None;
        while !samples.is_empty() {
            let take = samples.len().min(FRAME - self.frame.len());
            self.frame.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.frame.len() == FRAME {
                let speech = self.judge(crate::events::rms(&self.frame));
                self.frame.clear();
                self.flags.push_back(speech);
                if self.flags.len() > HISTORY_FRAMES {
                    self.flags.pop_front();
                }
                completed = Some(speech);
            }
        }
        completed
    }

    /// Samples until the frame in progress is complete.
    fn until_frame_end(&self) -> usize {
        FRAME - self.frame.len()
    }

    fn judge(&mut self, rms: f32) -> bool {
        let noise = self.noise.get_or_insert(rms.max(1e-5));
        let speech = rms >= SPEECH_FLOOR.max(*noise * SPEECH_OVER_NOISE);
        // The floor drops at once to a quieter frame, follows noise up
        // slowly, and barely moves while someone talks.
        if rms < *noise {
            *noise = rms.max(1e-5);
        } else {
            *noise += (rms - *noise) * if speech { 0.001 } else { 0.05 };
        }
        speech
    }

    /// Whether speech runs up to the newest frame: its last speech frame is
    /// within a syllable gap of it, and [`SPEECH_FRAMES`] speech frames lead
    /// up to it with no longer gap.
    fn talking(&self) -> bool {
        let mut speech = 0;
        let mut gap = 0;
        for &flag in self.flags.iter().rev() {
            if flag {
                speech += 1;
                gap = 0;
                if speech >= SPEECH_FRAMES {
                    return true;
                }
            } else {
                gap += 1;
                if gap > SPEECH_GAP_FRAMES {
                    return false;
                }
            }
        }
        false
    }
}

/// The audio a stopped recording still takes: [`RELEASE_TAIL`], or, while
/// speech carries on past it, until [`QUIET_FRAMES`] of quiet or
/// [`MAX_RELEASE_TAIL`].
#[derive(Default)]
pub(crate) struct ReleaseTail {
    kept: usize,
    extending: bool,
    quiet: usize,
}

pub(crate) struct Fed {
    /// How many of the samples fed belong to the recording.
    pub keep: usize,
    pub done: bool,
}

impl ReleaseTail {
    /// Whether speech has made the tail grow past [`RELEASE_TAIL`].
    pub(crate) fn extending(&self) -> bool {
        self.extending
    }

    /// Takes the next `samples` captured after the stop, judging them with
    /// `voice`, which has heard the recording so far.
    pub(crate) fn feed(&mut self, samples: &[f32], voice: &mut Voice) -> Fed {
        let base = samples_in(RELEASE_TAIL);
        let max = samples_in(MAX_RELEASE_TAIL);
        let mut taken = 0;
        loop {
            if !self.extending && self.kept >= base {
                if !voice.talking() {
                    return Fed {
                        keep: taken,
                        done: true,
                    };
                }
                self.extending = true;
            }
            if self.kept >= max {
                return Fed {
                    keep: taken,
                    done: true,
                };
            }
            if taken == samples.len() {
                return Fed {
                    keep: taken,
                    done: false,
                };
            }
            let room = if self.extending { max } else { base } - self.kept;
            let take = room.min(voice.until_frame_end()).min(samples.len() - taken);
            let completed = voice.extend(&samples[taken..taken + take]);
            self.kept += take;
            taken += take;
            if self.extending
                && let Some(speech) = completed
            {
                self.quiet = if speech { 0 } else { self.quiet + 1 };
                if self.quiet >= QUIET_FRAMES {
                    return Fed {
                        keep: taken,
                        done: true,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEECH: f32 = 0.1;
    const ROOM: f32 = 0.0005;

    /// A recording of `before` seconds of room noise and speech up to the
    /// stop, then the tail fed in 20 ms buffers of `after(ms)` until it is
    /// done: how many milliseconds of it are kept.
    fn tail_after(after: impl Fn(usize) -> f32) -> usize {
        let mut voice = Voice::default();
        voice.extend(&[ROOM; 16_000]);
        voice.extend(&[SPEECH; 8_000]);
        let mut tail = ReleaseTail::default();
        let mut kept = 0;
        for ms in (0..2_000).step_by(20) {
            let fed = tail.feed(&[after(ms); FRAME], &mut voice);
            kept += fed.keep;
            if fed.done {
                return kept * 1_000 / SAMPLE_RATE as usize;
            }
        }
        panic!("the tail never ended");
    }

    /// A stop in silence keeps exactly the base tail; a last word still
    /// being said grows it until the speech ends, and continuous speech
    /// is cut at the longest tail.
    #[test]
    fn the_tail_grows_while_speech_carries_on_and_caps_at_600_ms() {
        assert_eq!(tail_after(|_| ROOM), 200);
        assert_eq!(
            tail_after(|ms| if ms < 300 { SPEECH } else { ROOM }),
            300 + 160
        );
        assert_eq!(tail_after(|_| SPEECH), 600);
    }
}
