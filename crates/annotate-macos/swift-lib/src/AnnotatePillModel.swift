// The Annotate capsule as plain state, made to look and move like the
// dictation HUD's plate (`crates/dictation-ui-macos/swift-lib/src`:
// `HudScope.swift`'s springs, widths and palette, `HudModel.swift`'s
// timings, `HudPlacement.swift`'s place under the menu bar): a 26 pt pill
// that opens with a little bounce, narrows to the HUD's processing width
// while the annotation is prepared, shows ready or failed for a moment and
// fades away. The two packages link into one app but are separate Swift
// modules, so the few constants and the spring are copied here. No clock is
// read; every call takes `now` in seconds.

import CoreGraphics
import Foundation

/// What the capsule shows.
enum AnnotatePillStage: Equatable {
  case recording
  /// The recording stopped; its contact sheet and summary are being made.
  case processing
  /// They are ready to paste.
  case ready
  case failed
}

enum AnnotatePillLook {
  /// The HUD's plate: `HudTuning.height` and `processingWidth`.
  static let height: CGFloat = 26
  static let processingWidth: CGFloat = 76
  /// The plate's gap under the menu bar, `HudPlacement.topOffset`.
  static let topOffset: CGFloat = 6
  /// Room around the plate in its panel for the shadow.
  static let margin: CGFloat = 12
  /// The panel's plate at its widest.
  static let maxWidth: CGFloat = 300
  /// `HudTuning.transition` and `bounce`.
  static let transition: Double = 0.3
  static let bounce: Double = 0.15
  static let hiddenScale: Double = 0.9
  /// `HudSpringSpec.reducedFade`.
  static let reducedFade: Double = 0.16
  /// `HudModel.processingMinimum`: a fast finish still shows it worked.
  static let processingMinimum: Double = 0.2
  /// Ready shows as long as the HUD's notch check mark does, a little more
  /// with the image and summary to wait for; a failure long enough to read
  /// the colour.
  static let readyVisible: Double = 0.8
  static let failedVisible: Double = 2.5
  /// A capsule the host never tells how the annotation came out goes.
  static let processingLimit: Double = 10 * 60

  /// The plate's colours, sRGB, as `HudPalette` and the live text plate.
  struct Palette {
    var plate: (r: Double, g: Double, b: Double)
    var rim: (white: Double, alpha: Double)
    var shadowAlpha: Double
    var text: (r: Double, g: Double, b: Double)
    var faint: (r: Double, g: Double, b: Double)
    var danger: (r: Double, g: Double, b: Double)
  }

  static func palette(dark: Bool) -> Palette {
    func hex(_ value: UInt32) -> (r: Double, g: Double, b: Double) {
      (Double(value >> 16) / 255, Double((value >> 8) & 0xFF) / 255, Double(value & 0xFF) / 255)
    }
    return Palette(
      plate: hex(dark ? 0x191919 : 0xFFFFFF),
      rim: dark ? (white: 1.0, alpha: 0.16) : (white: 0.0, alpha: 0.12),
      shadowAlpha: dark ? 0.45 : 0.12,
      text: hex(dark ? 0xFAFAFA : 0x161616),
      faint: hex(dark ? 0x5C5C5C : 0xAEAEAE),
      danger: hex(dark ? 0xF17471 : 0xB82D35))
  }

  /// The check mark's pop, `HudEars.pop`: in over 0.16 s with a little
  /// overshoot; whole at once with Reduce Motion.
  static func pop(clock: Double, reduceMotion: Bool) -> Double {
    guard !reduceMotion else { return 1 }
    let t = min(max(clock / 0.16, 0), 1)
    return max(smoothstep(0, 1, t) * (1 + 0.2 * sin(t * .pi)), 0.05)
  }

  /// How much of the light passing along the processing trace is at `x`
  /// (pt from its start, `span` long), `clock` seconds in: one band at a
  /// time, as the HUD's Flow look; still in the middle with Reduce Motion.
  static func light(x: Double, span: Double, clock: Double, reduceMotion: Bool) -> Double {
    let band = 7.0
    let lap = span + 4 * band
    let travelled = (clock * 48).truncatingRemainder(dividingBy: lap) - 2 * band
    let head = reduceMotion ? span / 2 : travelled
    let d = (x - head) / band
    return exp(-d * d)
  }

  private static func smoothstep(_ a: Double, _ b: Double, _ x: Double) -> Double {
    let t = min(max((x - a) / (b - a), 0), 1)
    return t * t * (3 - 2 * t)
  }
}

/// `HudSpring`: an exact, frame-rate independent damped oscillator, set by
/// a duration and a bounce as SwiftUI's springs are.
struct AnnotateSpring {
  private(set) var x: Double
  private(set) var v: Double = 0
  private(set) var target: Double

  init(at value: Double) {
    x = value
    target = value
  }

  mutating func jump(_ value: Double) {
    x = value
    v = 0
    target = value
  }

