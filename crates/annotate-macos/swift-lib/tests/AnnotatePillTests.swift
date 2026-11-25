import CoreGraphics
import XCTest

@testable import AnnotateMacOS

final class AnnotatePillTests: XCTestCase {
  private let tick = 1.0 / 60

  /// Steps `model` at 60 Hz from `start` to `end` seconds.
  private func run(_ model: inout AnnotatePillModel, from start: Double, to end: Double) {
    var now = start
    while now + tick <= end {
      now += tick
      model.step(now: now, dt: tick, reduceMotion: false)
    }
  }

  /// A capsule recording since 0 s, settled at 1 s.
  private func recording() -> AnnotatePillModel {
    var model = AnnotatePillModel()
    model.show(now: 0, reduceMotion: false)
    run(&model, from: 0, to: 1)
    return model
  }

  /// A recording stopped at 1 s, processing until 61 s.
  private func processing() -> AnnotatePillModel {
    var model = recording()
    model.set(.processing, now: 1)
    run(&model, from: 1, to: 61)
    return model
  }

  func testAStoppedRecordingShowsProcessingUntilReadyThenGoesAway() {
    var model = processing()
    // As long as the annotation takes, narrowed to the HUD's processing width.
    XCTAssertEqual(model.stage, .processing)
    XCTAssertTrue(model.shown)
    XCTAssertEqual(model.pose.width, AnnotatePillLook.processingWidth, accuracy: 0.5)
    XCTAssertEqual(model.pose.opacity, 1, accuracy: 0.01)

    model.set(.ready, now: 61)
    run(&model, from: 61, to: 61.05)
    XCTAssertEqual(model.stage, .ready)
    run(&model, from: 61.05, to: 61 + AnnotatePillLook.readyVisible - 0.05)
    XCTAssertTrue(model.shown)
    run(&model, from: 61 + AnnotatePillLook.readyVisible - 0.05, to: 63)
    XCTAssertFalse(model.shown)
    XCTAssertFalse(model.active)
  }

  func testAFastFinishStillShowsProcessingBeforeReady() {
    var model = recording()
    model.set(.processing, now: 1)
    model.set(.ready, now: 1.05)
    run(&model, from: 1, to: 1.15)
    XCTAssertEqual(model.stage, .processing)
    run(&model, from: 1.15, to: 1 + AnnotatePillLook.processingMinimum + 0.05)
    XCTAssertEqual(model.stage, .ready)
    XCTAssertTrue(model.shown)
  }

  func testAFailureShowsInsteadOfDisappearingAndStaysLongerThanReady() {
    var model = processing()
    model.set(.failed, now: 61)
    let pastReady = 61 + AnnotatePillLook.readyVisible + 0.2
    run(&model, from: 61, to: pastReady)
    XCTAssertEqual(model.stage, .failed)
    XCTAssertTrue(model.shown)
    run(&model, from: pastReady, to: 61 + AnnotatePillLook.failedVisible + 1)
    XCTAssertFalse(model.active)
  }

  func testADiscardedAnnotationFadesAtOnceAndALateOutcomeChangesNothing() {
    var model = processing()
    model.hide()
    run(&model, from: 61, to: 62)
    XCTAssertFalse(model.active)
    model.set(.ready, now: 62)
    run(&model, from: 62, to: 63)
    XCTAssertFalse(model.active)
    XCTAssertEqual(model.stage, .processing)
  }

  func testACapsuleNobodyAnswersGoesAfterItsLimit() {
    var model = recording()
    model.set(.processing, now: 1)
    run(&model, from: 1, to: AnnotatePillLook.processingLimit)
    XCTAssertTrue(model.shown)
    run(&model, from: AnnotatePillLook.processingLimit, to: AnnotatePillLook.processingLimit + 3)
    XCTAssertFalse(model.active)
  }

  func testThePillSitsUnderTheMenuBarOrTheNotchAsTheDictationHudDoes() {
    let size = AnnotatePillLayout.panelSize
    let margin = AnnotatePillLook.margin
    /// The plate's top edge for a panel at `origin`.
    func plateTop(_ origin: CGPoint) -> CGFloat { origin.y + size.height - margin }

    let flat = AnnotateScreen(
      id: 2, frame: CGRect(x: -2560, y: 0, width: 2560, height: 1440),
      visibleFrame: CGRect(x: -2560, y: 0, width: 2560, height: 1416), safeTop: 0, scale: 1)
    let origin = AnnotatePillLayout.panelOrigin(on: flat)
    XCTAssertEqual(plateTop(origin), 1416 - 6)
    XCTAssertEqual(origin.x + size.width / 2, -1280)

    // A full-screen space hides the menu bar; the plate still clears the
    // notch.
    let notched = AnnotateScreen(
      id: 1, frame: CGRect(x: 0, y: 0, width: 1512, height: 982),
      visibleFrame: CGRect(x: 0, y: 0, width: 1512, height: 982), safeTop: 32, scale: 2)
    XCTAssertEqual(plateTop(AnnotatePillLayout.panelOrigin(on: notched)), 982 - 32 - 6)
  }

  func testEachPillButtonAnswersOnlyInsideItselfAndTheQuestionFitsItsWords() {
    let recording = AnnotatePillLayout.make(asking: false)
    XCTAssertLessThanOrEqual(recording.width, AnnotatePillLook.maxWidth)
    XCTAssertEqual(recording.targets.map { $0.0 }, [.pen, .stop, .discard])
    for (target, rect) in recording.targets {
      XCTAssertEqual(recording.target(at: CGPoint(x: rect.midX, y: rect.midY)), target)
      XCTAssertGreaterThanOrEqual(rect.minY, 0)
      XCTAssertLessThanOrEqual(rect.maxX, recording.width)
    }
    XCTAssertNil(recording.target(at: CGPoint(x: recording.clock.midX, y: 13)))

    // Portuguese words are longer: the plate widens and nothing overlaps.
    let asking = AnnotatePillLayout.make(asking: true, labels: (82, 66, 48))
    let rects = asking.targets.map { $0.1 }
    XCTAssertEqual(asking.targets.map { $0.0 }, [.confirmDiscard, .keep])
    XCTAssertLessThanOrEqual(asking.question.maxX, rects[0].minX)
    XCTAssertLessThanOrEqual(rects[0].maxX, rects[1].minX)
    XCTAssertGreaterThanOrEqual(rects[0].width, 66)
    XCTAssertGreaterThan(asking.width, recording.width)
  }
}
