// The Annotate capsule on screen: `AnnotatePillModel`'s plate in a panel
// above the ink, drawn with Core Graphics as the dictation HUD's plate is
// drawn in Metal, in the HUD's colours, which follow the system's
// appearance. Stepped at 60 Hz while it shows; it stays after a stop to
// show the annotation being prepared, then ready or failed, and closes
// itself once faded. Main thread only.

import AppKit
import QuartzCore

final class AnnotatePillView: NSView {
  var pose = AnnotatePillPose(width: AnnotatePillLayout.recordingWidth, opacity: 0, scale: 1)
  var stage = AnnotatePillStage.recording
  /// Seconds into `stage`.
  var stageClock: Double = 0
  var clock = "0:00"
  var asking = false
  var penOn = false
  var dark = false
  var reduceMotion = false
  var labels = AnnotateLabels()
  var onAction: ((AnnotateCapsuleTarget) -> Void)?

  override var isOpaque: Bool { false }

  override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

  /// 13 pt at weight 440 with even digits, as the HUD's live text.
  static let font: NSFont = {
    let wght = NSNumber(value: 0x7767_6874)
    let base = NSFont.monospacedDigitSystemFont(ofSize: 13, weight: .regular)
    let descriptor = base.fontDescriptor.addingAttributes([.variation: [wght: 440]])
    return NSFont(descriptor: descriptor, size: 13) ?? base
  }()

  /// What shows now, the question measured in the plate's font.
  var controls: AnnotatePillLayout {
    guard asking else { return AnnotatePillLayout.make(asking: false) }
    return AnnotatePillLayout.make(
      asking: true,
      labels: (width(labels.discardQuestion), width(labels.discard), width(labels.keep)))
  }

  private func width(_ string: String) -> CGFloat {
    (string as NSString).size(withAttributes: [.font: Self.font]).width
  }

  /// The plate in the view: centred, at the pose's width.
  private var plate: NSRect {
    let height = AnnotatePillLook.height
    return NSRect(
      x: bounds.midX - pose.width / 2, y: bounds.midY - height / 2, width: pose.width,
      height: height)
  }

  /// Where the controls start: centred at their own width, so they keep
  /// still while the plate springs to it.
  private func origin(_ parts: AnnotatePillLayout) -> CGPoint {
    CGPoint(
      x: (bounds.midX - parts.width / 2).rounded(),
      y: bounds.midY - AnnotatePillLook.height / 2)
  }

  override func mouseDown(with event: NSEvent) {
    guard stage == .recording, pose.opacity > 0.5 else { return }
    let point = convert(event.locationInWindow, from: nil)
    let parts = controls
    let start = origin(parts)
    let local = CGPoint(x: point.x - start.x, y: point.y - start.y)
    if let target = parts.target(at: local) {
      onAction?(target)
    }
  }

  private static func color(_ rgb: (r: Double, g: Double, b: Double)) -> NSColor {
    NSColor(srgbRed: rgb.r, green: rgb.g, blue: rgb.b, alpha: 1)
  }

  private static let red = NSColor(
    srgbRed: AnnotateInkLook.red.r, green: AnnotateInkLook.red.g, blue: AnnotateInkLook.red.b,
    alpha: 1)

  override func draw(_ dirtyRect: NSRect) {
    guard let context = NSGraphicsContext.current?.cgContext, pose.opacity > 0.001 else { return }
    let palette = AnnotatePillLook.palette(dark: dark)
    context.saveGState()
    context.setAlpha(CGFloat(pose.opacity))
    let (midX, midY) = (bounds.midX, bounds.midY)
    context.translateBy(x: midX, y: midY)
    context.scaleBy(x: CGFloat(pose.scale), y: CGFloat(pose.scale))
    context.translateBy(x: -midX, y: -midY)

    let plate = self.plate
    let radius = plate.height / 2
    let shape = NSBezierPath(roundedRect: plate, xRadius: radius, yRadius: radius)
    // A soft shadow 4 pt below, as the HUD's.
    context.saveGState()
    context.setShadow(
      offset: CGSize(width: 0, height: -4), blur: 12,
      color: CGColor(gray: 0, alpha: CGFloat(palette.shadowAlpha)))
    Self.color(palette.plate).setFill()
    shape.fill()
    context.restoreGState()
    let line = 1 / max(window?.backingScaleFactor ?? 2, 1)
    let rim = NSBezierPath(
      roundedRect: plate.insetBy(dx: line / 2, dy: line / 2), xRadius: radius - line / 2,
      yRadius: radius - line / 2)
    rim.lineWidth = line
    NSColor(white: CGFloat(palette.rim.white), alpha: CGFloat(palette.rim.alpha)).setStroke()
    rim.stroke()

    shape.addClip()
    let text = Self.color(palette.text)
    switch stage {
    case .recording:
      drawControls(text: text)
    case .processing:
      drawTrace(palette, in: plate)
    case .ready:
      drawGlyph("checkmark", color: text, in: plate)
    case .failed:
      drawGlyph("exclamationmark", color: Self.color(palette.danger), in: plate)
    }
    context.restoreGState()
  }

