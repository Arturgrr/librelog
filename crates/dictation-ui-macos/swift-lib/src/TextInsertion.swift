import Carbon
import Cocoa
import IOKit
import SwiftRs

private enum Message {
  static let accessibility =
    "Enable Librelog in System Settings > Privacy & Security > Accessibility to insert dictation into other apps."
  static let secureAtStart =
    "Dictation is unavailable in password fields and while Secure Keyboard Entry is on."
  static let secureAtInsert =
    "Dictation is unavailable in password fields and while Secure Keyboard Entry is on. Copy your last dictation from Settings > Dictation."
  static let shortcutHeld =
    "Release the shortcut keys before inserting text. Copy your last dictation from Settings > Dictation."
  static let failed =
    "Could not insert text. Copy your last dictation from Settings > Dictation."
  static let clipboard =
    "Could not preserve the clipboard. Copy your last dictation from Settings > Dictation."
  static let prepare = "Could not prepare dictation for insertion."
  static let focusChanged = "The focused field changed after the dictation was inserted."
}

final class DictationPasteProvider: NSObject, NSPasteboardItemDataProvider {
  let text: String
  /// Signalled each time an app asks for the text, which it does once Command-V reaches it.
  let read = DispatchSemaphore(value: 0)
  var didRead: (() -> Void)?
  init(text: String) { self.text = text }
  func pasteboard(
    _ pasteboard: NSPasteboard?, item: NSPasteboardItem,
    provideDataForType type: NSPasteboard.PasteboardType
  ) {
    item.setString(text, forType: type)
    read.signal()
    // The pasteboard keeps the text once provided, so later reads never reach the provider.
    // Restore a moment after the first one: apps may read again, or a clipboard manager may have
    // read just before the target.
    guard let didRead else { return }
    self.didRead = nil
    DispatchQueue.main.asyncAfter(deadline: .now() + 1, execute: didRead)
  }
}

extension AXUIElement {
  fileprivate func value(_ attribute: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(self, attribute as CFString, &value) == .success
      ? value : nil
  }

  fileprivate func string(_ attribute: String) -> String? { value(attribute) as? String }

  fileprivate func int(_ attribute: String) -> Int? { (value(attribute) as? NSNumber)?.intValue }

  fileprivate func element(_ attribute: String) -> AXUIElement? {
    guard let value = value(attribute), CFGetTypeID(value) == AXUIElementGetTypeID() else {
      return nil
    }
    let element = unsafeBitCast(value, to: AXUIElement.self)
    AXUIElementSetMessagingTimeout(element, 1)
    return element
  }

  fileprivate func range(_ attribute: String) -> CFRange? {
    guard let value = value(attribute), CFGetTypeID(value) == AXValueGetTypeID() else {
      return nil
    }
    var range = CFRange()
    return AXValueGetValue(unsafeBitCast(value, to: AXValue.self), .cfRange, &range) ? range : nil
  }

  fileprivate func isSettable(_ attribute: String) -> Bool {
    var settable = DarwinBoolean(false)
    return AXUIElementIsAttributeSettable(self, attribute as CFString, &settable) == .success
      && settable.boolValue
  }

  /// Whether the element belongs to a web page. Too deep to tell counts as yes.
  fileprivate var isInWebArea: Bool {
    var current: AXUIElement? = self
    for _ in 0..<32 {
      guard let node = current else { return false }
      let role = node.string(kAXRoleAttribute)
      if role == "AXWebArea" { return true }
      if role == kAXWindowRole || role == kAXApplicationRole { return false }
      current = node.element(kAXParentAttribute)
    }
    return true
  }
}

extension FocusedText {
  fileprivate init(_ element: AXUIElement) {
    role = element.string(kAXRoleAttribute)
    guard isTextControl else { return }
    selectedTextSettable = element.isSettable(kAXSelectedTextAttribute)
    guard selectedTextSettable else { return }
    characterCount = element.int(kAXNumberOfCharactersAttribute)
    inWebArea = element.isInWebArea
  }
}