  mutating func step(to newTarget: Double, duration: Double, bounce: Double, dt: Double) {
    target = newTarget
    guard dt > 0, dt.isFinite else { return }
    let omega = 2 * Double.pi / max(duration, 0.01)
    let zeta = bounce >= 0 ? max(1 - bounce, 0) : 1 / max(1 + bounce, 0.01)
    let x0 = x - target
    let v0 = v
    if zeta < 1 - 1e-4 {
      let decay = zeta * omega
      let omegaD = omega * (1 - zeta * zeta).squareRoot()
      let e = exp(-decay * dt)
      let s = sin(omegaD * dt)
      let c = cos(omegaD * dt)
      let sinOverD = s / omegaD
      x = target + e * (x0 * (c + decay * sinOverD) + v0 * sinOverD)
      v = e * (-x0 * omega * omega * sinOverD + v0 * (c - decay * sinOverD))
    } else if zeta > 1 + 1e-4 {
      let root = (zeta * zeta - 1).squareRoot()
      let slow = -omega / (zeta + root)
      let fast = -omega * (zeta + root)
      let denom = slow - fast
      let slowExp = exp(slow * dt)
      let fastExp = exp(fast * dt)
      x =
        target + (-fast * slowExp + slow * fastExp) / denom * x0 + (slowExp - fastExp) / denom * v0
      v =
        slow * fast * (fastExp - slowExp) / denom * x0 + (slow * slowExp - fast * fastExp) / denom
        * v0
    } else {
      let e = exp(-omega * dt)
      let c = v0 + omega * x0
      x = target + (x0 + c * dt) * e
      v = (v0 - omega * c * dt) * e
    }
  }
}

/// The plate's pose for one frame.
struct AnnotatePillPose: Equatable {
  var width: CGFloat
  var opacity: Double
  var scale: Double
}

/// The capsule's lifecycle: recording, then processing until the host says
/// how the annotation came out, ready or failed for a moment, then gone.
struct AnnotatePillModel {
  private(set) var stage = AnnotatePillStage.recording
  /// When the current stage began, for its glyph's and light's clocks.
  private(set) var stageSince: Double = 0
  private var processingSince: Double = 0
  /// Ready came before processing had shown for its minimum.
  private var readyPending = false
  private var hidden = true
  /// Whether the current look is still meant to be seen.
  private(set) var shown = false
  /// False once the capsule is out of sight: its panel can go.
  private(set) var active = false
  /// The recording controls' width; the plate follows it while recording.
  var contentWidth: CGFloat = AnnotatePillLayout.recordingWidth
  private var width = AnnotateSpring(at: Double(AnnotatePillLayout.recordingWidth))
  private var opacity = AnnotateSpring(at: 0)
  private var scale = AnnotateSpring(at: AnnotatePillLook.hiddenScale)

  var pose: AnnotatePillPose {
    let (narrowest, widest) = (Double(AnnotatePillLook.height), Double(AnnotatePillLook.maxWidth))
    return AnnotatePillPose(
      width: CGFloat(min(max(width.x, narrowest), widest)), opacity: min(max(opacity.x, 0), 1),
      scale: max(scale.x, 0.01))
  }

  /// A recording starts: the plate opens from nothing.
  mutating func show(now: Double, reduceMotion: Bool) {
    stage = .recording
    stageSince = now
    readyPending = false
    hidden = false
    active = true
    shown = true
    width.jump(Double(contentWidth))
    opacity.jump(0)
    scale.jump(reduceMotion ? 1 : AnnotatePillLook.hiddenScale)
  }

  /// Processing, ready or failed. Ready waits for the processing look's
  /// minimum; nothing changes once the capsule is going.
  mutating func set(_ next: AnnotatePillStage, now: Double) {
    guard active, !hidden, next != stage else { return }
    switch next {
    case .recording:
      return
    case .processing:
      guard stage == .recording else { return }
      processingSince = now
      enter(.processing, now: now)
    case .ready:
      guard stage == .recording || stage == .processing else { return }
      if stage == .recording {
        processingSince = now
        enter(.processing, now: now)
      }
      readyPending = true
    case .failed:
      readyPending = false
      enter(.failed, now: now)
    }
  }

  /// Gone without an outcome: fades out from whatever it shows.
  mutating func hide() {
    hidden = true
    readyPending = false
  }

  private mutating func enter(_ next: AnnotatePillStage, now: Double) {
    stage = next
    stageSince = now
  }

