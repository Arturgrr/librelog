// Scrolling while the pen is on. The ink overlay takes the pointer so the
// pen draws, which would also take the wheel and the trackpad's scrolls and
// pinches from the app underneath. So a scroll that reaches the overlay
// lets the pointer through until that gesture and its momentum are over:
// the rest of it, and the next notches of a wheel, reach the app under the
// pointer as the system routes them. What still reaches the overlay (the
// first event, and a gesture the system keeps sending to the window it
// began in) is handed to the window under the pointer. A stroke being drawn
// keeps the pointer. Plain decisions here; `AnnotateSession` applies them.

import CoreGraphics
import Foundation

/// A scroll's or gesture's phase, as AppKit's `NSEvent.Phase` has it.
enum AnnotateGesturePhase: Equatable {
  /// A wheel's notch, or an event without phases.
  case unphased
  case mayBegin
  case began
  case changed
  case stationary
  case ended
  case cancelled
}

struct AnnotateScrollPass {
  /// With nothing in flight, the pointer comes back this long after the
  /// last scroll: past a wheel's next notch, and the gap between a
  /// trackpad's lift and its momentum.
  static let quiet: Double = 0.2
  /// A gesture whose end never came gives the pointer back after this.
  static let lost: Double = 1.0

  /// The overlay lets the pointer through.
  private(set) var passing = false
  /// Fingers on the trackpad.
  private var touching = false
  /// The momentum after they lifted.
  private var coasting = false
  private var last: Double = 0

  /// A scroll reached the overlay. Returns whether to let the pointer
  /// through and hand the event on: never while a stroke is drawn.
  mutating func reachedOverlay(
    phase: AnnotateGesturePhase, momentum: AnnotateGesturePhase, now: Double, drawing: Bool
  ) -> Bool {
    guard !drawing else { return false }
    passing = true
    observe(phase: phase, momentum: momentum, now: now)
    return true
  }

  /// A scroll or gesture seen anywhere while passing.
  mutating func observe(
    phase: AnnotateGesturePhase, momentum: AnnotateGesturePhase, now: Double
  ) {
    guard passing else { return }
    last = now
    switch phase {
    case .mayBegin, .began, .changed, .stationary: touching = true
    case .ended, .cancelled: touching = false
    case .unphased: break
    }
    switch momentum {
    case .mayBegin, .began, .changed, .stationary:
      touching = false
      coasting = true
    case .ended, .cancelled: coasting = false
    case .unphased: break
    }
  }

  /// True once the overlay should take the pointer back.
  mutating func tick(now: Double) -> Bool {
    guard passing else { return false }
    let idle = now - last
    guard (!touching && !coasting && idle >= Self.quiet) || idle >= Self.lost else { return false }
    reset()
    return true
  }

  mutating func reset() {
    passing = false
    touching = false
    coasting = false
  }
}

/// A window as `CGWindowListCopyWindowInfo` describes it: global points
/// from the main display's top-left corner, front to back.
struct AnnotateWindowInfo: Equatable {
  var number: Int
  var pid: Int32
  var layer: Int
  var alpha: Double
  var bounds: CGRect
}

enum AnnotateScrollTarget {
  /// Windows at this layer and above are status items, the menu bar and
  /// overlays like the ink's: never what a scroll is for.
  static let topLayer = 25

  /// The frontmost window under `point` that a scroll is for: shown, below
  /// the menu bar's level, and none of `own`.
  static func window(
    at point: CGPoint, in windows: [AnnotateWindowInfo], own: Set<Int>
  ) -> AnnotateWindowInfo? {
    windows.first { window in
      !own.contains(window.number) && window.layer < topLayer && window.alpha > 0
        && window.bounds.contains(point)
    }
  }
}