private enum SecureInput {
  /// Whether `pid` turned on secure keyboard entry: password fields do while focused, and so does
  /// a terminal's Secure Keyboard Entry. Another app holding it, often by mistake, does not block
  /// typing into this one.
  static func isOn(in pid: pid_t) -> Bool {
    guard IsSecureEventInputEnabled() else { return false }
    return owner().map { $0 == pid } ?? true
  }

  /// The process macOS records as having turned secure keyboard entry on for this login session.
  private static func owner() -> pid_t? {
    let entry = IORegistryEntryFromPath(kIOMainPortDefault, "IOService:/IOResources")
    guard entry != 0 else { return nil }
    defer { IOObjectRelease(entry) }
    guard
      let sessions = IORegistryEntryCreateCFProperty(
        entry, "IOConsoleUsers" as CFString, kCFAllocatorDefault, 0)?
        .takeRetainedValue() as? [[String: Any]],
      let session = sessions.first(where: {
        ($0["kCGSSessionUserIDKey"] as? NSNumber)?.uint32Value == getuid()
      }),
      let pid = session["kCGSSessionSecureInputPID"] as? NSNumber
    else { return nil }
    return pid.int32Value
  }
}

/// Where keyboard input goes now: the focused app, and its focused element when that decides how
/// to insert.
private struct Focus {
  let pid: pid_t
  let route: InsertionRoute?
  let element: AXUIElement?

  static func current() -> Focus? {
    guard let pid = focusedPID() else { return nil }
    let running = NSRunningApplication(processIdentifier: pid)
    let app = AppTraits(
      bundleID: running?.bundleIdentifier, bundleURL: running?.bundleURL,
      isCurrentProcess: pid == getpid())
    if let route = InsertionRoute.forApp(app) {
      return Focus(pid: pid, route: route, element: nil)
    }
    let application = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(application, 1)
    return Focus(pid: pid, route: nil, element: application.element(kAXFocusedUIElementAttribute))
  }

  /// Unlike the frontmost app, Accessibility's focused app follows panels such as Spotlight that
  /// take keys without activating.
  private static func focusedPID() -> pid_t? {
    let system = AXUIElementCreateSystemWide()
    AXUIElementSetMessagingTimeout(system, 1)
    var pid: pid_t = 0
    if let app = system.element(kAXFocusedApplicationAttribute),
      AXUIElementGetPid(app, &pid) == .success
    {
      return pid
    }
    return NSWorkspace.shared.frontmostApplication?.processIdentifier
  }

  var isSecure: Bool {
    SecureInput.isOn(in: pid) || element?.string(kAXSubroleAttribute) == kAXSecureTextFieldSubrole
  }

  func resolvedRoute() -> InsertionRoute {
    route ?? InsertionRoute.forFocus(element.map(FocusedText.init))
  }
}

private final class DictationTarget {
  static let shared = DictationTarget()
  var token = ""
  var clipboardSnapshot: [NSPasteboardItem]?
  var clipboardChangeCount: Int?
  var clipboardProvider: DictationPasteProvider?
  /// The paste the last insertion made, until something waits for it to be read.
  var pendingPaste: DictationPasteProvider?
  /// Where the last insertion went whole, until Return is pressed there.
  var lastInsertion: (pid: pid_t, element: AXUIElement?)?

  /// Text goes wherever focus is when the transcript is ready, so starting only checks what can
  /// already rule dictation out.
  func capture() -> String {
    token = ""
    guard AXIsProcessTrusted() else { return Message.accessibility }
    if Focus.current()?.isSecure == true { return Message.secureAtStart }
    token = UUID().uuidString
    return "ok:" + token
  }

  func takePendingPaste() -> DictationPasteProvider? {
    defer { pendingPaste = nil }
    return pendingPaste
  }

  func insert(token: String, text: String) -> String {
    pendingPaste = nil
    lastInsertion = nil
    guard !token.isEmpty, token == self.token else { return Message.failed }
    self.token = ""
    // Posting keystrokes to other apps needs Accessibility too.
    guard AXIsProcessTrusted() else { return Message.accessibility }
    guard let focus = Focus.current() else { return Message.failed }
    if focus.isSecure { return Message.secureAtInsert }

    let result: String
    switch focus.resolvedRoute() {
    case .accessibility:
      if let element = focus.element, insertWithAccessibility(element, text: text) {
        result = ""
      } else {
        result = paste(text)
      }
    case .paste:
      result = paste(text)
    case .type:
      result = KeyboardInput.type(text) ? "" : Message.failed
    }
    if result.isEmpty { lastInsertion = (pid: focus.pid, element: focus.element) }
    return result
  }