  /// Advances `dt` seconds to `now`; once out of sight, `active` turns false.
  mutating func step(now: Double, dt rawDt: Double, reduceMotion: Bool) {
    guard active else { return }
    let dt = min(max(rawDt, 1.0 / 240), 1.0 / 20)
    if readyPending && now - processingSince >= AnnotatePillLook.processingMinimum {
      readyPending = false
      enter(.ready, now: now)
    }
    let elapsed = now - stageSince
    let visible: Bool
    switch stage {
    case .recording: visible = !hidden
    case .processing: visible = !hidden && now - processingSince < AnnotatePillLook.processingLimit
    case .ready: visible = !hidden && elapsed < AnnotatePillLook.readyVisible
    case .failed: visible = !hidden && elapsed < AnnotatePillLook.failedVisible
    }
    let targetWidth =
      stage == .recording ? Double(contentWidth) : Double(AnnotatePillLook.processingWidth)
    let shownTarget: Double = visible ? 1 : 0
    let (duration, bounce) = (AnnotatePillLook.transition, AnnotatePillLook.bounce)
    if reduceMotion {
      width.jump(targetWidth)
      scale.jump(1)
      opacity.step(to: shownTarget, duration: AnnotatePillLook.reducedFade, bounce: 0, dt: dt)
    } else {
      width.step(to: targetWidth, duration: duration, bounce: bounce, dt: dt)
      opacity.step(to: shownTarget, duration: duration, bounce: 0, dt: dt)
      scale.step(
        to: visible ? 1 : AnnotatePillLook.hiddenScale, duration: duration, bounce: bounce, dt: dt)
    }
    shown = visible
    active = visible || opacity.x >= 0.002
  }
}

/// The recording controls inside the plate, `● 0:07 ✎ │ ■ ✕`, or
/// `Discard? [Discard] [Keep]` while it asks; in the plate's points from
/// its bottom-left corner (y up), the plate `height` tall.
struct AnnotatePillLayout {
  static let height = AnnotatePillLook.height
  static let button: CGFloat = 22
  static let leading: CGFloat = 10
  static let trailing: CGFloat = 2
  static let gap: CGFloat = 2
  /// Room for the clock up to `19:59`, the cap, in 13 pt digits.
  static let clockWidth: CGFloat = 40
  /// The question's buttons: their label plus this, at least `button`.
  static let labelPadding: CGFloat = 20

  static var recordingWidth: CGFloat {
    make(asking: false).width
  }

  var width: CGFloat
  var dot: CGRect
  var clock: CGRect
  var divider: CGRect
  var question: CGRect
  /// The buttons that show, front to back.
  var targets: [(AnnotateCapsuleTarget, CGRect)]

  /// `labels` are the question's, the discard button's and the keep
  /// button's widths, measured in the plate's font.
  static func make(
    asking: Bool, labels: (question: CGFloat, discard: CGFloat, keep: CGFloat) = (60, 50, 34)
  ) -> AnnotatePillLayout {
    let buttonY = (height - button) / 2
    if asking {
      let question = CGRect(x: 12, y: 0, width: ceil(max(labels.question, 0)), height: height)
      let discardWidth = max(ceil(labels.discard) + labelPadding, button)
      let keepWidth = max(ceil(labels.keep) + labelPadding, button)
      let discard = CGRect(x: question.maxX + 8, y: buttonY, width: discardWidth, height: button)
      let keep = CGRect(x: discard.maxX + 4, y: buttonY, width: keepWidth, height: button)
      return AnnotatePillLayout(
        width: min(keep.maxX + trailing, AnnotatePillLook.maxWidth), dot: .zero, clock: .zero,
        divider: .zero, question: question,
        targets: [(.confirmDiscard, discard), (.keep, keep)])
    }
    let dot = CGRect(x: leading, y: (height - 8) / 2, width: 8, height: 8)
    let clock = CGRect(x: dot.maxX + 6, y: 0, width: clockWidth, height: height)
    let pen = CGRect(x: clock.maxX + 4, y: buttonY, width: button, height: button)
    let divider = CGRect(x: pen.maxX + 6, y: (height - 14) / 2, width: 1, height: 14)
    let stop = CGRect(x: divider.maxX + 6, y: buttonY, width: button, height: button)
    let discard = CGRect(x: stop.maxX + gap, y: buttonY, width: button, height: button)
    return AnnotatePillLayout(
      width: discard.maxX + trailing, dot: dot, clock: clock, divider: divider, question: .zero,
      targets: [(.pen, pen), (.stop, stop), (.discard, discard)])
  }

  /// The button under `point`, in the plate's points.
  func target(at point: CGPoint) -> AnnotateCapsuleTarget? {
    targets.first { $0.1.contains(point) }?.0
  }

  /// The panel the plate sits in: wide enough for its widest, with room
  /// for its shadow.
  static var panelSize: CGSize {
    CGSize(
      width: AnnotatePillLook.maxWidth + 2 * AnnotatePillLook.margin,
      height: height + 2 * AnnotatePillLook.margin)
  }

  /// The panel's bottom-left corner on `screen`: the plate centred,
  /// `topOffset` under the menu bar, or under the notch where a full-screen
  /// space hides the menu bar, as the dictation HUD's.
  static func panelOrigin(on screen: AnnotateScreen) -> CGPoint {
    let top =
      min(screen.visibleFrame.maxY, screen.frame.maxY - screen.safeTop) - AnnotatePillLook.topOffset
    let size = panelSize
    return CGPoint(
      x: (screen.frame.midX - size.width / 2).rounded(),
      y: top + AnnotatePillLook.margin - size.height)
  }
}
