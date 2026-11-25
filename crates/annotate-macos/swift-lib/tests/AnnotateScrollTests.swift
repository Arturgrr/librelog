import CoreGraphics
import XCTest

@testable import AnnotateMacOS

final class AnnotateScrollTests: XCTestCase {
  func testAWheelGoesThroughUntilItsNotchesStop() {
    var pass = AnnotateScrollPass()
    XCTAssertTrue(
      pass.reachedOverlay(phase: .unphased, momentum: .unphased, now: 0, drawing: false))
    XCTAssertTrue(pass.passing)
    pass.observe(phase: .unphased, momentum: .unphased, now: 0.1)
    XCTAssertFalse(pass.tick(now: 0.25))
    XCTAssertTrue(pass.tick(now: 0.1 + AnnotateScrollPass.quiet + 0.01))
    XCTAssertFalse(pass.passing)
  }

  func testATrackpadScrollKeepsThePointerThroughItsMomentum() {
    var pass = AnnotateScrollPass()
    XCTAssertTrue(pass.reachedOverlay(phase: .began, momentum: .unphased, now: 0, drawing: false))
    pass.observe(phase: .changed, momentum: .unphased, now: 0.05)
    pass.observe(phase: .ended, momentum: .unphased, now: 0.1)
    pass.observe(phase: .unphased, momentum: .began, now: 0.12)
    // The momentum pauses longer than a wheel's quiet: still coasting.
    XCTAssertFalse(pass.tick(now: 0.5))
    pass.observe(phase: .unphased, momentum: .changed, now: 0.6)
    pass.observe(phase: .unphased, momentum: .ended, now: 0.9)
    XCTAssertFalse(pass.tick(now: 0.95))
    XCTAssertTrue(pass.tick(now: 0.9 + AnnotateScrollPass.quiet + 0.01))
  }

  func testAStrokeBeingDrawnKeepsThePointer() {
    var pass = AnnotateScrollPass()
    XCTAssertFalse(pass.reachedOverlay(phase: .began, momentum: .unphased, now: 0, drawing: true))
    XCTAssertFalse(pass.passing)
    // Seen elsewhere without passing, a scroll changes nothing.
    pass.observe(phase: .changed, momentum: .unphased, now: 0.1)
    XCTAssertFalse(pass.tick(now: 5))
  }

  func testAGestureWhoseEndNeverCameGivesThePointerBack() {
    var pass = AnnotateScrollPass()
    XCTAssertTrue(pass.reachedOverlay(phase: .began, momentum: .unphased, now: 0, drawing: false))
    XCTAssertFalse(pass.tick(now: AnnotateScrollPass.lost - 0.1))
    XCTAssertTrue(pass.tick(now: AnnotateScrollPass.lost + 0.01))
  }

  func testTheScrollGoesToTheFrontmostAppWindowUnderThePointer() {
    let point = CGPoint(x: 400, y: 300)
    let screen = CGRect(x: 0, y: 0, width: 1512, height: 982)
    let corner = CGRect(x: 900, y: 0, width: 100, height: 100)
    let app = CGRect(x: 0, y: 0, width: 800, height: 600)
    let windows = [
      // The ink overlay and the capsule.
      AnnotateWindowInfo(number: 1, pid: 9, layer: 26, alpha: 1, bounds: screen),
      AnnotateWindowInfo(number: 2, pid: 9, layer: 27, alpha: 1, bounds: screen),
      // The menu bar's level, an invisible window, one elsewhere.
      AnnotateWindowInfo(number: 3, pid: 4, layer: 25, alpha: 1, bounds: screen),
      AnnotateWindowInfo(number: 4, pid: 5, layer: 0, alpha: 0, bounds: screen),
      AnnotateWindowInfo(number: 5, pid: 6, layer: 0, alpha: 1, bounds: corner),
      AnnotateWindowInfo(number: 6, pid: 7, layer: 0, alpha: 1, bounds: app),
      AnnotateWindowInfo(number: 7, pid: 8, layer: 0, alpha: 1, bounds: screen),
    ]
    let target = AnnotateScrollTarget.window(at: point, in: windows, own: [1, 2])
    XCTAssertEqual(target?.number, 6)
    XCTAssertNil(AnnotateScrollTarget.window(at: point, in: Array(windows.prefix(4)), own: [1, 2]))
  }
}