  /// Presses Return where the last insertion went, only while the same app, and the same field
  /// where Accessibility tells fields apart, still has focus outside secure input.
  func pressReturn() -> String {
    defer { lastInsertion = nil }
    guard let last = lastInsertion, AXIsProcessTrusted(), let focus = Focus.current(),
      focus.pid == last.pid, !focus.isSecure
    else { return Message.focusChanged }
    if let element = last.element {
      guard let now = focus.element, CFEqual(element, now) else { return Message.focusChanged }
    }
    guard let keystroke = KeyboardInput.returnKeystroke() else { return Message.failed }
    KeyboardInput.post(keystroke)
    return ""
  }

  /// Sets the selected text, then checks the field changed: some apps report success and ignore it.
  private func insertWithAccessibility(_ element: AXUIElement, text: String) -> Bool {
    let count = element.int(kAXNumberOfCharactersAttribute)
    let range = element.range(kAXSelectedTextRangeAttribute)
    guard
      AXUIElementSetAttributeValue(element, kAXSelectedTextAttribute as CFString, text as CFString)
        == .success
    else { return false }
    if let after = element.int(kAXNumberOfCharactersAttribute), after != count { return true }
    if let range, let after = element.range(kAXSelectedTextRangeAttribute),
      after.location != range.location || after.length != range.length
    {
      return true
    }
    return false
  }

  private func paste(_ text: String) -> String {
    guard let keystroke = KeyboardInput.pasteKeystroke() else { return Message.failed }
    let pasteboard = NSPasteboard.general
    if clipboardChangeCount != pasteboard.changeCount {
      clipboardSnapshot = nil
    }
    var saved = clipboardSnapshot ?? []
    if clipboardSnapshot == nil {
      for original in pasteboard.pasteboardItems ?? [] {
        let copy = NSPasteboardItem()
        for type in original.types {
          guard let data = original.data(forType: type), copy.setData(data, forType: type) else {
            return Message.clipboard
          }
        }
        saved.append(copy)
      }
    }
    let item = NSPasteboardItem()
    let provider = DictationPasteProvider(text: text)
    item.setDataProvider(provider, forTypes: [.string])
    // nspasteboard.org markers: clipboard managers don't record the transcript.
    item.setString("", forType: NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"))
    item.setString("", forType: NSPasteboard.PasteboardType("org.nspasteboard.TransientType"))
    // Keeps the transcript off Universal Clipboard.
    let clearedCount = pasteboard.prepareForNewContents(with: .currentHostOnly)
    guard pasteboard.writeObjects([item]) else {
      clipboardSnapshot = saved
      clipboardChangeCount = clearedCount
      restoreClipboard(saved, expectedChangeCount: clearedCount, retries: 2)
      return Message.prepare
    }
    let changeCount = pasteboard.changeCount
    clipboardSnapshot = saved
    clipboardChangeCount = changeCount
    clipboardProvider = provider
    pendingPaste = provider
    provider.didRead = {
      self.restoreClipboard(saved, expectedChangeCount: changeCount, retries: 2)
    }
    // A destination may ignore Command-V and never request the promised text.
    DispatchQueue.main.asyncAfter(deadline: .now() + 30) {
      self.restoreClipboard(saved, expectedChangeCount: changeCount, retries: 2)
    }
    KeyboardInput.post(keystroke)
    return ""
  }

  private func restoreClipboard(_ saved: [NSPasteboardItem], expectedChangeCount: Int, retries: Int)
  {
    guard clipboardChangeCount == expectedChangeCount else { return }
    let pasteboard = NSPasteboard.general
    if pasteboard.changeCount == expectedChangeCount {
      let count = pasteboard.clearContents()
      if !saved.isEmpty && !pasteboard.writeObjects(saved) {
        guard pasteboard.changeCount == count else {
          clipboardSnapshot = nil
          clipboardChangeCount = nil
          clipboardProvider = nil
          return
        }
        clipboardChangeCount = count
        if retries > 0 {
          DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            self.restoreClipboard(saved, expectedChangeCount: count, retries: retries - 1)
          }
        } else {
          NSLog("Could not restore dictation clipboard; keeping its owned snapshot for recovery.")
        }
        return
      }
    }
    clipboardSnapshot = nil
    clipboardChangeCount = nil
    clipboardProvider = nil
  }

}

