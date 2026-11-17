//! Recording an area on Linux. The ScreenCast portal records a whole
//! monitor (or a window), so the area is chosen first, on a frozen
//! screenshot shown full screen, and the movie is the monitor's stream
//! cropped to it. The crop is only trusted when the stream is plainly the
//! monitor the area was drawn on; otherwise the whole monitor is recorded
//! and the recording says so.

use super::gst::{Rect, Shape};

/// A monitor as the windowing layer reports it, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// Where the selector's monitor is in the frozen screenshot, which shows
/// every monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub rect: Rect,
    /// Whether the region is surely that monitor's.
    pub trusted: bool,
}

/// The part of a `shot`-sized screenshot of all `monitors` that `this`
/// monitor shows. One monitor is the whole picture. With several, the
/// picture must be exactly their bounding box, with the geometry taken as
/// physical pixels, as logical ones (divided by the scale), or (XWayland
/// with scaling reports logical geometry for a physical still) multiplied
/// by the scale; else the whole picture is shown and nothing about it is
/// trusted.
pub fn region(shot: (u32, u32), monitors: &[Monitor], this: Monitor) -> Region {
    let whole = Rect {
        x: 0,
        y: 0,
        width: shot.0,
        height: shot.1,
    };
    if monitors.len() <= 1 {
        return Region {
            rect: whole,
            trusted: true,
        };
    }
    let bounds = |factor: fn(f64) -> f64| {
        let edge = |monitor: &Monitor, value: i32, size: u32| {
            let scale = factor(monitor.scale);
            (
                (f64::from(value) * scale).round() as i64,
                (f64::from(size) * scale).round() as i64,
            )
        };
        let mut left = i64::MAX;
        let mut top = i64::MAX;
        let mut right = i64::MIN;
        let mut bottom = i64::MIN;
        for monitor in monitors {
            let (x, width) = edge(monitor, monitor.x, monitor.width);
            let (y, height) = edge(monitor, monitor.y, monitor.height);
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + width);
            bottom = bottom.max(y + height);
        }
        let (x, width) = edge(&this, this.x, this.width);
        let (y, height) = edge(&this, this.y, this.height);
        (
            (right - left, bottom - top),
            (x - left, y - top, width, height),
        )
    };
    let factors: [fn(f64) -> f64; 3] = [|_| 1.0, |scale| 1.0 / scale, |scale| scale];
    for factor in factors {
        let ((width, height), (x, y, w, h)) = bounds(factor);
        if (width, height) == (i64::from(shot.0), i64::from(shot.1))
            && x >= 0
            && y >= 0
            && x + w <= width
            && y + h <= height
        {
            return Region {
                rect: Rect {
                    x: x as u32,
                    y: y as u32,
                    width: w as u32,
                    height: h as u32,
                },
                trusted: true,
            };
        }
    }
    Region {
        rect: whole,
        trusted: false,
    }
}

/// What the stream the portal started reports, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stream {
    pub node: u32,
    pub size: Option<(i32, i32)>,
    pub position: Option<(i32, i32)>,
}

/// An area drawn on the selector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    /// The selector's monitor in the screenshot.
    pub region: Region,
    /// The area, in screenshot pixels.
    pub rect: Rect,
    pub monitor: Monitor,
    /// How many monitors there are.
    pub monitors: usize,
}

pub const UNTRUSTED: &str =
    "The area could not be matched to the screen being recorded, so the whole screen was recorded.";
const TOLERANCE: i32 = 2;

/// Whether a stream at `position` (the portal's logical pixels) is the
/// monitor `selection` was drawn on, whose geometry may be physical,
/// logical or scaled the other way (XWayland).
pub fn on_monitor(selection: &Selection, position: Option<(i32, i32)>) -> bool {
    let monitor = selection.monitor;
    let scaled = |value: i32, factor: f64| (f64::from(value) * factor).round() as i32;
    position.is_some_and(|(px, py)| {
        [1.0, 1.0 / monitor.scale, monitor.scale]
            .iter()
            .map(|factor| (scaled(monitor.x, *factor), scaled(monitor.y, *factor)))
            .any(|(mx, my)| (px - mx).abs() <= TOLERANCE && (py - my).abs() <= TOLERANCE)
    })
}

