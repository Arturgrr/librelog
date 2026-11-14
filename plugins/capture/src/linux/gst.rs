//! GStreamer, run as `gst-launch-1.0` children: which encoder a recording
//! uses, the pipelines' arguments, and the child processes themselves.
//! Files and pipes reach a pipeline as inherited file descriptors
//! (`fdsrc fd=…`, `fdsink fd=…`), never as paths, so no file name is ever
//! parsed as pipeline syntax.

use std::io::Read;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const LAUNCH: &str = "gst-launch-1.0";
const INSPECT: &str = "gst-inspect-1.0";
/// The last bytes of a child's stderr kept for its error message.
const STDERR_TAIL: usize = 4096;

/// What to install, for messages about a missing piece.
pub const INSTALL_BASE: &str = "Install GStreamer and its PipeWire plugin: on Fedora `sudo dnf install gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good pipewire-gstreamer`, on Debian or Ubuntu `sudo apt install gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-pipewire`.";

/// The video encoders a recording can use, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoder {
    /// H.264 on the GPU (VA-API).
    VaH264,
    OpenH264,
    X264,
    Vp9,
    Vp8,
}

pub const ENCODERS: [Encoder; 5] = [
    Encoder::VaH264,
    Encoder::OpenH264,
    Encoder::X264,
    Encoder::Vp9,
    Encoder::Vp8,
];

/// The elements every recording needs, whatever it is encoded with.
pub const RECORD_ELEMENTS: [&str; 5] = [
    "pipewiresrc",
    "videorate",
    "videoconvert",
    "videoscale",
    "fdsink",
];

impl Encoder {
    pub fn is_h264(self) -> bool {
        matches!(self, Self::VaH264 | Self::OpenH264 | Self::X264)
    }

    /// `mp4` for H.264, `webm` otherwise.
    pub fn extension(self) -> &'static str {
        if self.is_h264() { "mp4" } else { "webm" }
    }

    /// The elements this encoder's part of the pipeline needs.
    pub fn elements(self) -> [&'static str; 3] {
        match self {
            Self::VaH264 => ["vah264enc", "h264parse", "mp4mux"],
            Self::OpenH264 => ["openh264enc", "h264parse", "mp4mux"],
            Self::X264 => ["x264enc", "h264parse", "mp4mux"],
            Self::Vp9 => ["vp9enc", "webmmux", "webmmux"],
            Self::Vp8 => ["vp8enc", "webmmux", "webmmux"],
        }
    }

    /// From raw video to a muxed stream: the encoder's input format, the
    /// encoder with real-time settings, its parser and the muxer. The MP4 is
    /// fragmented, so a recording whose pipeline died stays playable.
    fn chain(self) -> Vec<String> {
        let (format, encoder): (&str, &[&str]) = match self {
            Self::VaH264 => ("NV12", &["vah264enc"]),
            Self::OpenH264 => (
                "I420",
                &["openh264enc", "bitrate=8000000", "complexity=low"],
            ),
            Self::X264 => (
                "I420",
                &[
                    "x264enc",
                    "tune=zerolatency",
                    "speed-preset=veryfast",
                    "bitrate=8000",
                    "key-int-max=60",
                ],
            ),
            Self::Vp9 => (
                "I420",
                &[
                    "vp9enc",
                    "deadline=1",
                    "cpu-used=8",
                    "threads=4",
                    "target-bitrate=6000000",
                ],
            ),
            Self::Vp8 => (
                "I420",
                &[
                    "vp8enc",
                    "deadline=1",
                    "cpu-used=8",
                    "threads=4",
                    "target-bitrate=6000000",
                ],
            ),
        };
        let mut chain = vec![format!("video/x-raw,format={format}"), "!".to_owned()];
        chain.extend(encoder.iter().map(|part| (*part).to_owned()));
        if self.is_h264() {
            chain.extend(
                ["!", "h264parse", "!", "mp4mux", "fragment-duration=1000"].map(String::from),
            );
        } else {
            chain.extend(["!", "webmmux"].map(String::from));
        }
        chain
    }
}

