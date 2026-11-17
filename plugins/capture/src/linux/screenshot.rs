//! Screenshots on Linux, through the desktop's Screenshot portal: full
//! screen without asking, an area or a window through the desktop's own
//! picker (GNOME's and KDE's let the user choose any of the three). The
//! portal hands back a file; it is re-encoded where the plugin asked, with
//! its thumbnail, and the portal's own copy is removed when it was plainly
//! made for this request.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anlg_annotate_macos::{AnnotateError, codes};
use image::DynamicImage;
use serde::Deserialize;

use super::fail;
use super::media;

/// The Screenshot portal.
pub trait Shots: Send + Sync {
    /// The picture's file, or `None` when the user cancelled the picker.
    fn grab(&self, interactive: bool) -> Result<Option<PathBuf>, AnnotateError>;
}

/// Where a portal's file may be removed from: the temporary folder and
/// the user's Pictures folder (GNOME saves its screenshots in
/// `~/Pictures/Screenshots`).
#[derive(Clone, Debug, Default)]
pub struct Leftovers {
    pub temp: Option<PathBuf>,
    pub pictures: Option<PathBuf>,
}

/// A second of slack for file systems that keep coarse times.
const SLACK: std::time::Duration = std::time::Duration::from_secs(2);

impl Leftovers {
    /// Whether the portal's `file` is a copy made for a request that began
    /// at `asked`: written since then, in the temporary or Pictures folder.
    /// Anything else (an older file, one elsewhere) stays.
    pub fn made_for_us(&self, file: &Path, asked: SystemTime) -> bool {
        let fresh = std::fs::metadata(file)
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified + SLACK >= asked);
        let inside = |dir: &Option<PathBuf>| {
            dir.as_ref()
                .is_some_and(|dir| file.starts_with(dir) && file != dir.as_path())
        };
        fresh && file.is_file() && (inside(&self.temp) || inside(&self.pictures))
    }
}

/// Takes a picture through the portal; `None` when cancelled. The portal's
/// copy is removed when [`Leftovers::made_for_us`] says so.
pub fn grab(
    shots: &dyn Shots,
    leftovers: &Leftovers,
    interactive: bool,
) -> Result<Option<DynamicImage>, AnnotateError> {
    let asked = SystemTime::now();
    let Some(file) = shots.grab(interactive)? else {
        return Ok(None);
    };
    let picture = image::open(&file).map_err(|error| {
        fail(
            codes::IO,
            format!("could not read the desktop's screenshot: {error}"),
        )
    });
    if leftovers.made_for_us(&file, asked) {
        let _ = std::fs::remove_file(&file);
    }
    picture.map(Some)
}

#[derive(Deserialize)]
struct Request {
    mode: String,
    out_path: PathBuf,
    format: String,
    #[serde(default = "quality")]
    jpeg_quality: f64,
    thumbnail_path: PathBuf,
    #[serde(default = "long_edge")]
    thumbnail_long_edge: u32,
}

fn quality() -> f64 {
    0.9
}

fn long_edge() -> u32 {
    anlg_capture_core::layout::THUMBNAIL_LONG_EDGE
}