  private func drawControls(text: NSColor) {
    let parts = controls
    let start = origin(parts)
    func place(_ rect: CGRect) -> NSRect {
      rect.offsetBy(dx: start.x, dy: start.y)
    }
    if asking {
      label(labels.discardQuestion, in: place(parts.question), color: text, center: false)
      for (target, rect) in parts.targets {
        let frame = place(rect)
        let confirm = target == .confirmDiscard
        let round = frame.height / 2
        (confirm ? Self.red : text.withAlphaComponent(dark ? 0.12 : 0.06)).setFill()
        NSBezierPath(roundedRect: frame, xRadius: round, yRadius: round).fill()
        label(
          confirm ? labels.discard : labels.keep, in: frame, color: confirm ? .white : text,
          center: true)
      }
      return
    }
    Self.red.setFill()
    NSBezierPath(ovalIn: place(parts.dot)).fill()
    label(clock, in: place(parts.clock), color: text, center: false)
    text.withAlphaComponent(0.2).setFill()
    place(parts.divider).fill()
    for (target, rect) in parts.targets {
      let frame = place(rect)
      switch target {
      case .pen:
        if penOn {
          Self.red.setFill()
          NSBezierPath(ovalIn: frame).fill()
        }
        symbol("pencil.tip", in: frame, color: penOn ? .white : text, scale: 1)
      case .stop:
        text.setFill()
        let square = NSRect(x: frame.midX - 4, y: frame.midY - 4, width: 8, height: 8)
        NSBezierPath(roundedRect: square, xRadius: 2, yRadius: 2).fill()
      case .discard:
        text.setStroke()
        let cross = NSBezierPath()
        cross.lineWidth = 1.6
        cross.lineCapStyle = .round
        cross.move(to: NSPoint(x: frame.midX - 4, y: frame.midY - 4))
        cross.line(to: NSPoint(x: frame.midX + 4, y: frame.midY + 4))
        cross.move(to: NSPoint(x: frame.midX - 4, y: frame.midY + 4))
        cross.line(to: NSPoint(x: frame.midX + 4, y: frame.midY - 4))
        cross.stroke()
      case .confirmDiscard, .keep:
        break
      }
    }
  }

  /// Processing: a faint trace with light passing along it, as the HUD's
  /// scope shows a transcription.
  private func drawTrace(_ palette: AnnotatePillLook.Palette, in plate: NSRect) {
    let span = Double(AnnotatePillLook.processingWidth) - 32
    let left = Double(plate.midX) - span / 2
    let turn = reduceMotion ? 0 : stageClock * 2
    let samples = 36
    let (faint, strong) = (palette.faint, palette.text)
    var previous: NSPoint?
    for index in 0...samples {
      let x = span * Double(index) / Double(samples)
      let y = 2.5 * sin(2 * Double.pi * 1.6 * x / span - turn)
      let point = NSPoint(x: left + x, y: Double(plate.midY) + y)
      if let previous {
        let light = AnnotatePillLook.light(
          x: x, span: span, clock: stageClock, reduceMotion: reduceMotion)
        let mixed = (
          r: faint.r + (strong.r - faint.r) * light, g: faint.g + (strong.g - faint.g) * light,
          b: faint.b + (strong.b - faint.b) * light
        )
        Self.color(mixed).setStroke()
        let segment = NSBezierPath()
        segment.lineWidth = 1.5
        segment.lineCapStyle = .round
        segment.move(to: previous)
        segment.line(to: point)
        segment.stroke()
      }
      previous = point
    }
  }

  /// Ready's check mark or a failure's mark, popping in.
  private func drawGlyph(_ name: String, color: NSColor, in plate: NSRect) {
    let pop = AnnotatePillLook.pop(clock: stageClock, reduceMotion: reduceMotion)
    symbol(name, in: plate, color: color, scale: pop)
  }