/// The encoders usable with these elements, best first; or what to
/// install when there is none, or when a recording cannot run at all.
pub fn encoders(has: impl Fn(&str) -> bool) -> Result<Vec<Encoder>, String> {
    if !has("pipewiresrc") {
        return Err("Screen recording needs GStreamer's PipeWire plugin: on Fedora `sudo dnf install pipewire-gstreamer`, on Debian or Ubuntu `sudo apt install gstreamer1.0-pipewire`.".into());
    }
    if let Some(missing) = RECORD_ELEMENTS.iter().find(|element| !has(element)) {
        return Err(format!(
            "Screen recording needs the GStreamer element {missing}. {INSTALL_BASE}"
        ));
    }
    let usable: Vec<Encoder> = ENCODERS
        .into_iter()
        .filter(|encoder| encoder.elements().iter().all(|element| has(element)))
        .collect();
    if usable.is_empty() {
        return Err("Screen recording found no video encoder in GStreamer. For MP4 install H.264 (on Fedora `sudo dnf install gstreamer1-plugin-openh264 gstreamer1-plugins-bad-free`, on Debian or Ubuntu `sudo apt install gstreamer1.0-plugins-bad`), or for WebM install VP8/VP9 (`gstreamer1-plugins-good` on Fedora, `gstreamer1.0-plugins-good` on Debian or Ubuntu).".into());
    }
    Ok(usable)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// What a recording keeps of the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// All of it, scaled to even sizes for H.264.
    Whole,
    /// Frames scaled to `frame` (the size the selection was made on), then
    /// cropped to `rect`.
    Crop { frame: (u32, u32), rect: Rect },
}

/// One recording's pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recording {
    /// The PipeWire remote the portal opened.
    pub remote: RawFd,
    pub node: u32,
    pub fps: u32,
    pub shape: Shape,
    pub encoder: Encoder,
    /// The movie file, opened for writing.
    pub out: RawFd,
}

/// `gst-launch-1.0` arguments for a recording. `-e` turns SIGINT into an
/// end of stream, which finishes the file.
pub fn record_args(recording: &Recording) -> Vec<String> {
    let mut args: Vec<String> = [
        "-q".to_owned(),
        "-e".to_owned(),
        "pipewiresrc".to_owned(),
        format!("fd={}", recording.remote),
        format!("path={}", recording.node),
        "do-timestamp=true".to_owned(),
        "keepalive-time=1000".to_owned(),
        "!".to_owned(),
        "videorate".to_owned(),
        "!".to_owned(),
        format!("video/x-raw,framerate={}/1", recording.fps.clamp(1, 60)),
        "!".to_owned(),
        "videoconvert".to_owned(),
        "!".to_owned(),
        "videoscale".to_owned(),
        "!".to_owned(),
    ]
    .into();
    match recording.shape {
        Shape::Whole => {
            // H.264 takes even sizes only: the nearest even size.
            args.push(if recording.encoder.is_h264() {
                "video/x-raw,width=(int)[16,16384,2],height=(int)[16,16384,2],pixel-aspect-ratio=1/1"
                    .into()
            } else {
                "video/x-raw,pixel-aspect-ratio=1/1".into()
            });
        }
        Shape::Crop { frame, rect } => {
            let (width, height) = frame;
            args.push(format!(
                "video/x-raw,width={width},height={height},pixel-aspect-ratio=1/1"
            ));
            args.push("!".into());
            args.push("videocrop".into());
            args.push(format!("left={}", rect.x));
            args.push(format!("top={}", rect.y));
            args.push(format!(
                "right={}",
                width.saturating_sub(rect.x + rect.width)
            ));
            args.push(format!(
                "bottom={}",
                height.saturating_sub(rect.y + rect.height)
            ));
        }
    }
    args.push("!".into());
    args.push("videoconvert".into());
    args.push("!".into());
    args.extend(recording.encoder.chain());
    args.push("!".into());
    args.push("fdsink".into());
    args.push(format!("fd={}", recording.out));
    args
}

