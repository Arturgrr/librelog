//! The macOS layer's `AnnotateMachine` and `AnnotateStrokeRecorder` in Rust:
//! the ink mode with its safety timeouts, the capsule's discard question,
//! and strokes in the display's points. No clock is read here; every call
//! takes `now`, ms on the recording's clock.

use serde_json::{Value, json};

use crate::commands::InkMode;

/// Ink left on with the pointer still this long goes back to off.
pub const INK_IDLE_MS: u64 = 30_000;
/// An unanswered "Discard?" goes back to recording.
pub const QUESTION_MS: u64 = 4_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Pen,
    Stop,
    /// The ✕: asks "Discard?" first.
    Discard,
    ConfirmDiscard,
    Keep,
}

impl Press {
    pub fn parse(target: &str) -> Option<Self> {
        Some(match target {
            "pen" => Self::Pen,
            "stop" => Self::Stop,
            "discard" => Self::Discard,
            "confirm_discard" => Self::ConfirmDiscard,
            "keep" => Self::Keep,
            _ => return None,
        })
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Effects {
    pub ink_off: bool,
    pub question_closed: bool,
}

pub struct Machine {
    ink: InkMode,
    asking_since: Option<u64>,
    last_activity: u64,
    events: Vec<Value>,
}

fn event(kind: &str, reason: Option<&str>) -> Value {
    match reason {
        Some(reason) => json!({ "type": kind, "reason": reason }),
        None => json!({ "type": kind }),
    }
}

impl Machine {
    pub fn new(now: u64) -> Self {
        Self {
            ink: InkMode::Off,
            asking_since: None,
            last_activity: now,
            events: Vec::new(),
        }
    }

    pub fn ink(&self) -> InkMode {
        self.ink
    }

    pub fn asking(&self) -> bool {
        self.asking_since.is_some()
    }

    /// The host's ink mode; returns whether it changed.
    pub fn set_ink(&mut self, mode: InkMode, now: u64) -> bool {
        if mode == self.ink {
            return false;
        }
        self.ink = mode;
        self.last_activity = now;
        true
    }

    pub fn pointer_moved(&mut self, now: u64) {
        self.last_activity = now;
    }

    /// Escape: ink off, never the recording. Returns whether ink was on.
    pub fn escape(&mut self) -> bool {
        if self.ink == InkMode::Off {
            return false;
        }
        self.ink = InkMode::Off;
        self.events.push(event("ink_off", Some("escape")));
        true
    }

    /// A capsule button; returns whether the ink mode changed.
    pub fn press(&mut self, press: Press, now: u64) -> bool {
        match press {
            Press::Pen => {
                self.last_activity = now;
                // Any ink gives the pointer back in one click.
                if self.ink == InkMode::Off {
                    self.ink = InkMode::Drag;
                    self.events.push(event("pen_on", None));
                } else {
                    self.ink = InkMode::Off;
                    self.events.push(event("pen_off", None));
                }
                return true;
            }
            Press::Stop => {
                self.asking_since = None;
                if self.events.last().is_none_or(|last| last["type"] != "stop") {
                    self.events.push(event("stop", None));
                }
            }
            Press::Discard => self.asking_since = Some(now),
            Press::ConfirmDiscard => {
                if self.asking_since.take().is_some() {
                    self.events.push(event("discard", None));
                }
            }
            Press::Keep => self.asking_since = None,
        }
        false
    }

    pub fn tick(&mut self, now: u64) -> Effects {
        let mut effects = Effects::default();
        if self.ink != InkMode::Off && now.saturating_sub(self.last_activity) >= INK_IDLE_MS {
            self.ink = InkMode::Off;
            self.events.push(event("ink_off", Some("idle")));
            effects.ink_off = true;
        }
        if let Some(since) = self.asking_since
            && now.saturating_sub(since) >= QUESTION_MS
        {
            self.asking_since = None;
            effects.question_closed = true;
        }
        effects
    }

    /// The recording ended by itself: `max_duration` or `stream_error`.
    pub fn stopped(&mut self, reason: &str) {
        self.ink = InkMode::Off;
        self.asking_since = None;
        self.events.push(event("stopped", Some(reason)));
    }

    pub fn drain(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.events)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub mode: InkMode,
    /// `(x, y, t_ms)`: display points from its top-left corner.
    pub points: Vec<(f64, f64, u64)>,
}

impl Stroke {
    pub fn t0(&self) -> u64 {
        self.points.first().map_or(0, |p| p.2)
    }

    pub fn t1(&self) -> u64 {
        self.points.last().map_or(0, |p| p.2)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "mode": self.mode,
            "t0_ms": self.t0(),
            "t1_ms": self.t1(),
            "points": self
                .points
                .iter()
                .map(|&(x, y, t)| json!({ "x": x, "y": y, "t_ms": t }))
                .collect::<Vec<_>>(),
        })
    }
}

/// Strokes normalised as they arrive: clamped to the display, times never
/// going back, points closer than `MIN_STEP` to the last one dropped, and a
/// hover stroke that never moved dropped (a drag that never moved is a dot).
pub struct Strokes {
    size: (f64, f64),
    finished: Vec<Stroke>,
    live: Option<Stroke>,
}

const MIN_STEP: f64 = 0.5;

impl Strokes {
    pub fn new(size: (f64, f64)) -> Self {
        Self {
            size,
            finished: Vec::new(),
            live: None,
        }
    }

    pub fn live(&self) -> bool {
        self.live.is_some()
    }

    pub fn begin(&mut self, mode: InkMode, x: f64, y: f64, t: u64) {
        self.end();
        if mode == InkMode::Off {
            return;
        }
        let first = self.clamp(x, y, t, None);
        self.live = Some(Stroke {
            mode,
            points: vec![first],
        });
    }

    pub fn add(&mut self, x: f64, y: f64, t: u64) {
        let Some(last) = self.live.as_ref().and_then(|s| s.points.last().copied()) else {
            return;
        };
        let next = self.clamp(x, y, t, Some(last));
        if (next.0 - last.0).hypot(next.1 - last.1) < MIN_STEP {
            return;
        }
        if let Some(stroke) = self.live.as_mut() {
            stroke.points.push(next);
        }
    }

    pub fn end(&mut self) {
        let Some(stroke) = self.live.take() else {
            return;
        };
        if stroke.mode == InkMode::Hover && stroke.points.len() < 2 {
            return;
        }
        self.finished.push(stroke);
    }

    /// Every stroke, the live one ended.
    pub fn finish(mut self) -> Vec<Stroke> {
        self.end();
        self.finished
    }

    fn clamp(&self, x: f64, y: f64, t: u64, last: Option<(f64, f64, u64)>) -> (f64, f64, u64) {
        let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
        (
            finite(x).clamp(0.0, self.size.0.max(0.0)),
            finite(y).clamp(0.0, self.size.1.max(0.0)),
            t.max(last.map_or(0, |p| p.2)),
        )
    }
}
