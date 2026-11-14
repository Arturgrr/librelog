//! Pictures, thumbnails and GIFs on Linux: the `image` crate for pictures,
//! GStreamer pipelines for movies.

use std::io::{Cursor, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anlg_annotate_macos::{AnnotateError, codes};
use anlg_capture_core::fs::write_atomic;
use image::DynamicImage;
use serde::Deserialize;

use super::fail;
use super::gif::{self, Plan};
use super::gst::{self, Gst};

const FIRST_FRAME_WITHIN: Duration = Duration::from_secs(30);
/// A 30-minute recording is decoded through in well under this.
const GIF_WITHIN: Duration = Duration::from_secs(20 * 60);
const MAX_FRAME_PNG: u64 = 256 * 1024 * 1024;

fn io(error: impl std::fmt::Display) -> AnnotateError {
    fail(codes::IO, error)
}

/// Writes `picture` as PNG or JPEG (quality 0–1) at `out`.
pub fn save(picture: &DynamicImage, out: &Path, jpeg: Option<f64>) -> Result<(), AnnotateError> {
    let mut bytes = Cursor::new(Vec::new());
    match jpeg {
        Some(quality) => {
            let quality = (quality.clamp(0.05, 1.0) * 100.0).round() as u8;
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality);
            picture.to_rgb8().write_with_encoder(encoder).map_err(io)?;
        }
        None => picture
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(io)?,
    }
    write_atomic(out, bytes.get_ref(), false).map_err(io)
}

/// A PNG of `picture` no longer than `long_edge` at `out`; its size.
pub fn write_thumbnail(
    picture: &DynamicImage,
    out: &Path,
    long_edge: u32,
) -> Result<(u32, u32), AnnotateError> {
    let small = if picture.width().max(picture.height()) > long_edge {
        picture.thumbnail(long_edge, long_edge)
    } else {
        picture.clone()
    };
    let mut bytes = Cursor::new(Vec::new());
    small
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(io)?;
    write_atomic(out, bytes.get_ref(), true).map_err(io)?;
    Ok((small.width(), small.height()))
}

/// The movie's first frame, through GStreamer.
pub fn first_frame(gst: &dyn Gst, movie: &Path) -> Result<DynamicImage, AnnotateError> {
    let input = std::fs::File::open(movie).map_err(io)?;
    let (read, write) = gst::pipe().map_err(io)?;
    let args = gst::first_frame_args(input.as_raw_fd(), write.as_raw_fd());
    let reader = std::thread::spawn(move || {
        let mut png = Vec::new();
        let read = std::fs::File::from(read)
            .take(MAX_FRAME_PNG)
            .read_to_end(&mut png);
        read.map(|_| png)
    });
    let ran = {
        let inherit = [input.as_raw_fd(), write.as_raw_fd()];
        let ran = gst::run(gst, &args, &inherit, FIRST_FRAME_WITHIN);
        // Ours closes so the reader sees the end once the child is gone.
        drop(write);
        ran
    };
    let png = reader
        .join()
        .map_err(|_| fail(codes::INTERNAL, "the frame reader stopped"))?
        .map_err(io)?;
    ran.map_err(|error| fail(codes::EXTRACT, error))?;
    image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|error| {
        fail(
            codes::EXTRACT,
            format!("no frame could be read from the movie: {error}"),
        )
    })
}

fn open(gst: &dyn Gst, source: &Path) -> Result<DynamicImage, AnnotateError> {
    if gst::is_movie(source) {
        first_frame(gst, source)
    } else {
        // A GIF opens at its first frame.
        image::open(source).map_err(io)
    }
}

#[derive(Deserialize)]
struct ThumbnailRequest {
    source: PathBuf,
    out_path: PathBuf,
    #[serde(default = "long_edge")]
    long_edge: u32,
}

fn long_edge() -> u32 {
    anlg_capture_core::layout::THUMBNAIL_LONG_EDGE
}

pub fn thumbnail(gst: &dyn Gst, request: &str) -> Result<String, AnnotateError> {
    let request: ThumbnailRequest = super::parse(request)?;
    let picture = open(gst, &request.source)?;
    let (width, height) = write_thumbnail(&picture, &request.out_path, request.long_edge.max(16))?;
    Ok(serde_json::json!({
        "path": request.out_path, "width_px": width, "height_px": height,
    })
    .to_string())
}

#[derive(Deserialize)]
struct GifRequest {
    video: PathBuf,
    out_path: PathBuf,
    fps: u32,
    max_width: u32,
    max_frames: u32,
}