/// Frames of the movie on `input` as raw RGBA, `width` × `height` at
/// `fps_milli` / 1000 frames a second, written to `output`.
pub fn frames_args(
    input: RawFd,
    output: RawFd,
    width: u32,
    height: u32,
    fps_milli: u32,
) -> Vec<String> {
    [
        "-q".to_owned(),
        "fdsrc".to_owned(),
        format!("fd={input}"),
        "!".to_owned(),
        "decodebin".to_owned(),
        "!".to_owned(),
        "videoconvert".to_owned(),
        "!".to_owned(),
        "videorate".to_owned(),
        "!".to_owned(),
        "videoscale".to_owned(),
        "!".to_owned(),
        format!(
            "video/x-raw,format=RGBA,width={width},height={height},pixel-aspect-ratio=1/1,framerate={}/1000",
            fps_milli.max(1)
        ),
        "!".to_owned(),
        "fdsink".to_owned(),
        format!("fd={output}"),
    ]
    .into()
}

/// The movie on `input`'s first frame as a PNG on `output`.
pub fn first_frame_args(input: RawFd, output: RawFd) -> Vec<String> {
    [
        "-q".to_owned(),
        "fdsrc".to_owned(),
        format!("fd={input}"),
        "!".to_owned(),
        "decodebin".to_owned(),
        "!".to_owned(),
        "videoconvert".to_owned(),
        "!".to_owned(),
        "pngenc".to_owned(),
        "snapshot=true".to_owned(),
        "!".to_owned(),
        "fdsink".to_owned(),
        format!("fd={output}"),
    ]
    .into()
}

/// A running pipeline.
pub trait Process: Send {
    /// `Some(success)` once it has exited.
    fn try_wait(&mut self) -> Option<bool>;
    /// Asks it to finish (SIGINT: with `-e`, an end of stream).
    fn interrupt(&mut self);
    /// Stops it at once and reaps it.
    fn kill(&mut self);
    /// The end of what it wrote to stderr.
    fn errors(&self) -> String;
}

/// GStreamer on this computer.
pub trait Gst: Send + Sync {
    /// Whether `gst-launch-1.0` is installed.
    fn installed(&self) -> bool;
    fn has(&self, element: &str) -> bool;
    /// Starts `gst-launch-1.0 args`, inheriting exactly `inherit` besides
    /// stdin, stdout and stderr.
    fn launch(&self, args: &[String], inherit: &[RawFd]) -> std::io::Result<Box<dyn Process>>;
}