/// The contract's `screenshot`. `show_cursor` and `labels` are not used:
/// the desktop's picker has its own pointer switch and words.
pub fn take(
    shots: &dyn Shots,
    leftovers: &Leftovers,
    request: &str,
) -> Result<String, AnnotateError> {
    let request: Request = super::parse(request)?;
    let interactive = request.mode != "fullscreen";
    let Some(picture) = grab(shots, leftovers, interactive)? else {
        return Ok(r#"{"status":"cancelled"}"#.into());
    };
    let jpeg = (request.format == "jpeg").then_some(request.jpeg_quality);
    media::save(&picture, &request.out_path, jpeg)?;
    if let Err(error) = media::write_thumbnail(
        &picture,
        &request.thumbnail_path,
        request.thumbnail_long_edge.max(16),
    ) {
        tracing::warn!(%error, "capture thumbnail");
    }
    Ok(serde_json::json!({
        "status": "captured",
        "path": request.out_path,
        "thumbnail_path": request.thumbnail_path,
        "width_px": picture.width(),
        "height_px": picture.height(),
        "scale": 1.0,
        "kind": request.mode,
        "display_id": 0,
        "rect": { "x": 0, "y": 0, "width": 0, "height": 0 },
        "app": null,
        "window_title": null,
    })
    .to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use super::*;

    /// The portal: writes a picture into `dir` (as GNOME writes into
    /// Pictures), or cancels, or fails, as scripted.
    pub struct FakeShots {
        pub dir: PathBuf,
        pub answer: Mutex<Result<bool, AnnotateError>>,
        pub asked: Mutex<Vec<bool>>,
    }

    impl FakeShots {
        pub fn new(dir: &Path) -> Self {
            Self {
                dir: dir.to_path_buf(),
                answer: Mutex::new(Ok(true)),
                asked: Mutex::default(),
            }
        }
    }

    impl Shots for FakeShots {
        fn grab(&self, interactive: bool) -> Result<Option<PathBuf>, AnnotateError> {
            self.asked.lock().unwrap().push(interactive);
            match self.answer.lock().unwrap().clone()? {
                false => Ok(None),
                true => {
                    let file = self.dir.join("Screenshot from 2026-11-14 14-03-22.png");
                    image::RgbaImage::from_pixel(64, 32, image::Rgba([0, 120, 255, 255]))
                        .save(&file)
                        .unwrap();
                    Ok(Some(file))
                }
            }
        }
    }

    #[test]
    fn a_portal_screenshot_lands_at_out_path_in_the_asked_format_or_is_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let pictures = dir.path().join("Pictures/Screenshots");
        std::fs::create_dir_all(&pictures).unwrap();
        let shots = FakeShots::new(&pictures);
        let leftovers = Leftovers {
            temp: None,
            pictures: Some(dir.path().join("Pictures")),
        };
        let out = dir.path().join(".capture-1.jpg");
        let thumbnail = dir.path().join(".capture-2.png");
        let request = |mode: &str| {
            serde_json::json!({
                "mode": mode, "out_path": out, "format": "jpeg", "jpeg_quality": 0.8,
                "thumbnail_path": thumbnail, "thumbnail_long_edge": 16,
                "show_cursor": true, "labels": {},
            })
            .to_string()
        };
        let reply: serde_json::Value =
            serde_json::from_str(&take(&shots, &leftovers, &request("area")).unwrap()).unwrap();
        assert_eq!(reply["status"], "captured");
        assert_eq!(
            (reply["width_px"].as_u64(), reply["height_px"].as_u64()),
            (Some(64), Some(32))
        );
        assert_eq!(reply["kind"], "area");
        assert_eq!(
            image::ImageFormat::from_path(&out).unwrap(),
            image::ImageFormat::Jpeg
        );
        assert_eq!(&std::fs::read(&out).unwrap()[..2], b"\xff\xd8");
        assert_eq!(image::open(&thumbnail).unwrap().width(), 16);
        // GNOME's copy in Pictures was made for this request: gone.
        assert_eq!(std::fs::read_dir(&pictures).unwrap().count(), 0);

        take(&shots, &leftovers, &request("fullscreen")).unwrap();
        assert_eq!(*shots.asked.lock().unwrap(), [true, false]);

        *shots.answer.lock().unwrap() = Ok(false);
        assert_eq!(
            take(&shots, &leftovers, &request("window")).unwrap(),
            r#"{"status":"cancelled"}"#
        );
        *shots.answer.lock().unwrap() = Err(fail(codes::INTERNAL, "the portal is not running"));
        assert_eq!(
            take(&shots, &leftovers, &request("window"))
                .unwrap_err()
                .code,
            codes::INTERNAL
        );
    }

    #[test]
    fn only_a_fresh_portal_file_in_temp_or_pictures_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let pictures = dir.path().join("Pictures");
        let elsewhere = dir.path().join("Documents");
        std::fs::create_dir_all(&pictures).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let leftovers = Leftovers {
            temp: None,
            pictures: Some(pictures.clone()),
        };
        let asked = SystemTime::now();
        let fresh = pictures.join("new.png");
        let old = pictures.join("old.png");
        let other = elsewhere.join("new.png");
        for file in [&fresh, &old, &other] {
            std::fs::write(file, b"png").unwrap();
        }
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(asked - std::time::Duration::from_secs(60))
            .unwrap();
        assert!(leftovers.made_for_us(&fresh, asked));
        assert!(!leftovers.made_for_us(&old, asked));
        assert!(!leftovers.made_for_us(&other, asked));
    }
}
