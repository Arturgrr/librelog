use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anlg_audio_utils::Source;

use crate::encoder::f32_to_i16;
use crate::{Error, MonoStreamEncoder};

const ENCODE_FRAMES: usize = 4096;

/// One MP3 segment of a split recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub path: PathBuf,
    /// Where the segment starts in the source recording.
    pub start: Duration,
}

/// Decodes any supported audio file, downmixes it to mono, and writes it back out
/// as MP3 segments of at most `segment_duration`. Providers that cap upload size
/// or time out on long audio get one request per segment. A segment ends at the
/// quietest moment of its last few seconds rather than mid-word at its limit.
pub fn encode_mono_segments(
    source_path: &Path,
    output_dir: &Path,
    segment_duration: Duration,
) -> Result<Vec<Segment>, Error> {
    let source = anlg_audio_utils::source_from_path(source_path)?;
    let sample_rate: u32 = source.sample_rate().into();
    let channels = usize::from(u16::from(source.channels())).max(1);
    let frames_per_segment = frames_per_segment(sample_rate, segment_duration)?;
    let search = anlg_audio_utils::quiet_search_samples(frames_per_segment, sample_rate);

    let mut segments: Vec<Segment> = Vec::new();
    let mut current: Option<SegmentEncoder> = None;
    // Frames in the segments already finished.
    let mut finished = 0_usize;
    // The end of the current segment's window, where its cut is looked for.
    let mut lookahead: Vec<f32> = Vec::with_capacity(search);
    let open = |segments: &mut Vec<Segment>, start: usize| {
        let path = output_dir.join(format!("segment-{:04}.mp3", segments.len()));
        let encoder = SegmentEncoder::create(&path, sample_rate)?;
        segments.push(Segment {
            path,
            start: Duration::from_secs_f64(start as f64 / f64::from(sample_rate)),
        });
        Ok::<_, Error>(encoder)
    };

    for sample in anlg_audio_utils::mono_frames(source, channels) {
        if current.is_none() {
            current = Some(open(&mut segments, finished)?);
        }
        let segment = current.as_mut().expect("segment was just created");
        if segment.frames < frames_per_segment - search {
            segment.push(sample)?;
            continue;
        }
        lookahead.push(sample);
        if segment.frames + lookahead.len() < frames_per_segment {
            continue;
        }

        let cut = anlg_audio_utils::quietest_point(&lookahead, sample_rate).max(1);
        for sample in lookahead.drain(..cut) {
            segment.push(sample)?;
        }
        finished += segment.frames;
        current
            .take()
            .expect("segment was just observed")
            .finish()?;
        if !lookahead.is_empty() {
            let mut next = open(&mut segments, finished)?;
            for sample in lookahead.drain(..) {
                next.push(sample)?;
            }
            current = Some(next);
        }
    }

    if let Some(mut segment) = current {
        for sample in lookahead.drain(..) {
            segment.push(sample)?;
        }
        segment.finish()?;
    }

    Ok(segments)
}

fn frames_per_segment(sample_rate: u32, segment_duration: Duration) -> Result<usize, Error> {
    let frames = (u128::from(sample_rate) * segment_duration.as_millis()) / 1000;
    usize::try_from(frames)
        .ok()
        .filter(|frames| *frames > 0)
        .ok_or(Error::InvalidSegmentDuration(segment_duration))
}

struct SegmentEncoder {
    encoder: MonoStreamEncoder,
    writer: BufWriter<File>,
    pcm: Vec<i16>,
    encoded: Vec<u8>,
    frames: usize,
}

impl SegmentEncoder {
    fn create(path: &Path, sample_rate: u32) -> Result<Self, Error> {
        Ok(Self {
            encoder: MonoStreamEncoder::new(sample_rate)?,
            writer: BufWriter::new(File::create(path)?),
            pcm: Vec::with_capacity(ENCODE_FRAMES),
            encoded: Vec::new(),
            frames: 0,
        })
    }

    fn push(&mut self, sample: f32) -> Result<(), Error> {
        self.pcm.push(f32_to_i16(sample));
        self.frames += 1;

        if self.pcm.len() >= ENCODE_FRAMES {
            self.drain()?;
        }
        Ok(())
    }

    fn drain(&mut self) -> Result<(), Error> {
        self.encoded.clear();
        self.encoder.encode_i16(&self.pcm, &mut self.encoded)?;
        self.writer.write_all(&self.encoded)?;
        self.pcm.clear();
        Ok(())
    }

    fn finish(mut self) -> Result<(), Error> {
        self.drain()?;
        self.encoded.clear();
        self.encoder.flush(&mut self.encoded)?;
        self.writer.write_all(&self.encoded)?;
        self.writer.flush()?;
        self.writer.get_ref().sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_per_segment_scales_with_sample_rate() {
        assert_eq!(
            frames_per_segment(16_000, Duration::from_secs(10)).unwrap(),
            160_000
        );
        assert!(frames_per_segment(16_000, Duration::ZERO).is_err());
    }
}