/// Waits up to `within` for `process` to exit by itself.
pub fn wait(process: &mut dyn Process, within: Duration) -> Option<bool> {
    let deadline = Instant::now() + within;
    loop {
        if let Some(success) = process.try_wait() {
            return Some(success);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Asks `process` to finish, waits up to `within`, then kills it.
/// `Some(success)` when it finished by itself.
pub fn finish(process: &mut dyn Process, within: Duration) -> Option<bool> {
    if let Some(done) = process.try_wait() {
        return Some(done);
    }
    process.interrupt();
    let done = wait(process, within);
    if done.is_none() {
        process.kill();
    }
    done
}

pub fn missing_launcher() -> String {
    format!("GStreamer's gst-launch-1.0 is not installed. {INSTALL_BASE}")
}

/// Runs a pipeline to its end, killing it after `within`.
pub fn run(
    gst: &dyn Gst,
    args: &[String],
    inherit: &[RawFd],
    within: Duration,
) -> Result<(), String> {
    if !gst.installed() {
        return Err(missing_launcher());
    }
    let mut process = gst
        .launch(args, inherit)
        .map_err(|error| format!("could not start {LAUNCH}: {error}"))?;
    match wait(process.as_mut(), within) {
        Some(true) => Ok(()),
        Some(false) => Err(format!("GStreamer failed: {}", process.errors())),
        None => {
            process.kill();
            Err(format!(
                "GStreamer did not finish within {} s",
                within.as_secs()
            ))
        }
    }
}

/// The installed tools.
#[derive(Default)]
pub struct Tools {
    known: Mutex<std::collections::HashMap<String, bool>>,
}

fn on_path(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|path| {
                std::fs::metadata(path).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
    })
}

pub fn installed(program: &str) -> bool {
    on_path(program).is_some()
}

impl Gst for Tools {
    fn installed(&self) -> bool {
        installed(LAUNCH)
    }

    fn has(&self, element: &str) -> bool {
        let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(has) = known.get(element) {
            return *has;
        }
        let has = installed(INSPECT)
            && crate::process::run(
                INSPECT,
                &["--exists", element],
                None,
                Duration::from_secs(10),
            )
            .is_ok();
        known.insert(element.to_owned(), has);
        has
    }

    fn launch(&self, args: &[String], inherit: &[RawFd]) -> std::io::Result<Box<dyn Process>> {
        Child::spawn(args, inherit).map(|child| Box::new(child) as Box<dyn Process>)
    }
}

struct Child {
    child: std::process::Child,
    stderr: Arc<Mutex<Vec<u8>>>,
    exited: Option<bool>,
}

/// Marks every descriptor from 3 up close-on-exec, then clears it on the
/// ones to keep. Runs between fork and exec: only async-signal-safe calls.
fn keep_only(inherit: &[RawFd]) -> std::io::Result<()> {
    const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
    let marked = unsafe {
        libc::syscall(
            libc::SYS_close_range,
            3 as libc::c_uint,
            libc::c_uint::MAX,
            CLOSE_RANGE_CLOEXEC,
        )
    } == 0;
    if !marked {
        let mut limit = libc::rlimit {
            rlim_cur: 1024,
            rlim_max: 1024,
        };
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
        let top = limit.rlim_cur.min(65_536) as RawFd;
        for fd in 3..top {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
    }
    for fd in inherit {
        if unsafe { libc::fcntl(*fd, libc::F_SETFD, 0) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

impl Child {
    fn spawn(args: &[String], inherit: &[RawFd]) -> std::io::Result<Self> {
        use std::os::unix::process::CommandExt;
        let program = on_path(LAUNCH)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, missing_launcher()))?;
        let keep = inherit.to_vec();
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            // Its own process group: a Ctrl-C meant for the app's terminal
            // does not cut the movie short.
            .process_group(0);
        unsafe {
            command.pre_exec(move || keep_only(&keep));
        }
        let mut child = command.spawn()?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut pipe) = child.stderr.take() {
            let tail = stderr.clone();
            std::thread::spawn(move || {
                let mut buffer = [0u8; 1024];
                while let Ok(read) = pipe.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                    tail.extend_from_slice(&buffer[..read]);
                    let extra = tail.len().saturating_sub(STDERR_TAIL);
                    tail.drain(..extra);
                }
            });
        }
        Ok(Self {
            child,
            stderr,
            exited: None,
        })
    }
}

impl Process for Child {
    fn try_wait(&mut self) -> Option<bool> {
        if self.exited.is_none() {
            match self.child.try_wait() {
                Ok(Some(status)) => self.exited = Some(status.success()),
                Ok(None) => {}
                Err(_) => self.exited = Some(false),
            }
        }
        self.exited
    }

    fn interrupt(&mut self) {
        // Only a child not yet reaped: its pid is still its own.
        if self.try_wait().is_none()
            && let Ok(pid) = libc::pid_t::try_from(self.child.id())
        {
            unsafe { libc::kill(pid, libc::SIGINT) };
        }
    }

    fn kill(&mut self) {
        if self.try_wait().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.exited = Some(false);
        }
    }

    fn errors(&self) -> String {
        let tail = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&tail).trim().to_owned()
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A pipe whose ends are close-on-exec until a child inherits one.
pub fn pipe() -> std::io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::FromRawFd;
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe {
        (
            std::os::fd::OwnedFd::from_raw_fd(fds[0]),
            std::os::fd::OwnedFd::from_raw_fd(fds[1]),
        )
    })
}

