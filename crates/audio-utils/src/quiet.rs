//! Cutting long audio into model-sized windows at a quiet moment instead of
//! an exact sample count, which lands mid-word. Adapted from Hex's
//! `quiet_cut` and `chunk_ranges` (MIT, Christopher Langton).

use std::ops::Range;
use std::time::Duration;

/// The span a long clip is cut through.
const QUIET_SPAN: Duration = Duration::from_millis(100);
/// How far back from a window's end the quietest span is looked for.
const QUIET_SEARCH: Duration = Duration::from_secs(5);

fn samples_in(duration: Duration, sample_rate: u32) -> usize {
    (u128::from(sample_rate) * duration.as_millis() / 1_000) as usize
}

/// How many samples at the end of a window of `max_samples` the cut is looked
/// for in: five seconds, or half the window when that is shorter.
pub fn quiet_search_samples(max_samples: usize, sample_rate: u32) -> usize {
    samples_in(QUIET_SEARCH, sample_rate)
        .min(max_samples.max(1) / 2)
        .max(1)
}

/// The middle of the quietest 100 ms of `samples`, the latest one on a tie.
pub fn quietest_point(samples: &[f32], sample_rate: u32) -> usize {
    if samples.is_empty() {
        return 0;
    }
    let span = samples_in(QUIET_SPAN, sample_rate).clamp(1, samples.len());
    let energy_of = |sample: f32| f64::from(sample) * f64::from(sample);
    let mut energy: f64 = samples[..span].iter().copied().map(energy_of).sum();
    let mut quietest = energy;
    let mut quietest_start = 0;
    for next in 1..=samples.len() - span {
        energy += energy_of(samples[next + span - 1]) - energy_of(samples[next - 1]);
        if energy <= quietest {
            quietest = energy;
            quietest_start = next;
        }
    }
    quietest_start + span / 2
}

/// Where a window of at most `max_samples` is cut into the next one: the
/// quietest point of its last [`quiet_search_samples`], never its first
/// sample. Only `samples[..max_samples]` is read, so a streaming reader can
/// cut before it has read the rest.
pub fn quiet_cut(samples: &[f32], max_samples: usize, sample_rate: u32) -> usize {
    let max_samples = max_samples.max(1).min(samples.len().max(1));
    let search = quiet_search_samples(max_samples, sample_rate).min(max_samples);
    let from = max_samples - search;
    (from + quietest_point(&samples[from..max_samples.min(samples.len())], sample_rate)).max(1)
}

/// Consecutive ranges of at most `max_samples` covering every sample once,
/// each cut by [`quiet_cut`]. Continuous speech with no quieter stretch near
/// a window's end is still cut at its quietest point, which can be inside a
/// word.
pub fn quiet_ranges(samples: &[f32], max_samples: usize, sample_rate: u32) -> Vec<Range<usize>> {
    let max_samples = max_samples.max(1);
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        if samples.len() - start <= max_samples {
            ranges.push(start..samples.len());
            break;
        }
        let cut = start + quiet_cut(&samples[start..], max_samples, sample_rate);
        ranges.push(start..cut);
        start = cut;
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// A tone with one quiet 100 ms inside each window's last five seconds:
    /// every cut falls in the middle of it, not at the window's last sample.
    #[test]
    fn long_audio_is_cut_at_the_quietest_point_near_each_window_end() {
        let second = RATE as usize;
        let mut samples = vec![0.5_f32; 70 * second];
        for quiet_at in [27 * second, 55 * second] {
            samples[quiet_at..quiet_at + second / 10].fill(0.0);
        }

        let ranges = quiet_ranges(&samples, 30 * second, RATE);

        assert_eq!(
            ranges,
            vec![
                0..27 * second + second / 20,
                27 * second + second / 20..55 * second + second / 20,
                55 * second + second / 20..70 * second,
            ]
        );
    }
}
