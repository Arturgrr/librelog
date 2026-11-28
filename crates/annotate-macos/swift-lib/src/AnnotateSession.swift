// One recording at a time: the state machine, the recorder, the ink
// overlay and the capsule, and what the host reads back. The machine, the
// strokes and the windows live on the main thread; the host's calls come
// from its worker threads, hop to the main thread only for short steps,
// and do ScreenCaptureKit's waiting where they are. Events and the status
// sit behind a lock, so polling never waits on the main thread.

import AppKit
import ApplicationServices
import Foundation
import ScreenCaptureKit

/// A recording being finished, or finished and not yet claimed.
final class AnnotateFinish {
  let dir: URL
  let reason: String
  /// Whether `start` made `candidates/`; cancel removes only what it made.
  let ownsCandidates: Bool
  private let group = DispatchGroup()
  private let outcome = AnnotateLocked<Result<AnnotateStopResult, AnnotateFailure>?>(nil)

  init(dir: URL, reason: String, ownsCandidates: Bool) {
    self.dir = dir
    self.reason = reason
    self.ownsCandidates = ownsCandidates
    group.enter()
  }

  var isDone: Bool { outcome.get() != nil }

  func complete(_ result: Result<AnnotateStopResult, AnnotateFailure>) {
    guard outcome.get() == nil else { return }
    outcome.set(result)
    group.leave()
  }

  func wait(_ seconds: Double) -> Result<AnnotateStopResult, AnnotateFailure> {
    _ = group.wait(timeout: .now() + seconds)
    return outcome.get()
      ?? .failure(AnnotateFailure(.timeout, "the recording did not finish in time"))
  }
}

/// What a recording keeps from its start to its finish. Main thread.
private final class AnnotateCurrent {
  let screen: AnnotateScreen
  let dir: URL
  let front: AnnotateFrontApp
  let ink: AnnotateInkPanel
  let pill: AnnotatePill
  var recorder: AnnotateRecorder?
  var anchorTicks: UInt64 = 0
  var wallMs: Int64 = 0
  var display: AnnotateDisplayInfo?
  var createdCandidates = false
  var strokes: AnnotateStrokeRecorder
  var lastMouse: CGPoint?
  var animating = true
  /// A scroll going through the overlay, and the window it is for.
  var scroll = AnnotateScrollPass()
  var scrollTarget: AnnotateWindowInfo?

  init(screen: AnnotateScreen, dir: URL, front: AnnotateFrontApp, labels: AnnotateLabels) {
    self.screen = screen
    self.dir = dir
    self.front = front
    ink = AnnotateInkPanel(screen: screen)
    pill = AnnotatePill(screen: screen, labels: labels)
    strokes = AnnotateStrokeRecorder(size: screen.frame.size)
  }
}

/// What a finishing recording hands its worker.
private struct AnnotateFinishing {
  var current: AnnotateCurrent
  var recorder: AnnotateRecorder
  var strokes: [AnnotateStroke]
  var stopTicks: UInt64
  var endedBy: String
}

final class AnnotateSession {
  static let shared = AnnotateSession()

  private struct Mailbox {
    var events: [AnnotateEvent] = []
    var status = AnnotateStatus.idle
  }

  private let mailbox = AnnotateLocked(Mailbox())
  private let pending = AnnotateLocked<AnnotateFinish?>(nil)

  // Main thread only.
  private var machine = AnnotateMachine()
  private var current: AnnotateCurrent?
  /// The capsule of the recording that stopped last, while it shows the
  /// annotation being prepared, then ready or failed.
  private var lingering: AnnotatePill?
  private var timer: Timer?
  private var monitors: [Any] = []
  /// Told once that scrolls can't go through the pen without Accessibility.
  private var toldAccessibility = false

  private var timebase: AnnotateTimebase { AnnotateHostClock.timebase }

  // MARK: Any thread

