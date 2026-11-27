use std::time::Duration;

use anlg_audio_chunking::{AudioChunk, Chunker, SpeechChunker, SpeechChunkingConfig};
use anlg_audio_utils::quiet_ranges;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

const DEFAULT_SPEECH_REDEMPTION_TIME: Duration = Duration::from_millis(150);
const MAX_CHUNK_SAMPLES: usize = TARGET_SAMPLE_RATE as usize * 25;

pub fn chunk_channel_audio<E>(samples: &[f32]) -> Result<Vec<AudioChunk>, E>
where
    E: From<anlg_audio_chunking::Error>,
{
    let mut chunker =
        SpeechChunker::new(SpeechChunkingConfig::speech(DEFAULT_SPEECH_REDEMPTION_TIME))?;
    Ok(chunk_channel_audio_with(samples, &mut chunker)?)
}

fn chunk_channel_audio_with<C>(
    samples: &[f32],
    chunker: &mut C,
) -> Result<Vec<AudioChunk>, C::Error>
where
    C: Chunker,
{
    let chunks = chunker.chunk(samples, TARGET_SAMPLE_RATE)?;
    let mut normalized = Vec::new();

    for chunk in chunks {
        if chunk.samples.len() <= MAX_CHUNK_SAMPLES {
            normalized.push(chunk);
            continue;
        }

        // An exact sample count cuts mid-word; each window ends at a quiet
        // moment near its limit instead.
        for range in quiet_ranges(&chunk.samples, MAX_CHUNK_SAMPLES, TARGET_SAMPLE_RATE) {
            normalized.push(AudioChunk {
                samples: chunk.samples[range.clone()].to_vec(),
                sample_start: chunk.sample_start + range.start,
                sample_end: chunk.sample_start + range.end,
            });
        }
    }

    tracing::info!(
        chunk_count = normalized.len(),
        chunk_durations_ms = ?normalized
            .iter()
            .map(|chunk| (chunk.sample_end - chunk.sample_start) * 1000 / TARGET_SAMPLE_RATE as usize)
            .collect::<Vec<_>>(),
        "audio_chunking_complete"
    );

    Ok(normalized)
}

pub fn split_resampled_channels(samples: &[f32], channel_count: usize) -> Vec<Vec<f32>> {
    if channel_count <= 1 {
        return vec![samples.to_vec()];
    }

    anlg_audio_utils::deinterleave(samples, channel_count)
}

pub fn channel_duration_sec(samples: &[f32]) -> f64 {
    samples.len() as f64 / TARGET_SAMPLE_RATE as f64
}

#[cfg(test)]
mod tests {
    use anlg_audio_chunking::{AudioChunk, Chunker};

    use super::*;
    use crate::{initial_resolved_until, next_resolved_until};

    struct FakeChunker {
        chunks: Vec<AudioChunk>,
    }

    impl Chunker for FakeChunker {
        type Error = std::convert::Infallible;

        fn chunk(
            &mut self,
            _samples: &[f32],
            _sample_rate: u32,
        ) -> Result<Vec<AudioChunk>, Self::Error> {
            Ok(self.chunks.clone())
        }
    }

    #[test]
    fn empty_audio_marks_channel_complete() {
        let chunks = chunk_channel_audio::<anlg_audio_chunking::Error>(&[]).unwrap();

        assert!(chunks.is_empty());
        assert_eq!(initial_resolved_until(&chunks, 40.0), 40.0);
    }

    #[test]
    fn oversized_chunks_are_split_at_a_quiet_moment_before_the_limit() {
        let second = TARGET_SAMPLE_RATE as usize;
        let oversized = MAX_CHUNK_SAMPLES + second;
        let mut samples = vec![1.0; oversized];
        samples[23 * second..23 * second + second / 10].fill(0.0);
        let mut chunker = FakeChunker {
            chunks: vec![AudioChunk {
                samples,
                sample_start: second * 2,
                sample_end: second * 2 + oversized,
            }],
        };

        let chunks = chunk_channel_audio_with(&vec![0.0; oversized], &mut chunker).unwrap();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].sample_start, second * 2);
        assert_eq!(chunks[0].sample_end, second * 2 + 23 * second + second / 20);
        assert_eq!(chunks[1].sample_start, chunks[0].sample_end);
        assert_eq!(chunks[1].sample_end, second * 2 + oversized);
        assert_eq!(
            chunks[1].samples.len(),
            chunks[1].sample_end - chunks[1].sample_start
        );
    }

    #[test]
    fn resolved_progress_uses_sample_offsets() {
        let mut chunker = FakeChunker {
            chunks: vec![
                AudioChunk {
                    samples: vec![0.0; TARGET_SAMPLE_RATE as usize * 2],
                    sample_start: TARGET_SAMPLE_RATE as usize * 4,
                    sample_end: TARGET_SAMPLE_RATE as usize * 6,
                },
                AudioChunk {
                    samples: vec![0.0; TARGET_SAMPLE_RATE as usize * 3],
                    sample_start: TARGET_SAMPLE_RATE as usize * 8,
                    sample_end: TARGET_SAMPLE_RATE as usize * 11,
                },
            ],
        };

        let chunks =
            chunk_channel_audio_with(&vec![0.0; TARGET_SAMPLE_RATE as usize * 11], &mut chunker)
                .unwrap();

        assert_eq!(initial_resolved_until(&chunks, 20.0), 4.0);
        assert_eq!(next_resolved_until(&chunks, 0, 20.0), 8.0);
        assert_eq!(next_resolved_until(&chunks, 1, 20.0), 20.0);
    }
}