/// The part of a field a read keeps around its selection.
enum FieldWindow {
  /// `window` UTF-16 units on each side of `selection`, clamped to the field and never splitting
  /// a surrogate pair at either edge. `units` is the whole value when it was read.
  static func bounds(units: [UInt16]?, length: Int, selection: CFRange, window: Int) -> (Int, Int) {
    let end = selection.location + selection.length
    var from = max(0, selection.location - window)
    var to = min(length, end + window)
    if let units {
      if from > 0, from < selection.location, UTF16.isTrailSurrogate(units[from]) { from += 1 }
      if to < units.count, to > end, UTF16.isLeadSurrogate(units[to - 1]) { to -= 1 }
    }
    return (from, to)
  }
}

/// Reads of the focused text field and its window for the options a person turned on. Secure
/// fields, Secure Keyboard Entry and Librelog's own windows are never read.
private enum FieldReader {
  /// The focused text element. Without `deep`, only where insertion itself asks Accessibility;
  /// with it, also web views and Electron apps, whose accessibility trees cost to build.
  static func element(deep: Bool) -> AXUIElement? {
    guard AXIsProcessTrusted(), let focus = Focus.current(), !focus.isSecure else { return nil }
    var element = focus.element
    if element == nil, deep, focus.pid != getpid() {
      let running = NSRunningApplication(processIdentifier: focus.pid)
      let app = AppTraits(
        bundleID: running?.bundleIdentifier, bundleURL: nil, isCurrentProcess: false)
      // A terminal's Accessibility text is its screen, not a field.
      if !app.isTerminal {
        let application = AXUIElementCreateApplication(focus.pid)
        AXUIElementSetMessagingTimeout(application, 1)
        element = application.element(kAXFocusedUIElementAttribute)
      }
    }
    guard let element, element.string(kAXSubroleAttribute) != kAXSecureTextFieldSubrole,
      FocusedText(role: element.string(kAXRoleAttribute)).isTextControl
        || element.isSettable(kAXSelectedTextAttribute)
    else { return nil }
    return element
  }

  static func facts() -> String {
    guard AXIsProcessTrusted(), let focus = Focus.current() else { return "" }
    let running = NSRunningApplication(processIdentifier: focus.pid)
    let application = AXUIElementCreateApplication(focus.pid)
    AXUIElementSetMessagingTimeout(application, 1)
    var facts: [String: Any] = ["secure": focus.isSecure]
    facts["bundleId"] = running?.bundleIdentifier
    facts["appName"] = running?.localizedName
    facts["title"] = application.element(kAXFocusedWindowAttribute)?.string(kAXTitleAttribute)
    return json(facts)
  }

  /// The field around its selection, `window` UTF-16 units each side, as JSON; "" if unreadable.
  static func read(window: Int, deep: Bool) -> String {
    guard let element = element(deep: deep),
      let range = element.range(kAXSelectedTextRangeAttribute)
    else { return "" }
    var length = element.int(kAXNumberOfCharactersAttribute)
    var units: [UInt16]?
    if length.map({ $0 <= 200_000 }) ?? true, let value = element.string(kAXValueAttribute) {
      units = Array(value.utf16)
      length = units?.count
    }
    guard let length, range.location >= 0, range.length >= 0, range.length <= 100_000,
      range.location + range.length <= length
    else { return "" }
    let end = range.location + range.length
    let (from, to) = FieldWindow.bounds(
      units: units, length: length, selection: range, window: window)
    func slice(_ start: Int, _ stop: Int) -> String? {
      if start >= stop { return "" }
      if let units { return String(decoding: units[start..<stop], as: UTF16.self) }
      var part = CFRange(location: start, length: stop - start)
      guard let value = AXValueCreate(.cfRange, &part) else { return nil }
      var text: CFTypeRef?
      guard
        AXUIElementCopyParameterizedAttributeValue(
          element, kAXStringForRangeParameterizedAttribute as CFString, value, &text) == .success
      else { return nil }
      return text as? String
    }
    guard let before = slice(from, range.location), let selected = slice(range.location, end),
      let after = slice(end, to)
    else { return "" }
    return json([
      "length": length, "start": range.location, "before": before, "selected": selected,
      "after": after,
    ])
  }