/// Whether `path` names a movie the decoder pipelines read.
pub fn is_movie(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            anlg_capture_core::names::MOVIE_EXTENSIONS
                .iter()
                .any(|movie| extension.eq_ignore_ascii_case(movie))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoder_is_the_best_one_installed_or_a_message_naming_what_to_install() {
        let with = |elements: &[&str]| {
            let elements: Vec<String> = RECORD_ELEMENTS
                .iter()
                .chain(elements)
                .map(|element| element.to_string())
                .collect();
            encoders(move |element| elements.iter().any(|known| known == element))
        };
        let h264 = ["h264parse", "mp4mux"];
        assert_eq!(
            with(&[&["vah264enc", "x264enc", "vp9enc", "webmmux"][..], &h264].concat()).unwrap(),
            vec![Encoder::VaH264, Encoder::X264, Encoder::Vp9]
        );
        assert_eq!(
            with(&[&["openh264enc"][..], &h264].concat()).unwrap(),
            vec![Encoder::OpenH264]
        );
        // H.264 without its parser is no use; VP8 in WebM is.
        assert_eq!(
            with(&["x264enc", "mp4mux", "vp8enc", "webmmux"]).unwrap(),
            vec![Encoder::Vp8]
        );
        let none = with(&["mp4mux"]).unwrap_err();
        assert!(none.contains("gstreamer1-plugin-openh264"), "{none}");
        let no_pipewire = encoders(|element| element != "pipewiresrc").unwrap_err();
        assert!(no_pipewire.contains("pipewire-gstreamer"), "{no_pipewire}");
    }

    #[test]
    fn a_recording_pipeline_reads_the_portals_node_and_writes_only_to_its_descriptor() {
        let whole = Recording {
            remote: 41,
            node: 77,
            fps: 30,
            shape: Shape::Whole,
            encoder: Encoder::OpenH264,
            out: 42,
        };
        let args = record_args(&whole).join(" ");
        assert!(
            args.starts_with("-q -e pipewiresrc fd=41 path=77 do-timestamp=true keepalive-time=1000 ! videorate ! video/x-raw,framerate=30/1 ! videoconvert ! videoscale ! video/x-raw,width=(int)[16,16384,2],height=(int)[16,16384,2]"),
            "{args}"
        );
        assert!(
            args.ends_with("! video/x-raw,format=I420 ! openh264enc bitrate=8000000 complexity=low ! h264parse ! mp4mux fragment-duration=1000 ! fdsink fd=42"),
            "{args}"
        );

        let cropped = Recording {
            fps: 60,
            shape: Shape::Crop {
                frame: (2880, 1800),
                rect: Rect {
                    x: 100,
                    y: 50,
                    width: 640,
                    height: 480,
                },
            },
            encoder: Encoder::Vp9,
            ..whole
        };
        let args = record_args(&cropped).join(" ");
        assert!(args.contains("framerate=60/1"), "{args}");
        assert!(
            args.contains("! video/x-raw,width=2880,height=1800,pixel-aspect-ratio=1/1 ! videocrop left=100 top=50 right=2140 bottom=1270 ! videoconvert ! video/x-raw,format=I420 ! vp9enc deadline=1"),
            "{args}"
        );
        assert!(args.ends_with("! webmmux ! fdsink fd=42"), "{args}");
        assert_eq!(Encoder::Vp9.extension(), "webm");
        assert_eq!(Encoder::X264.extension(), "mp4");
    }
}