  func status() -> AnnotateStatus {
    var status = mailbox.get().status
    status.resultPending = pending.get()?.isDone ?? false
    return status
  }

  func drainEvents() -> [AnnotateEvent] {
    mailbox.update { mailbox in
      defer { mailbox.events.removeAll() }
      return mailbox.events
    }
  }

  func setInk(_ mode: AnnotateInkMode) -> Result<AnnotateNothing, AnnotateFailure> {
    guard mailbox.get().status.phase == .recording else {
      return .failure(AnnotateFailure(.notRecording, "nothing is recording"))
    }
    DispatchQueue.main.async { self.applyHostInk(mode) }
    return .success(AnnotateNothing())
  }

  /// The pen shortcut: as the capsule's pen.
  func togglePen() -> Result<AnnotateNothing, AnnotateFailure> {
    guard mailbox.get().status.phase == .recording else {
      return .failure(AnnotateFailure(.notRecording, "nothing is recording"))
    }
    DispatchQueue.main.async { self.capsuleAction(.pen) }
    return .success(AnnotateNothing())
  }

  /// How the stopped recording's annotation came out: `processing`,
  /// `ready`, `failed`, or `hidden` when it was thrown away.
  func capsule(_ name: String) -> Result<AnnotateNothing, AnnotateFailure> {
    let stage: AnnotatePillStage?
    switch name {
    case "processing": stage = .processing
    case "ready": stage = .ready
    case "failed": stage = .failed
    case "hidden": stage = nil
    default:
      return .failure(
        AnnotateFailure(
          .invalidArgument, "the capsule shows processing, ready, failed or hidden, not \(name)"))
    }
    DispatchQueue.main.async { self.showOutcome(stage) }
    return .success(AnnotateNothing())
  }

  /// Whether the narration's microphone records, and its level from 0 to
  /// 1, for the recording's capsule.
  func narration(heard: Bool, level: Double) -> Result<AnnotateNothing, AnnotateFailure> {
    DispatchQueue.main.async { self.current?.pill.narration(heard: heard, level: level) }
    return .success(AnnotateNothing())
  }

  // MARK: Worker threads

  func start(_ request: AnnotateStartRequest) -> Result<AnnotateStartReply, AnnotateFailure> {
    guard request.dir.hasPrefix("/") else {
      return .failure(AnnotateFailure(.invalidArgument, "dir must be an absolute path"))
    }
    let dir = URL(fileURLWithPath: request.dir, isDirectory: true)
    let current: AnnotateCurrent
    let labels = request.labels ?? AnnotateLabels()
    switch annotateOnMain({ reserve(dir: dir, labels: labels) }) {
    case .success(let reserved): current = reserved
    case .failure(let failure): return .failure(failure)
    }
    var created: [URL] = []
    let result = prepareAndStart(current, created: &created)
    if case .failure = result {
      annotateOnMain { abortStart(current) }
      remove(created)
    }
    return result
  }

  func stop() -> Result<AnnotateStopResult, AnnotateFailure> {
    switch annotateOnMain({ beginFinishing(reason: "stop") }) {
    case .failure(let failure):
      return .failure(failure)
    case .success(let step):
      let result: Result<AnnotateStopResult, AnnotateFailure>
      switch step {
      case .run(let finishing, let finish):
        result = runFinish(finishing, finish)
      case .wait(let finish):
        result = finish.wait(AnnotateRecorder.finishTimeout * 2)
      }
      claim()
      return result
    }
  }