/// How to cut `selection` out of `stream`: a crop in the selection's own
/// pixels (frames are scaled to the monitor region first), the whole
/// stream when the area is the whole monitor, or the whole stream with a
/// warning when the mapping cannot be trusted. Crop sizes are even.
pub fn crop(selection: &Selection, stream: &Stream) -> (Shape, Option<&'static str>) {
    let region = selection.region.rect;
    let x = selection.rect.x.clamp(region.x, region.x + region.width) - region.x;
    let y = selection.rect.y.clamp(region.y, region.y + region.height) - region.y;
    let width = selection.rect.width.min(region.width - x) & !1;
    let height = selection.rect.height.min(region.height - y) & !1;
    if width < 16 || height < 16 || (width + 2 >= region.width && height + 2 >= region.height) {
        return (Shape::Whole, None);
    }
    let aspect_matches = stream.size.is_none_or(|(w, h)| {
        w > 0
            && h > 0
            && ((f64::from(w) / f64::from(h))
                - (f64::from(region.width) / f64::from(region.height)))
            .abs()
                < 0.01
    });
    let same_monitor = selection.monitors <= 1 || on_monitor(selection, stream.position);
    if !selection.region.trusted || !aspect_matches || !same_monitor {
        return (Shape::Whole, Some(UNTRUSTED));
    }
    (
        Shape::Crop {
            frame: (region.width, region.height),
            rect: Rect {
                x,
                y,
                width,
                height,
            },
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x: i32, y: i32, width: u32, height: u32, scale: f64) -> Monitor {
        Monitor {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn an_area_maps_onto_its_monitors_stream_or_falls_back_to_the_whole_monitor() {
        // One monitor at scale 2: the screenshot and the selection are in
        // physical pixels, the stream reports logical ones.
        let laptop = monitor(0, 0, 2880, 1800, 2.0);
        let single = Selection {
            region: region((2880, 1800), &[laptop], laptop),
            rect: rect(101, 51, 641, 481),
            monitor: laptop,
            monitors: 1,
        };
        let stream = Stream {
            node: 5,
            size: Some((1440, 900)),
            position: None,
        };
        assert_eq!(
            crop(&single, &stream),
            (
                Shape::Crop {
                    frame: (2880, 1800),
                    rect: rect(101, 51, 640, 480)
                },
                None
            )
        );

        // The second of two monitors, side by side: its part of the shot.
        let left = monitor(0, 0, 1920, 1080, 1.0);
        let right = monitor(1920, 0, 2560, 1440, 1.0);
        let offset = region((4480, 1440), &[left, right], right);
        assert_eq!(
            offset,
            Region {
                rect: rect(1920, 0, 2560, 1440),
                trusted: true
            }
        );
        let selection = Selection {
            region: offset,
            rect: rect(2020, 100, 800, 600),
            monitor: right,
            monitors: 2,
        };
        let on_right = Stream {
            node: 6,
            size: Some((2560, 1440)),
            position: Some((1920, 0)),
        };
        assert_eq!(
            crop(&selection, &on_right),
            (
                Shape::Crop {
                    frame: (2560, 1440),
                    rect: rect(100, 100, 800, 600)
                },
                None
            )
        );

        // The portal streamed the other monitor, or a shot that is not the
        // monitors' bounding box: the whole monitor, with the warning.
        let on_left = Stream {
            position: Some((0, 0)),
            size: Some((1920, 1080)),
            ..on_right
        };
        assert_eq!(crop(&selection, &on_left), (Shape::Whole, Some(UNTRUSTED)));
        let no_position = Stream {
            position: None,
            ..on_right
        };
        assert_eq!(
            crop(&selection, &no_position),
            (Shape::Whole, Some(UNTRUSTED))
        );
        let odd = region((3000, 1440), &[left, right], right);
        assert!(!odd.trusted);
        assert_eq!(
            crop(
                &Selection {
                    region: odd,
                    ..selection
                },
                &on_right
            ),
            (Shape::Whole, Some(UNTRUSTED))
        );

        // XWayland with scaling: the monitors' geometry is logical, the
        // still physical; times the scale, it is their bounding box.
        let small = monitor(0, 0, 1440, 900, 2.0);
        let big = monitor(1440, 0, 1920, 1080, 2.0);
        let scaled = region((6720, 2160), &[small, big], big);
        assert_eq!(
            scaled,
            Region {
                rect: rect(2880, 0, 3840, 2160),
                trusted: true
            }
        );
        let drawn = Selection {
            region: scaled,
            rect: rect(3000, 100, 1000, 800),
            monitor: big,
            monitors: 2,
        };
        let big_stream = Stream {
            node: 7,
            size: Some((1920, 1080)),
            position: Some((1440, 0)),
        };
        assert_eq!(
            crop(&drawn, &big_stream),
            (
                Shape::Crop {
                    frame: (3840, 2160),
                    rect: rect(120, 100, 1000, 800)
                },
                None
            )
        );

        // Drawn over the whole monitor: no crop at all.
        let all = Selection {
            rect: rect(0, 0, 2880, 1800),
            ..single
        };
        assert_eq!(crop(&all, &stream), (Shape::Whole, None));
    }
}
