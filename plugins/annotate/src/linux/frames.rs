//! Reading the movie back with `gst-launch-1.0` children: small candidate
//! frames when the recording stops, full-size key frames when it is
//! finalized. A movie is read once from its start, four frames a second;
//! files reach a pipeline as inherited descriptors, never as paths.
//!
//! The ink overlay is on screen, so on Linux the movie shows the ink.
//! Annotate draws the marks onto its key frames itself, so a frame taken
//! while ink showed is replaced by the last one before that ink appeared:
//! the pointer belongs to the overlay while ink is on, so the screen
//! underneath rarely changes meanwhile.

use std::collections::HashSet;
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::time::{Duration, Instant};

use anlg_annotate_macos::{AnnotateError, codes};
use serde::Deserialize;
use serde_json::json;
use tauri_plugin_capture::recorder::{Gst, first_frame_args, pipe, wait};

use super::fail;

/// The frames' spacing in the movie.
pub const PERIOD_MS: i64 = 250;
pub const CANDIDATE_EDGE: u32 = 480;
/// Key frames' long edge at most, as on macOS.
pub const FRAME_EDGE: u32 = 2560;
/// Ink can show this long before the first point the overlay reported...
const INK_BEFORE_MS: i64 = 300;
/// ...and this long after the last: a lifted stroke is held 650 ms and fades
/// over 650 ms, and the screen lags the clock.
const INK_AFTER_MS: i64 = 650 + 650 + 400;
/// A frame after this much continuous ink keeps its own picture: the screen
/// before it is too old to stand in.
const LOOKBACK_MS: i64 = 10_000;
const SIZE_WITHIN: Duration = Duration::from_secs(15);
const READ_BASE: Duration = Duration::from_secs(20);
/// More than any one frame's PNG.
const PNG_LIMIT: u64 = 256 * 1024 * 1024;

pub struct Decoded {
    pub count: usize,
    /// Every frame of the movie was read.
    pub complete: bool,
}

/// GStreamer, behind a trait for tests.
pub trait Decoder: Send + Sync {
    /// The movie's frame size.
    fn size(&self, video: &Path) -> Result<(u32, u32), String>;
    /// The movie's frames from its start, `PERIOD_MS` apart, as RGBA at
    /// `size`, handed to `frame` until it returns false, the movie ends or
    /// `within` passes.
    fn frames(
        &self,
        video: &Path,
        size: (u32, u32),
        within: Duration,
        frame: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<Decoded, String>;
}

/// The longest side at most `edge`, never larger, both sides even.
pub fn fit((width, height): (u32, u32), edge: u32) -> (u32, u32) {
    let long = f64::from(width.max(height).max(1));
    let factor = (f64::from(edge) / long).min(1.0);
    let side = |value: u32| (((f64::from(value) * factor).round() as u32) & !1).max(2);
    (side(width), side(height))
}

/// When ink showed: each stroke's `(t0, t1)` widened by how long its ink
/// can be on screen, merged where they nearly touch.
pub fn episodes(strokes: &[(u64, u64)]) -> Vec<(i64, i64)> {
    let mut spans: Vec<(i64, i64)> = strokes
        .iter()
        .map(|&(t0, t1)| (t0 as i64 - INK_BEFORE_MS, t1 as i64 + INK_AFTER_MS))
        .collect();
    spans.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start <= last.1 + 2 * PERIOD_MS => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// The moment whose picture stands for `t`: `t` itself, or the start of
/// the ink showing at `t`.
pub fn clean(t: i64, episodes: &[(i64, i64)]) -> i64 {
    episodes
        .iter()
        .find(|(start, end)| *start <= t && t <= *end && t - start <= LOOKBACK_MS)
        .map_or(t, |(start, _)| *start)
}

/// The frame shown at movie time `ms`.
pub fn index(ms: i64) -> usize {
    (ms.max(0) / PERIOD_MS) as usize
}

/// Where frame 0 falls on the recording's clock. The movie ends when the
/// recording does; its head can lag the clock's start.
pub fn offset(duration_ms: i64, count: usize) -> i64 {
    duration_ms - count as i64 * PERIOD_MS
}

fn read_within(movie_ms: i64) -> Duration {
    // Decoding must run at least twice as fast as the movie plays.
    READ_BASE + Duration::from_millis(movie_ms.max(0) as u64 / 2)
}

pub struct Sampled {
    /// `(t_ms, path)` on the recording's clock.
    pub candidates: Vec<(i64, PathBuf)>,
    pub movie: Movie,
}

/// What finalizing needs to read the same frames again.
#[derive(Clone, Debug)]
pub struct Movie {
    pub video: PathBuf,
    pub offset: i64,
    pub count: usize,
    pub size: (u32, u32),
    pub episodes: Vec<(i64, i64)>,
}

fn write_jpeg(path: &Path, (width, height): (u32, u32), rgba: &[u8]) -> Result<(), String> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or("a frame of the wrong size")?;
    let rgb = image::DynamicImage::ImageRgba8(image).to_rgb8();
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 80)
        .encode_image(&rgb)
        .map_err(|error| error.to_string())
}