  private func label(_ string: String, in rect: NSRect, color: NSColor, center: Bool) {
    let attributes: [NSAttributedString.Key: Any] = [.font: Self.font, .foregroundColor: color]
    let size = (string as NSString).size(withAttributes: attributes)
    let x = center ? rect.midX - size.width / 2 : rect.minX
    (string as NSString).draw(
      at: NSPoint(x: x, y: rect.midY - size.height / 2), withAttributes: attributes)
  }

  /// An SF Symbol in `color`, `scale` times its size, centred in `rect`.
  private func symbol(_ name: String, in rect: NSRect, color: NSColor, scale: Double) {
    let configuration = NSImage.SymbolConfiguration(pointSize: 11, weight: .bold)
    guard
      let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
        .withSymbolConfiguration(configuration),
      let context = NSGraphicsContext.current?.cgContext
    else {
      return
    }
    let size = NSSize(
      width: image.size.width * CGFloat(scale), height: image.size.height * CGFloat(scale))
    let frame = NSRect(
      x: rect.midX - size.width / 2, y: rect.midY - size.height / 2, width: size.width,
      height: size.height)
    context.saveGState()
    context.beginTransparencyLayer(auxiliaryInfo: nil)
    image.draw(in: frame)
    color.setFill()
    frame.fill(using: .sourceAtop)
    context.endTransparencyLayer()
    context.restoreGState()
  }
}

/// The capsule's panel and its clock.
final class AnnotatePill {
  let panel: AnnotatePanel
  let view: AnnotatePillView
  private var model = AnnotatePillModel()
  private var timer: Timer?
  private var lastStep: Double?
  /// Called once, when the panel closes.
  var onGone: (() -> Void)?

  var onAction: ((AnnotateCapsuleTarget) -> Void)? {
    get { view.onAction }
    set { view.onAction = newValue }
  }

  var clock = "0:00" {
    didSet { if clock != oldValue { redraw() } }
  }
  var penOn = false {
    didSet { if penOn != oldValue { redraw() } }
  }
  var asking = false {
    didSet { if asking != oldValue { redraw() } }
  }

  var windowNumber: Int { panel.windowNumber }

  init(screen: AnnotateScreen, labels: AnnotateLabels) {
    let size = AnnotatePillLayout.panelSize
    let view = AnnotatePillView(frame: NSRect(origin: .zero, size: size))
    view.labels = labels
    let panel = AnnotatePanel(
      frame: NSRect(origin: AnnotatePillLayout.panelOrigin(on: screen), size: size),
      level: AnnotateLevels.capsule)
    // Left unset, so the panel's clear margins let clicks through.
    panel.contentView = view
    self.view = view
    self.panel = panel
  }

  /// A recording starts: on screen, opening.
  func show() {
    view.dark = Self.systemDark()
    view.reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    model.contentWidth = view.controls.width
    model.show(now: CACurrentMediaTime(), reduceMotion: view.reduceMotion)
    panel.orderFrontRegardless()
    startTimer()
    step()
  }

  /// Processing, ready or failed.
  func set(_ stage: AnnotatePillStage) {
    model.set(stage, now: CACurrentMediaTime())
    startTimer()
  }

  /// Fades out without an outcome.
  func hide() {
    model.hide()
    startTimer()
  }

  /// Gone at once: the recording was thrown away, or another one starts.
  func close() {
    timer?.invalidate()
    timer = nil
    view.onAction = nil
    panel.orderOut(nil)
    panel.close()
    let gone = onGone
    onGone = nil
    gone?()
  }

  private func redraw() {
    view.clock = clock
    view.penOn = penOn
    view.asking = asking
    model.contentWidth = view.controls.width
    view.needsDisplay = true
  }

  private func startTimer() {
    guard timer == nil, model.active else { return }
    lastStep = nil
    let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in self?.step() }
    RunLoop.main.add(timer, forMode: .common)
    self.timer = timer
  }

  private func step() {
    let now = CACurrentMediaTime()
    let dt = lastStep.map { now - $0 } ?? 1.0 / 60
    lastStep = now
    model.step(now: now, dt: dt, reduceMotion: view.reduceMotion)
    let pose = model.pose
    let moving = model.stage != .recording || pose != view.pose || model.stage != view.stage
    view.pose = pose
    view.stage = model.stage
    view.stageClock = now - model.stageSince
    if moving { view.needsDisplay = true }
    if !model.active { close() }
  }

  /// The system's appearance, whatever the app's own theme, as the HUD's.
  private static func systemDark() -> Bool {
    UserDefaults.standard.string(forKey: "AppleInterfaceStyle")?.lowercased() == "dark"
  }
}