  /// Static text of the field's window in reading order, the newest `max` characters, within a
  /// small time and node budget. Secure fields and the field itself are skipped.
  static func windowText(max: Int) -> String {
    guard let element = element(deep: true), let window = element.element(kAXWindowAttribute)
    else { return "" }
    let deadline = Date().addingTimeInterval(0.25)
    var parts: [String] = []
    var visited = 0
    func walk(_ node: AXUIElement, depth: Int) {
      guard visited < 800, depth < 30, Date() < deadline, !CFEqual(node, element) else { return }
      visited += 1
      AXUIElementSetMessagingTimeout(node, 0.2)
      if node.string(kAXSubroleAttribute) == kAXSecureTextFieldSubrole { return }
      if node.string(kAXRoleAttribute) == kAXStaticTextRole,
        let text = node.string(kAXValueAttribute)?.trimmingCharacters(in: .whitespacesAndNewlines),
        !text.isEmpty
      {
        parts.append(text)
      }
      guard let children = node.value(kAXChildrenAttribute) as? [AXUIElement] else { return }
      for child in children { walk(child, depth: depth + 1) }
    }
    walk(window, depth: 0)
    return String(parts.joined(separator: "\n").suffix(max))
  }

  private static func json(_ object: [String: Any]) -> String {
    guard let data = try? JSONSerialization.data(withJSONObject: object) else { return "" }
    return String(decoding: data, as: UTF8.self)
  }
}

private func onMain<T>(_ work: () -> T) -> T {
  Thread.isMainThread ? work() : DispatchQueue.main.sync(execute: work)
}

@_cdecl("_capture_dictation_target")
public func captureDictationTarget() -> SRString {
  SRString(onMain { DictationTarget.shared.capture() })
}

@_cdecl("_insert_dictation_text")
public func insertDictationText(target: SRString, text: SRString) -> SRString {
  let token = target.toString()
  let text = text.toString()
  // Rust calls this off the main thread, so waiting here doesn't stall the indicator.
  guard KeyboardInput.waitForModifierRelease() else { return SRString(Message.shortcutHeld) }
  return SRString(onMain { DictationTarget.shared.insert(token: token, text: text) })
}

@_cdecl("_press_dictation_return")
public func pressDictationReturn() -> SRString {
  // A held Shift would make it a new line, Command or Option another shortcut.
  guard KeyboardInput.waitForModifierRelease() else { return SRString(Message.shortcutHeld) }
  return SRString(onMain { DictationTarget.shared.pressReturn() })
}

@_cdecl("_dictation_field_facts")
public func dictationFieldFacts() -> SRString {
  SRString(onMain { FieldReader.facts() })
}

@_cdecl("_dictation_field_read")
public func dictationFieldRead(window: Int, deep: Bool) -> SRString {
  SRString(onMain { FieldReader.read(window: window, deep: deep) })
}

@_cdecl("_dictation_window_text")
public func dictationWindowText(max: Int) -> SRString {
  SRString(onMain { FieldReader.windowText(max: max) })
}

/// Waits up to `timeout` seconds for the app to read what the last insertion pasted, so the next
/// one doesn't replace it first. True once it has, or when that insertion pasted nothing.
@_cdecl("_wait_for_dictation_paste_read")
public func waitForDictationPasteRead(timeout: Double) -> Bool {
  // The read arrives on the main thread, which waiting there would hold up.
  if Thread.isMainThread { return false }
  guard let paste = onMain({ DictationTarget.shared.takePendingPaste() }) else { return true }
  return paste.read.wait(timeout: .now() + timeout) == .success
}