/// Converts the movie to a GIF at `out_path`. `duration_ms` is the
/// recording's length when known; else the GIF keeps the first
/// `max_frames` frames at the asked fps.
pub fn make_gif(
    gst: &dyn Gst,
    request: &str,
    duration_ms: Option<u64>,
) -> Result<String, AnnotateError> {
    let request: GifRequest = super::parse(request)?;
    if !gst.installed() {
        return Err(fail(codes::INTERNAL, gst::missing_launcher()));
    }
    // The first frame tells the real pixel size.
    let first = first_frame(gst, &request.video)?;
    let fps = request.fps.clamp(1, 50);
    let duration_ms =
        duration_ms.unwrap_or_else(|| u64::from(request.max_frames.max(1)) * 1000 / u64::from(fps));
    let plan = gif::plan(
        duration_ms,
        (first.width(), first.height()),
        fps,
        request.max_width,
        request.max_frames,
    );
    let frames = encode_movie(gst, &request.video, &request.out_path, &plan)?;
    if frames == 0 {
        let _ = std::fs::remove_file(&request.out_path);
        return Err(fail(
            codes::EXTRACT,
            "no frame could be read from the movie",
        ));
    }
    let bytes = std::fs::metadata(&request.out_path).map_or(0, |metadata| metadata.len());
    Ok(serde_json::json!({
        "path": request.out_path, "width_px": plan.width, "height_px": plan.height,
        "frames": frames, "fps": plan.fps, "bytes": bytes,
    })
    .to_string())
}

fn encode_movie(
    gst: &dyn Gst,
    movie: &Path,
    out: &Path,
    plan: &Plan,
) -> Result<u32, AnnotateError> {
    let input = std::fs::File::open(movie).map_err(io)?;
    let (read, write) = gst::pipe().map_err(io)?;
    let fps_milli = (plan.fps * 1000.0).round().max(1.0) as u32;
    let args = gst::frames_args(
        input.as_raw_fd(),
        write.as_raw_fd(),
        plan.width,
        plan.height,
        fps_milli,
    );
    let process = gst
        .launch(&args, &[input.as_raw_fd(), write.as_raw_fd()])
        .map_err(|error| {
            fail(
                codes::INTERNAL,
                format!("could not start {}: {error}", gst::LAUNCH),
            )
        })?;
    drop(write);
    drop(input);
    let process = std::sync::Arc::new(std::sync::Mutex::new(process));
    // A pipeline that hangs is killed, which ends the reading below.
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let watchdog = {
        let process = process.clone();
        std::thread::spawn(move || {
            if finished.recv_timeout(GIF_WITHIN).is_err() {
                process.lock().unwrap_or_else(|e| e.into_inner()).kill();
            }
        })
    };
    let file = std::fs::File::create(out).map_err(io)?;
    let mut frames = std::io::BufReader::with_capacity(1 << 20, std::fs::File::from(read));
    let encoded = gif::encode(&mut frames, std::io::BufWriter::new(file), plan);
    let _ = done.send(());
    let _ = watchdog.join();
    let mut process = process.lock().unwrap_or_else(|e| e.into_inner());
    // Done with what it decoded: the rest of the movie is not needed.
    let succeeded = process.try_wait();
    process.kill();
    let written = encoded.map_err(|error| fail(codes::IO, error))?;
    if written == 0 && succeeded != Some(true) {
        return Err(fail(
            codes::EXTRACT,
            format!("GStreamer could not decode the movie: {}", process.errors()),
        ));
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_of_a_picture_or_gif_is_a_small_png() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("shot.jpg");
        image::RgbImage::from_pixel(1200, 600, image::Rgb([1, 2, 3]))
            .save(&source)
            .unwrap();
        let out = dir.path().join("thumb.png");
        let request = serde_json::json!({ "source": source, "out_path": out, "long_edge": 480 });
        struct NoGst;
        impl Gst for NoGst {
            fn installed(&self) -> bool {
                false
            }
            fn has(&self, _: &str) -> bool {
                false
            }
            fn launch(&self, _: &[String], _: &[i32]) -> std::io::Result<Box<dyn gst::Process>> {
                Err(std::io::Error::other("no GStreamer"))
            }
        }
        let reply: serde_json::Value =
            serde_json::from_str(&thumbnail(&NoGst, &request.to_string()).unwrap()).unwrap();
        assert_eq!(
            (reply["width_px"].as_u64(), reply["height_px"].as_u64()),
            (Some(480), Some(240))
        );
        assert_eq!(image::open(&out).unwrap().width(), 480);

        // A movie without GStreamer says what to install.
        let movie = dir.path().join("a.webm");
        std::fs::write(&movie, b"webm").unwrap();
        let request = serde_json::json!({ "source": movie, "out_path": out });
        let error = thumbnail(&NoGst, &request.to_string()).unwrap_err();
        assert!(error.message.contains("gst-launch-1.0"), "{error}");
    }
}