/// Writes `folder/{t_ms:08}.jpg` candidates of the movie, which ran
/// `duration_ms` on the recording's clock.
pub fn sample(
    decoder: &dyn Decoder,
    video: &Path,
    folder: &Path,
    duration_ms: i64,
    episodes: Vec<(i64, i64)>,
) -> Result<Sampled, String> {
    let size = decoder.size(video)?;
    let small = fit(size, CANDIDATE_EDGE);
    crate::session::ensure_root(folder).map_err(|error| error.to_string())?;
    let pending = |k: usize| folder.join(format!(".frame-{k:06}.jpg"));
    let mut k = 0;
    let decoded = decoder.frames(video, small, read_within(duration_ms), &mut |rgba| {
        if let Err(error) = write_jpeg(&pending(k), small, rgba) {
            tracing::warn!(%error, "annotate candidate frame");
        }
        k += 1;
        true
    })?;
    let count = decoded.count;
    // Without the whole movie its end is unknown: start-aligned instead.
    let offset = if decoded.complete {
        offset(duration_ms, count)
    } else {
        0
    };
    let time = |k: usize| offset + k as i64 * PERIOD_MS;
    let source: Vec<usize> = (0..count)
        .map(|k| {
            let t = time(k);
            let shown = clean(t, &episodes);
            if shown == t {
                k
            } else {
                index(shown - offset).min(k)
            }
        })
        .collect();
    let used: HashSet<usize> = source.iter().copied().collect();
    let name = |k: usize| folder.join(format!("{:08}.jpg", time(k)));
    for k in 0..count {
        if used.contains(&k) {
            let _ = std::fs::rename(pending(k), name(k));
        } else {
            let _ = std::fs::remove_file(pending(k));
        }
    }
    let candidates = (0..count)
        .filter(|&k| time(k) >= 0)
        .map(|k| (time(k), name(source[k])))
        .filter(|(_, path)| path.is_file())
        .collect();
    Ok(Sampled {
        candidates,
        movie: Movie {
            video: video.to_path_buf(),
            offset,
            count,
            size,
            episodes,
        },
    })
}

#[derive(Deserialize)]
struct ExtractRequest {
    video: PathBuf,
    #[serde(default)]
    video_offset_ms: Option<i64>,
    times_ms: Vec<i64>,
    dir: PathBuf,
}

/// The contract's `extract_frames`, with what `stop` learnt of the movie
/// when it is the one asked for.
pub fn extract(
    decoder: &dyn Decoder,
    request: &str,
    known: Option<&Movie>,
) -> Result<String, AnnotateError> {
    let request: ExtractRequest = super::parse(request)?;
    if !request.video.is_absolute() || !request.dir.is_absolute() {
        return Err(fail(
            codes::INVALID_ARGUMENT,
            "video and dir must be absolute paths",
        ));
    }
    if !request.video.is_file() {
        return Err(fail(
            codes::INVALID_ARGUMENT,
            format!("{} does not exist", request.video.display()),
        ));
    }
    let known = known.filter(|movie| movie.video == request.video);
    let offset = known.map_or(request.video_offset_ms.unwrap_or(0), |movie| movie.offset);
    let episodes = known.map_or(&[][..], |movie| &movie.episodes[..]);
    let size = match known {
        Some(movie) => movie.size,
        None => decoder
            .size(&request.video)
            .map_err(|error| fail(codes::EXTRACT, error))?,
    };
    let size = fit(size, FRAME_EDGE);
    let wanted: Vec<(i64, usize)> = request
        .times_ms
        .iter()
        .map(|&t| {
            let at = index(clean(t, episodes) - offset);
            (
                t,
                known.map_or(at, |movie| at.min(movie.count.saturating_sub(1))),
            )
        })
        .collect();
    let Some(last) = wanted.iter().map(|&(_, at)| at).max() else {
        return Ok(json!({ "frames": [], "failed": [] }).to_string());
    };
    std::fs::create_dir_all(&request.dir).map_err(|error| fail(codes::IO, error))?;
    let mut frames = Vec::new();
    let mut failed = Vec::new();
    let mut k = 0;
    decoder
        .frames(
            &request.video,
            size,
            read_within(last as i64 * PERIOD_MS),
            &mut |rgba| {
                for &(t, _) in wanted.iter().filter(|(_, at)| *at == k) {
                    let path = request.dir.join(format!("frame-{t}.png"));
                    let frame =
                        anlg_annotate_core::RgbaFrame::new(0, size.0, size.1, rgba.to_vec());
                    let written = anlg_annotate_core::encode_png(&frame)
                        .map_err(|error| error.to_string())
                        .and_then(|png| std::fs::write(&path, png).map_err(|e| e.to_string()));
                    let video_ms = k as i64 * PERIOD_MS;
                    match written {
                        Ok(()) => frames.push(json!({
                            "t_ms": t, "video_ms": video_ms, "actual_ms": video_ms, "path": path,
                        })),
                        Err(message) => failed.push(json!({ "t_ms": t, "message": message })),
                    }
                }
                k += 1;
                k <= last
            },
        )
        .map_err(|error| fail(codes::EXTRACT, error))?;
    for &(t, _) in wanted.iter().filter(|(_, at)| *at >= k) {
        failed.push(json!({ "t_ms": t, "message": "the movie ended before this time" }));
    }
    if frames.is_empty() {
        let message = failed
            .first()
            .and_then(|failure| failure["message"].as_str())
            .unwrap_or("no frame could be read")
            .to_owned();
        return Err(fail(codes::EXTRACT, message));
    }
    Ok(json!({ "frames": frames, "failed": failed }).to_string())
}

