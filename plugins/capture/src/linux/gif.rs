//! A recording as a looping GIF: how many frames at what size (the same
//! budget as the macOS layer), and the encoder fed raw RGBA frames one at
//! a time, so a long recording is never held in memory.

use std::io::{Read, Write};

/// At most this many output pixels over all frames.
pub const MAX_PIXELS: u64 = 150_000_000;
/// The fps is never lowered below what keeps this many frames.
const MIN_FRAMES: u64 = 10;
/// NeuQuant's sampling: 1 is best, 30 fastest; 10 is the crate's default.
const QUANTIZE_SPEED: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// Frames a second, possibly lowered to fit.
    pub fps: f64,
}

/// The GIF for a `duration_ms` movie of `source` pixels: no wider than
/// `max_width` (never scaled up), sampled at `fps` unless more than
/// `max_frames` frames or [`MAX_PIXELS`] pixels would be needed, then at
/// the fps that fits, but never fewer than ten frames (a GIF too large for
/// ten frames is made smaller instead).
pub fn plan(
    duration_ms: u64,
    source: (u32, u32),
    fps: u32,
    max_width: u32,
    max_frames: u32,
) -> Plan {
    let (source_width, source_height) = (source.0.max(1), source.1.max(1));
    let mut width = source_width.min(max_width.max(1));
    let mut height =
        ((u64::from(source_height) * u64::from(width)) / u64::from(source_width)).max(1) as u32;
    let seconds = (duration_ms.max(1) as f64) / 1000.0;
    let natural = ((seconds * f64::from(fps.max(1))).ceil() as u64).max(1);
    let pixels = |width: u32, height: u32| u64::from(width) * u64::from(height);
    let mut cap = u64::from(max_frames.max(1)).min(MAX_PIXELS / pixels(width, height).max(1));
    let floor = natural.min(MIN_FRAMES);
    if cap < floor {
        let shrink = (MAX_PIXELS as f64 / (floor as f64 * pixels(width, height) as f64)).sqrt();
        width = ((f64::from(width) * shrink).floor() as u32).max(1);
        height = ((f64::from(height) * shrink).floor() as u32).max(1);
        cap = floor;
    }
    let frames = natural.min(cap);
    let fps = if frames < natural {
        frames as f64 / seconds
    } else {
        f64::from(fps.max(1))
    };
    Plan {
        width,
        height,
        frames: frames as u32,
        fps,
    }
}

/// Frame `index`'s delay in hundredths of a second: the real sample times,
/// rounded without drifting, at least 2.
pub fn delay(index: u32, fps: f64) -> u16 {
    let at = |frame: u32| (f64::from(frame) * 100.0 / fps).round() as i64;
    (at(index + 1) - at(index)).clamp(2, i64::from(u16::MAX)) as u16
}

/// Encodes up to `plan.frames` RGBA frames read from `frames` into `out`;
/// returns how many there were. Stops at the end of the input.
pub fn encode(frames: &mut impl Read, out: impl Write, plan: &Plan) -> Result<u32, String> {
    let (width, height) = (
        u16::try_from(plan.width).map_err(|_| "the GIF is too wide")?,
        u16::try_from(plan.height).map_err(|_| "the GIF is too tall")?,
    );
    let mut encoder =
        gif::Encoder::new(out, width, height, &[]).map_err(|error| error.to_string())?;
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|error| error.to_string())?;
    let mut rgba = vec![0u8; plan.width as usize * plan.height as usize * 4];
    let mut written = 0;
    while written < plan.frames {
        match read_frame(frames, &mut rgba) {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => return Err(error.to_string()),
        }
        let mut frame = gif::Frame::from_rgba_speed(width, height, &mut rgba, QUANTIZE_SPEED);
        frame.delay = delay(written, plan.fps);
        encoder
            .write_frame(&frame)
            .map_err(|error| error.to_string())?;
        written += 1;
    }
    encoder.into_inner().map_err(|error| error.to_string())?;
    Ok(written)
}

/// Fills `frame`; false at a clean end of input (a partial last frame is
/// dropped).
fn read_frame(input: &mut impl Read, frame: &mut [u8]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < frame.len() {
        match input.read(&mut frame[filled..]) {
            Ok(0) => return Ok(false),
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_or_large_gifs_lower_the_fps_but_keep_ten_frames() {
        // Short and small: as asked.
        assert_eq!(
            plan(2_000, (640, 360), 15, 720, 900),
            Plan {
                width: 640,
                height: 360,
                frames: 30,
                fps: 15.0
            }
        );
        // Scaled down to the width, never up.
        let wide = plan(2_000, (2880, 1800), 15, 720, 900);
        assert_eq!((wide.width, wide.height), (720, 450));
        // Too many frames: the fps goes down to fit 900.
        let long = plan(120_000, (480, 270), 15, 720, 900);
        assert_eq!(long.frames, 900);
        assert!((long.fps - 7.5).abs() < 1e-9, "{long:?}");
        // Too many pixels: 150 M / (1080 × 1080) is 128 frames.
        let large = plan(60_000, (1080, 1080), 20, 1080, 900);
        assert_eq!(large.frames, 128);
        assert!(u64::from(large.frames) * 1080 * 1080 <= MAX_PIXELS);
        // So large that ten frames would not fit: smaller, still ten.
        let huge = plan(60_000, (8000, 8000), 20, 8000, 900);
        assert_eq!(huge.frames, 10);
        assert!(u64::from(huge.frames) * u64::from(huge.width * huge.height) <= MAX_PIXELS);
    }

    #[test]
    fn frames_are_encoded_as_a_looping_gif_with_their_sample_times() {
        let plan = Plan {
            width: 4,
            height: 2,
            frames: 3,
            fps: 15.0,
        };
        // Five frames come in; the plan keeps three.
        let mut input: Vec<u8> = Vec::new();
        for shade in [0u8, 60, 120, 180, 240] {
            input.extend(std::iter::repeat_n([shade, 255 - shade, 0, 255], 8).flatten());
        }
        let mut out = Vec::new();
        assert_eq!(encode(&mut input.as_slice(), &mut out, &plan).unwrap(), 3);
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(out.as_slice()).unwrap();
        assert_eq!((decoder.width(), decoder.height()), (4, 2));
        let mut delays = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            delays.push(frame.delay);
        }
        // 1/15 s is 6.67 hundredths: 7, 6, 7.
        assert_eq!(delays, [7, 6, 7]);
        assert_eq!(decoder.repeat(), gif::Repeat::Infinite);
        // A cut-off frame at the end is dropped, not padded.
        let mut out = Vec::new();
        assert_eq!(encode(&mut &input[..40], &mut out, &plan).unwrap(), 1);
    }
}