  /// Stops and deletes. Idempotent: nothing to cancel is fine.
  func cancel() -> Result<AnnotateNothing, AnnotateFailure> {
    switch annotateOnMain({ beginFinishing(reason: "cancel") }) {
    case .failure(let failure):
      if failure.code == .notRecording { return .success(AnnotateNothing()) }
      return .failure(failure)
    case .success(let step):
      let finish: AnnotateFinish
      switch step {
      case .run(let finishing, let started):
        _ = runFinish(finishing, started)
        finish = started
      case .wait(let waiting):
        _ = waiting.wait(AnnotateRecorder.finishTimeout * 2)
        finish = waiting
      }
      claim()
      remove(
        [finish.dir.appendingPathComponent(AnnotateFiles.video)]
          + (finish.ownsCandidates
            ? [finish.dir.appendingPathComponent(AnnotateFiles.candidates)] : []))
      return .success(AnnotateNothing())
    }
  }

  private func prepareAndStart(_ current: AnnotateCurrent, created: inout [URL]) -> Result<
    AnnotateStartReply, AnnotateFailure
  > {
    let files = FileManager.default
    let video = current.dir.appendingPathComponent(AnnotateFiles.video)
    let candidates = current.dir.appendingPathComponent(AnnotateFiles.candidates)
    if files.fileExists(atPath: video.path) {
      return .failure(AnnotateFailure(.invalidArgument, "\(video.path) already exists"))
    }
    do {
      try files.createDirectory(at: current.dir, withIntermediateDirectories: true)
      if !files.fileExists(atPath: candidates.path) {
        try files.createDirectory(at: candidates, withIntermediateDirectories: true)
        created.append(candidates)
      }
    } catch {
      return .failure(AnnotateFailure(.io, "could not create \(current.dir.path): \(error)"))
    }
    created.append(video)

    let content: SCShareableContent
    switch Self.shareableContent() {
    case .success(let value): content = value
    case .failure(let failure): return .failure(failure)
    }
    guard let display = content.displays.first(where: { $0.displayID == current.screen.id })
    else {
      return .failure(AnnotateFailure(.noDisplay, "the display under the pointer is gone"))
    }
    let ownWindows = annotateOnMain {
      Set([current.ink.windowNumber, current.pill.windowNumber])
    }
    let pid = ProcessInfo.processInfo.processIdentifier
    // This app's chrome (panels, HUDs: anything above the normal level) and
    // the two panels; its normal windows are recorded like any app's.
    let excluded = content.windows.filter { window in
      window.owningApplication?.processID == pid
        && (window.windowLayer != 0 || ownWindows.contains(Int(window.windowID)))
    }
    let filter = SCContentFilter(display: display, excludingWindows: excluded)
    let screen = current.screen
    guard
      let video = AnnotateGeometry.pixelSize(
        points: screen.frame.size, scale: screen.scale, longEdge: AnnotateRecorder.videoLongEdge),
      let candidate = AnnotateGeometry.pixelSize(
        points: screen.frame.size, scale: screen.scale,
        longEdge: AnnotateRecorder.candidateLongEdge)
    else {
      return .failure(AnnotateFailure(.noDisplay, "the display has no size"))
    }

    let recorder = AnnotateRecorder()
    recorder.onFailure = { message in
      DispatchQueue.main.async { self.finishByItself(reason: "stream_error", message: message) }
    }
    let anchor = AnnotateHostClock.now()
    let wallMs = Int64((Date().timeIntervalSince1970 * 1000).rounded())
    let plan = AnnotateRecordingPlan(
      filter: filter, videoSize: video, candidateSize: candidate, dir: current.dir,
      anchorTicks: anchor)
    if let failure = recorder.start(plan) {
      return .failure(failure)
    }
    let info = AnnotateDisplayInfo(
      id: screen.id, widthPt: Double(screen.frame.width), heightPt: Double(screen.frame.height),
      scale: Double(screen.scale),
      widthPx: Int((screen.frame.width * screen.scale).rounded()),
      heightPx: Int((screen.frame.height * screen.scale).rounded()), videoWidth: video.width,
      videoHeight: video.height)
    let ownsCandidates = created.contains(candidates)
    let running = annotateOnMain { () -> Bool in
      current.recorder = recorder
      current.createdCandidates = ownsCandidates
      current.anchorTicks = anchor
      current.wallMs = wallMs
      current.display = info
      return began(current)
    }
    guard running else {
      // Cancelled between the reservation and the stream starting.
      _ = recorder.stop()
      return .failure(AnnotateFailure(.busy, "the recording was cancelled while starting"))
    }
    return .success(
      AnnotateStartReply(
        startedHostTicks: anchor, startedHostNs: timebase.nanoseconds(anchor),
        startedWallMs: wallMs, display: info))
  }