/// `gst-launch-1.0` arguments: the movie on `input` as raw RGBA frames at
/// `size`, `PERIOD_MS` apart, on `output`. The rate drops frames before
/// they are converted.
pub fn frames_args(input: RawFd, output: RawFd, (width, height): (u32, u32)) -> Vec<String> {
    [
        "-q".to_owned(),
        "fdsrc".to_owned(),
        format!("fd={input}"),
        "!".to_owned(),
        "decodebin".to_owned(),
        "!".to_owned(),
        "videorate".to_owned(),
        "!".to_owned(),
        format!("video/x-raw,framerate={}/1", 1000 / PERIOD_MS),
        "!".to_owned(),
        "videoconvert".to_owned(),
        "!".to_owned(),
        "videoscale".to_owned(),
        "!".to_owned(),
        format!("video/x-raw,format=RGBA,width={width},height={height},pixel-aspect-ratio=1/1"),
        "!".to_owned(),
        "fdsink".to_owned(),
        format!("fd={output}"),
    ]
    .into()
}

pub struct Gstreamer(pub Arc<dyn Gst>);

fn missing() -> String {
    "Reading the recording back needs GStreamer's gst-launch-1.0 and a decoder for the movie: on Fedora `sudo dnf install gstreamer1 gstreamer1-plugins-base gstreamer1-plugin-openh264`, on Debian or Ubuntu `sudo apt install gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-libav`.".into()
}

impl Gstreamer {
    fn launch(
        &self,
        video: &Path,
        args: impl FnOnce(RawFd, RawFd) -> Vec<String>,
    ) -> Result<
        (
            Box<dyn tauri_plugin_capture::recorder::Process>,
            std::fs::File,
        ),
        String,
    > {
        if !self.0.installed() {
            return Err(missing());
        }
        let input = std::fs::File::open(video).map_err(|error| error.to_string())?;
        let (read, write) = pipe().map_err(|error| error.to_string())?;
        let args = args(input.as_raw_fd(), write.as_raw_fd());
        let process = self
            .0
            .launch(&args, &[input.as_raw_fd(), write.as_raw_fd()])
            .map_err(|error| format!("could not start GStreamer: {error}"))?;
        Ok((process, std::fs::File::from(read)))
    }
}

impl Decoder for Gstreamer {
    fn size(&self, video: &Path) -> Result<(u32, u32), String> {
        let (mut process, output) = self.launch(video, first_frame_args)?;
        let reader = std::thread::spawn(move || {
            let mut png = Vec::new();
            let _ = output.take(PNG_LIMIT).read_to_end(&mut png);
            png
        });
        if wait(process.as_mut(), SIZE_WITHIN).is_none() {
            process.kill();
        }
        let png = reader.join().unwrap_or_default();
        image::ImageReader::new(std::io::Cursor::new(png))
            .with_guessed_format()
            .ok()
            .and_then(|reader| reader.into_dimensions().ok())
            .filter(|&(width, height)| width > 0 && height > 0)
            .ok_or_else(|| {
                format!(
                    "GStreamer could not read the recording: {} {}",
                    process.errors(),
                    missing()
                )
            })
    }

    fn frames(
        &self,
        video: &Path,
        size: (u32, u32),
        within: Duration,
        frame: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<Decoded, String> {
        let (mut process, mut output) =
            self.launch(video, |input, out| frames_args(input, out, size))?;
        let len = size.0 as usize * size.1 as usize * 4;
        let (send, frames) = sync_channel::<Vec<u8>>(2);
        let reader = std::thread::spawn(move || {
            loop {
                let mut buffer = vec![0; len];
                if output.read_exact(&mut buffer).is_err() || send.send(buffer).is_err() {
                    return;
                }
            }
        });
        let deadline = Instant::now() + within;
        let mut count = 0;
        let mut complete = true;
        loop {
            match frames.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(buffer) => {
                    count += 1;
                    if !frame(&buffer) {
                        complete = false;
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    tracing::warn!("annotate: reading the movie took too long");
                    complete = false;
                    break;
                }
            }
        }
        drop(frames);
        if complete {
            complete = wait(process.as_mut(), Duration::from_secs(5)) == Some(true);
        }
        process.kill();
        let _ = reader.join();
        if count == 0 {
            return Err(format!(
                "GStreamer read no frame of the recording: {}",
                process.errors()
            ));
        }
        Ok(Decoded { count, complete })
    }
}