  static func shareableContent() -> Result<SCShareableContent, AnnotateFailure> {
    let reply = AnnotateLocked<(content: SCShareableContent?, error: Error?)?>(nil)
    let done = DispatchSemaphore(value: 0)
    SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) {
      content, error in
      reply.set((content, error))
      done.signal()
    }
    guard done.wait(timeout: .now() + AnnotateRecorder.startTimeout) == .success,
      let answer = reply.get()
    else {
      return .failure(AnnotateFailure(.timeout, "ScreenCaptureKit did not list the displays"))
    }
    if let content = answer.content { return .success(content) }
    if !CGPreflightScreenCaptureAccess() {
      return .failure(AnnotateFailure(.permission, "Screen Recording is not allowed"))
    }
    return .failure(AnnotateRecorder.failure(answer.error, "could not list the displays"))
  }

  /// Builds the result from the recorder's outcome; marks the session
  /// idle. On the calling worker.
  private func runFinish(_ finishing: AnnotateFinishing, _ finish: AnnotateFinish) -> Result<
    AnnotateStopResult, AnnotateFailure
  > {
    let outcome = finishing.recorder.stop()
    let current = finishing.current
    let result: Result<AnnotateStopResult, AnnotateFailure>
    if let failure = outcome.failure {
      result = .failure(failure)
    } else if let display = current.display {
      let offset = outcome.firstFrameTicks.map {
        timebase.milliseconds(from: current.anchorTicks, to: $0)
      }
      result = .success(
        AnnotateStopResult(
          videoPath: current.dir.appendingPathComponent(AnnotateFiles.video).path,
          startedHostTicks: current.anchorTicks,
          startedHostNs: timebase.nanoseconds(current.anchorTicks),
          startedWallMs: current.wallMs,
          durationMs: timebase.milliseconds(from: current.anchorTicks, to: finishing.stopTicks),
          videoOffsetMs: offset, videoDurationMs: outcome.videoDurationMs, display: display,
          frontApp: current.front, candidates: outcome.candidates, strokes: finishing.strokes,
          endedBy: finishing.endedBy, warnings: outcome.warnings))
    } else {
      result = .failure(AnnotateFailure(.internal, "the recording has no display"))
    }
    annotateOnMain {
      finish.complete(result)
      machine.reset()
      if finishing.endedBy != "stop" && finishing.endedBy != "cancel" {
        machine.stoppedByItself(reason: finishing.endedBy, message: outcome.failure?.message)
      }
      flush()
    }
    return result
  }

  /// The host has the finished recording: nothing pending, and no
  /// `stopped` event left to tell of it.
  private func claim() {
    pending.set(nil)
    mailbox.update { mailbox in mailbox.events.removeAll { $0.type == .stopped } }
  }

  /// Removes what a recording wrote; the directory itself is the host's.
  private func remove(_ urls: [URL]) {
    for url in urls { try? FileManager.default.removeItem(at: url) }
  }

  // MARK: Main thread

  /// Nothing starting, recording or finishing.
  var isIdle: Bool { machine.phase == .idle }

  private enum FinishStep {
    case run(AnnotateFinishing, AnnotateFinish)
    case wait(AnnotateFinish)
  }

  private func reserve(dir: URL, labels: AnnotateLabels) -> Result<
    AnnotateCurrent, AnnotateFailure
  > {
    if machine.phase == .idle, pending.get()?.isDone == true {
      // An unclaimed result of a recording that ended by itself; its files
      // stay where they are.
      claim()
    }
    // One capture or Annotate recording at a time; Capture checks the
    // other way round, both on the main thread.
    guard CaptureSession.shared.isIdle else {
      return .failure(AnnotateFailure(.busy, "a screenshot or screen recording is under way"))
    }
    guard machine.reserve() else {
      return .failure(AnnotateFailure(.busy, "a recording is \(machine.phase.rawValue)"))
    }
    // A stop or discard left from the last recording is not this one's,
    // nor is its capsule.
    mailbox.update { mailbox in mailbox.events.removeAll() }
    lingering?.close()
    let screens = NSScreen.screens.compactMap { AnnotateScreen($0) }
    guard let screen = AnnotateGeometry.screen(under: NSEvent.mouseLocation, in: screens) else {
      machine.reset()
      flush()
      return .failure(AnnotateFailure(.noDisplay, "no display"))
    }
    let current = AnnotateCurrent(
      screen: screen, dir: dir, front: AnnotateFront.read(), labels: labels)
    // On screen before the filter is built, so it can leave them out.
    current.ink.orderFrontRegardless()
    current.pill.show()
    self.current = current
    flush()
    return .success(current)
  }

  private func abortStart(_ current: AnnotateCurrent) {
    guard self.current === current else { return }
    tearDown()
    machine.reset()
    flush()
  }

  /// The stream runs: the clock starts and ink can be drawn. False when
  /// this start was given up meanwhile.
  private func began(_ current: AnnotateCurrent) -> Bool {
    guard self.current === current, machine.phase == .starting else { return false }
    let now = timebase.seconds(current.anchorTicks)
    machine.started(now: now)
    current.ink.inkView.onMouse = { [weak self] kind, point in self?.mouse(kind, point) }
    current.ink.inkView.onPass = { [weak self] event in self?.passThrough(event) }
    current.pill.onAction = { [weak self] target in self?.capsuleAction(target) }
    // Escape reaches a global monitor only while the app may monitor keys
    // (Accessibility); the local one covers the app's own windows.
    let onEscape: (NSEvent) -> Void = { [weak self] event in
      if event.keyCode == 53 { self?.escape() }
    }
    if let monitor = NSEvent.addGlobalMonitorForEvents(matching: .keyDown, handler: onEscape) {
      monitors.append(monitor)
    }
    let local = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
      guard event.keyCode == 53, let self, self.machine.ink != .off else { return event }
      self.escape()
      return nil
    }
    if let local { monitors.append(local) }
    // Where a scroll that went through the overlay is, to know when it ends,
    // and a click, which means to draw; mouse events reach a global monitor
    // without Accessibility.
    let gestures: NSEvent.EventTypeMask = [
      .scrollWheel, .magnify, .rotate, .smartMagnify, .leftMouseDown,
    ]
    let watched = NSEvent.addGlobalMonitorForEvents(matching: gestures) { [weak self] event in
      self?.scrolled(event)
    }
    if let watched { monitors.append(watched) }
    let scrolls = NSEvent.addLocalMonitorForEvents(matching: gestures) { [weak self] event in
      self?.scrolled(event)
      return event
    }
    if let scrolls { monitors.append(scrolls) }
    let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in self?.tick() }
    RunLoop.main.add(timer, forMode: .common)
    self.timer = timer
    tick()
    return true
  }

  private func beginFinishing(reason: String) -> Result<FinishStep, AnnotateFailure> {
    switch machine.phase {
    case .starting:
      return .failure(AnnotateFailure(.busy, "the recording is still starting"))
    case .idle, .finishing:
      if let finish = pending.get() { return .success(.wait(finish)) }
      return .failure(AnnotateFailure(.notRecording, "nothing is recording"))
    case .recording:
      guard let finishing = finishing(endedBy: reason) else {
        return .failure(AnnotateFailure(.internal, "the recording has no recorder"))
      }
      let finish = AnnotateFinish(
        dir: finishing.current.dir, reason: reason,
        ownsCandidates: finishing.current.createdCandidates)
      pending.set(finish)
      return .success(.run(finishing, finish))
    }
  }

  /// Recording to finishing: the last stroke ends, the windows go.
  private func finishing(endedBy: String) -> AnnotateFinishing? {
    guard let current, let recorder = current.recorder, machine.finishing() else { return nil }
    let stopTicks = AnnotateHostClock.now()
    current.strokes.end(tMs: timebase.milliseconds(from: current.anchorTicks, to: stopTicks))
    let finishing = AnnotateFinishing(
      current: current, recorder: recorder, strokes: current.strokes.strokes,
      stopTicks: stopTicks, endedBy: endedBy)
    // A stopped recording's capsule stays to show it being prepared.
    tearDown(keepingPill: endedBy != "cancel")
    flush()
    return finishing
  }

  /// The cap or a stream failure: finish on a worker; `stop` collects it.
  private func finishByItself(reason: String, message: String? = nil) {
    guard machine.phase == .recording, let finishing = finishing(endedBy: reason) else { return }
    let finish = AnnotateFinish(
      dir: finishing.current.dir, reason: reason,
      ownsCandidates: finishing.current.createdCandidates)
    pending.set(finish)
    DispatchQueue.global(qos: .userInitiated).async {
      _ = self.runFinish(finishing, finish)
    }
  }

  /// Every window and timer of the recording goes, but the capsule when it
  /// is kept to show the annotation being prepared. Every way out of a
  /// recording comes through here.
  private func tearDown(keepingPill: Bool = false) {
    timer?.invalidate()
    timer = nil
    for monitor in monitors { NSEvent.removeMonitor(monitor) }
    monitors.removeAll()
    if let current {
      current.ink.inkView.onMouse = nil
      current.ink.inkView.onPass = nil
      current.pill.onAction = nil
      current.ink.takesPointer(false)
      current.ink.orderOut(nil)
      current.ink.close()
      if keepingPill {
        linger(current.pill)
      } else {
        current.pill.close()
      }
    }
    current = nil
  }

  /// The stopped recording's capsule shows processing until the host says
  /// how the annotation came out, then goes by itself.
  private func linger(_ pill: AnnotatePill) {
    lingering?.close()
    lingering = pill
    pill.onGone = { [weak self, weak pill] in
      guard let self, let pill, self.lingering === pill else { return }
      self.lingering = nil
    }
    pill.set(.processing)
  }

  private func showOutcome(_ stage: AnnotatePillStage?) {
    guard let pill = lingering else { return }
    if let stage {
      pill.set(stage)
    } else {
      pill.hide()
    }
  }

  /// A scroll or pinch reached the overlay while ink is on: the pointer
  /// goes through until it is over, and this event goes to the window
  /// under the pointer. Handing it on needs Accessibility; without it the
  /// pen keeps the wheel, as before, since a gesture the system keeps
  /// sending to the overlay would otherwise be lost whole.
  private func passThrough(_ event: NSEvent) {
    guard let current, machine.phase == .recording, machine.ink != .off else { return }
    let (phase, momentum) = Self.phases(event)
    let now = nowTicks().seconds
    let began = !current.scroll.passing
    if began && !AXIsProcessTrusted() {
      if !toldAccessibility {
        toldAccessibility = true
        NSLog("Annotate: scrolling under the pen needs Accessibility; the pen keeps the wheel")
      }
      return
    }
    let drawing = machine.ink == .drag && current.strokes.live != nil
    let passes = current.scroll.reachedOverlay(
      phase: phase, momentum: momentum, now: now, drawing: drawing)
    guard passes else { return }
    machine.pointerMoved(now: now)
    current.ink.passesPointer(true)
    if began || current.scrollTarget == nil {
      let own = ProcessInfo.processInfo.processIdentifier
      let point = event.cgEvent?.location ?? .zero
      current.scrollTarget = AnnotateScrollTarget.window(
        at: point, in: Self.windows(), own: own)
    }
    if let target = current.scrollTarget {
      Self.forward(event, to: target)
    }
  }

  /// A scroll seen anywhere, for when one that went through ends, or a
  /// click, after which the pen takes the pointer back at once. That click
  /// itself already went to the app underneath: a monitor sees it only
  /// after.
  private func scrolled(_ event: NSEvent) {
    guard let current, current.scroll.passing else { return }
    if event.type == .leftMouseDown {
      if current.scroll.mouseDown() { current.ink.passesPointer(false) }
      return
    }
    let (phase, momentum) = Self.phases(event)
    current.scroll.observe(phase: phase, momentum: momentum, now: nowTicks().seconds)
  }

  /// An event's phases, read only where AppKit keeps them.
  private static func phases(_ event: NSEvent) -> (AnnotateGesturePhase, AnnotateGesturePhase) {
    switch event.type {
    case .scrollWheel:
      return (AnnotateGesturePhase(event.phase), AnnotateGesturePhase(event.momentumPhase))
    case .magnify, .rotate:
      return (AnnotateGesturePhase(event.phase), .unphased)
    default:
      return (.unphased, .unphased)
    }
  }

  /// The on-screen windows, front to back.
  private static func windows() -> [AnnotateWindowInfo] {
    let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
    guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]]
    else { return [] }
    return list.compactMap { info in
      guard let number = info[kCGWindowNumber as String] as? Int,
        let pid = info[kCGWindowOwnerPID as String] as? Int32,
        let layer = info[kCGWindowLayer as String] as? Int,
        let bounds = info[kCGWindowBounds as String] as? NSDictionary,
        let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary)
      else { return nil }
      let alpha = info[kCGWindowAlpha as String] as? Double ?? 1
      return AnnotateWindowInfo(number: number, pid: pid, layer: layer, alpha: alpha, bounds: rect)
    }
  }

  /// Hands a copy of `event` to `target`'s app, addressed to that window.
  /// Like any synthetic event it needs Accessibility, which pasting a
  /// dictation already asks for; `passThrough` checks it first.
  private static func forward(_ event: NSEvent, to target: AnnotateWindowInfo) {
    guard let copy = event.cgEvent?.copy() else { return }
    // `kCGMouseEventWindowUnderMousePointer` and its `ThatCanHandleThisEvent`.
    for raw: UInt32 in [91, 92] {
      if let field = CGEventField(rawValue: raw) {
        copy.setIntegerValueField(field, value: Int64(target.number))
      }
    }
    copy.postToPid(target.pid)
  }

  private func nowTicks() -> (ticks: UInt64, seconds: Double) {
    let ticks = AnnotateHostClock.now()
    return (ticks, timebase.seconds(ticks))
  }

  private func tMs(_ current: AnnotateCurrent, _ ticks: UInt64) -> Int {
    timebase.milliseconds(from: current.anchorTicks, to: ticks)
  }

  private func applyHostInk(_ mode: AnnotateInkMode) {
    let now = nowTicks()
    guard machine.setInk(mode, now: now.seconds) else { return }
    inkChanged(at: now.ticks)
    flush()
  }

  private func escape() {
    guard machine.escape() else { return }
    inkChanged(at: AnnotateHostClock.now())
    flush()
  }

  /// Follows the machine's ink mode: the live stroke ends, the overlay
  /// takes or releases the pointer, and hover starts at the pointer.
  private func inkChanged(at ticks: UInt64) {
    guard let current else { return }
    let t = tMs(current, ticks)
    current.strokes.end(tMs: t)
    current.ink.takesPointer(machine.ink != .off)
    if machine.ink == .off {
      current.scroll.reset()
      current.ink.passesPointer(false)
    }
    current.pill.penOn = machine.ink == .drag
    if machine.ink == .hover { hover(NSEvent.mouseLocation, current, t) }
    current.animating = true
  }

  private func hover(_ mouse: CGPoint, _ current: AnnotateCurrent, _ t: Int) {
    guard AnnotateGeometry.contains(mouse, current.screen) else {
      current.strokes.end(tMs: t)
      return
    }
    let point = AnnotateGeometry.local(mouse, on: current.screen)
    if current.strokes.live == nil {
      current.strokes.begin(.hover, at: point, tMs: t)
    } else {
      current.strokes.add(point, tMs: t)
    }
  }

  private func mouse(_ kind: AnnotateMouse, _ point: CGPoint) {
    guard let current, machine.phase == .recording else { return }
    let now = nowTicks()
    machine.pointerMoved(now: now.seconds)
    guard machine.ink == .drag else { return }
    let t = tMs(current, now.ticks)
    switch kind {
    case .down: current.strokes.begin(.drag, at: point, tMs: t)
    case .dragged: current.strokes.add(point, tMs: t)
    case .up:
      current.strokes.add(point, tMs: t)
      current.strokes.end(tMs: t)
    }
    current.animating = true
  }

  private func capsuleAction(_ target: AnnotateCapsuleTarget) {
    guard current != nil else { return }
    let now = nowTicks()
    let action: AnnotateCapsuleAction
    switch target {
    case .pen: action = .pen
    case .stop: action = .stop
    case .discard: action = .discard
    case .confirmDiscard: action = .confirmDiscard
    case .keep: action = .keep
    }
    let ink = machine.ink
    machine.capsule(action, now: now.seconds)
    if machine.ink != ink { inkChanged(at: now.ticks) }
    current?.pill.asking = machine.asking
    flush()
  }

  private func tick() {
    guard let current, machine.phase == .recording else {
      // Nothing records: no window of a recording may stay.
      if timer != nil && machine.phase != .recording { tearDown() }
      return
    }
    let now = nowTicks()
    let t = tMs(current, now.ticks)
    let mouse = NSEvent.mouseLocation
    if mouse != current.lastMouse {
      current.lastMouse = mouse
      machine.pointerMoved(now: now.seconds)
      if machine.ink == .hover {
        hover(mouse, current, t)
        current.animating = true
      }
    }
    for effect in machine.tick(now: now.seconds) {
      switch effect {
      case .inkOff:
        inkChanged(at: now.ticks)
      case .questionClosed:
        current.pill.asking = false
      case .maxDuration:
        finishByItself(reason: "max_duration")
        return
      }
    }
    let animating = AnnotateInkLook.animating(current.strokes, nowMs: t)
    if animating || current.animating {
      current.ink.inkView.scene = AnnotateInkScene(
        strokes: AnnotateInkLook.visible(current.strokes, nowMs: t).map { shown in
          (points: shown.stroke.points.map { CGPoint(x: $0.x, y: $0.y) }, alpha: shown.alpha)
        },
        glow: AnnotateInkLook.glow(ms: Double(t)))
      current.ink.inkView.needsDisplay = true
    }
    current.animating = animating
    if current.scroll.tick(now: now.seconds) {
      current.ink.passesPointer(false)
    }
    let elapsed = machine.elapsedMs(now: now.seconds)
    current.pill.clock = AnnotateMachine.clock(ms: elapsed)
    flush()
  }

  /// Publishes the machine's events and status for the host's polls.
  private func flush() {
    let events = machine.drainEvents()
    let status = AnnotateStatus(
      phase: machine.phase, elapsedMs: machine.elapsedMs(now: nowTicks().seconds), ink: machine.ink,
      resultPending: false)
    mailbox.update { mailbox in
      mailbox.events.append(contentsOf: events)
      mailbox.status = status
    }
  }
}
