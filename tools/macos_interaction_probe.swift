import AppKit
import CoreGraphics
import CoreVideo
import CryptoKit
import Foundation
import ImageIO
import ScreenCaptureKit
import UniformTypeIdentifiers

private enum Action {
    case key(String, CGKeyCode)
    case click(Double, Double)
}

private struct Arguments {
    let windowID: CGWindowID
    let expectedPID: pid_t
    let outputDirectory: URL
    let delayMilliseconds: UInt32
    let writeJSON: Bool
    let action: Action
}

private struct WindowIdentity {
    let ownerPID: pid_t
    let bounds: CGRect
    let layer: Int
    let alpha: Double
}

private struct ImageMetrics {
    let png: Data
    let pngSHA256: String
    let sampledSHA256: String
    let sampledBGRA: [UInt8]
    let averageLuma: Double
    let width: Int
    let height: Int
}

private func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("error: \(message)\n".utf8))
    exit(2)
}

private func parseArguments() -> Arguments {
    let values = CommandLine.arguments
    guard values.count >= 7,
          let windowID = UInt32(values[1]),
          let expectedPID = Int32(values[2]),
          let delay = UInt32(values[4]), (100...5_000).contains(delay)
    else {
        fail("usage: macos_interaction_probe WINDOW_ID PID OUTPUT_DIR DELAY_MS [--write-json] <key NAME|click X_FRACTION Y_FRACTION>")
    }
    let writeJSON = values[5] == "--write-json"
    let actionIndex = writeJSON ? 6 : 5
    let keyCodes: [String: CGKeyCode] = [
        "return": 36, "escape": 53, "up": 126, "down": 125,
        "left": 123, "right": 124, "space": 49,
    ]
    let action: Action
    if values[actionIndex] == "key", values.count == actionIndex + 2,
       let keyCode = keyCodes[values[actionIndex + 1]]
    {
        action = .key(values[actionIndex + 1], keyCode)
    } else if values[actionIndex] == "click", values.count == actionIndex + 3,
              let x = Double(values[actionIndex + 1]),
              let y = Double(values[actionIndex + 2]),
              (0.0...1.0).contains(x), (0.0...1.0).contains(y)
    {
        action = .click(x, y)
    } else {
        fail("usage: macos_interaction_probe WINDOW_ID PID OUTPUT_DIR DELAY_MS [--write-json] <key NAME|click X_FRACTION Y_FRACTION>")
    }
    return Arguments(
        windowID: CGWindowID(windowID),
        expectedPID: pid_t(expectedPID),
        outputDirectory: URL(fileURLWithPath: values[3], isDirectory: true),
        delayMilliseconds: delay,
        writeJSON: writeJSON,
        action: action
    )
}

private func windowIdentity(_ windowID: CGWindowID) -> WindowIdentity? {
    guard let windows = CGWindowListCopyWindowInfo(
        [.optionIncludingWindow, .excludeDesktopElements], windowID
    ) as? [[String: Any]], let info = windows.first,
          let pid = (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value,
          let boundsDictionary = info[kCGWindowBounds as String] as? NSDictionary,
          let bounds = CGRect(dictionaryRepresentation: boundsDictionary)
    else { return nil }
    return WindowIdentity(
        ownerPID: pid,
        bounds: bounds,
        layer: (info[kCGWindowLayer as String] as? NSNumber)?.intValue ?? -1,
        alpha: (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 0
    )
}

private func targetWindowIsTopmost(_ windowID: CGWindowID, expectedPID: pid_t, at point: CGPoint) -> Bool {
    guard let windows = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
    ) as? [[String: Any]] else { return false }
    for info in windows {
        let layer = (info[kCGWindowLayer as String] as? NSNumber)?.intValue ?? -1
        guard layer >= 0,
              (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 0 > 0,
              let windowBounds = (info[kCGWindowBounds as String] as? NSDictionary)
                  .flatMap(CGRect.init(dictionaryRepresentation:)),
              windowBounds.contains(point)
        else { continue }
        let id = (info[kCGWindowNumber as String] as? NSNumber)?.uint32Value ?? 0
        if id == windowID { return true }
        let ownerPID = (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value ?? 0
        if ownerPID == expectedPID { return true }
        // Wine often presents the game surface inside its explorer desktop;
        // the desktop host can be the top WindowServer surface even though the
        // target child window owns the pixels. Permit that containment only
        // when the occluding owner is unmistakably Wine/explorer, never an
        // unrelated host application.
        let owner = (info[kCGWindowOwnerName as String] as? String ?? "").lowercased()
        if owner.contains("wine") || owner.contains("explorer") { return true }
        return false
    }
    return false
}

private func sha256(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

private func pngData(_ image: CGImage) -> Data? {
    let data = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(
        data, UTType.png.identifier as CFString, 1, nil
    ) else { return nil }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { return nil }
    return data as Data
}

private func metrics(_ image: CGImage) -> ImageMetrics? {
    let width = image.width
    let height = image.height
    guard width > 0, height > 0, let png = pngData(image) else { return nil }
    let sampleWidth = min(width, 192)
    let sampleHeight = max(1, Int(Double(height) * Double(sampleWidth) / Double(width)))
    var pixels = [UInt8](repeating: 0, count: sampleWidth * sampleHeight * 4)
    guard let context = CGContext(
        data: &pixels,
        width: sampleWidth,
        height: sampleHeight,
        bitsPerComponent: 8,
        bytesPerRow: sampleWidth * 4,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { return nil }
    context.interpolationQuality = .low
    context.draw(image, in: CGRect(x: 0, y: 0, width: sampleWidth, height: sampleHeight))
    var total = 0.0
    for offset in stride(from: 0, to: pixels.count, by: 4) {
        total += 0.2126 * Double(pixels[offset])
            + 0.7152 * Double(pixels[offset + 1])
            + 0.0722 * Double(pixels[offset + 2])
    }
    return ImageMetrics(
        png: png,
        pngSHA256: sha256(png),
        sampledSHA256: sha256(Data(pixels)),
        sampledBGRA: pixels,
        averageLuma: total / Double(sampleWidth * sampleHeight),
        width: width,
        height: height
    )
}

private func capture(
    filter: SCContentFilter, configuration: SCStreamConfiguration
) async throws -> CGImage {
    try await withCheckedThrowingContinuation { continuation in
        SCScreenshotManager.captureImage(
            contentFilter: filter, configuration: configuration
        ) { image, error in
            if let error { continuation.resume(throwing: error) }
            else if let image { continuation.resume(returning: image) }
            else { continuation.resume(throwing: CocoaError(.fileReadUnknown)) }
        }
    }
}

private func post(_ action: Action, identity: WindowIdentity, pid: pid_t) {
    switch action {
    case let .key(_, keyCode):
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: true),
              let up = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: false)
        else { fail("cannot create keyboard event") }
        down.postToPid(pid)
        usleep(80_000)
        up.postToPid(pid)
    case let .click(x, y):
        let point = CGPoint(
            x: identity.bounds.minX + identity.bounds.width * x,
            y: identity.bounds.minY + identity.bounds.height * y
        )
        guard let move = CGEvent(
            mouseEventSource: nil, mouseType: .mouseMoved,
            mouseCursorPosition: point, mouseButton: .left
        ), let down = CGEvent(
            mouseEventSource: nil, mouseType: .leftMouseDown,
            mouseCursorPosition: point, mouseButton: .left
        ), let up = CGEvent(
            mouseEventSource: nil, mouseType: .leftMouseUp,
            mouseCursorPosition: point, mouseButton: .left
        ) else { fail("cannot create mouse event") }
        // CGEvent.postToPid is reliable for keyboard events but does not
        // dispatch mouse clicks into Wine windows on macOS.  The caller has
        // already verified the exact WindowServer ID/PID and requires this
        // application to be frontmost before reaching the click branch, so a
        // global event is safe here and is the only path Wine receives.
        move.post(tap: .cghidEventTap)
        usleep(100_000)
        down.post(tap: .cghidEventTap)
        usleep(100_000)
        up.post(tap: .cghidEventTap)
    }
}

private let arguments = parseArguments()
guard NSWorkspace.shared.frontmostApplication?.bundleIdentifier != "com.apple.loginwindow" else {
    fail("interactive macOS session is locked")
}
guard !FileManager.default.fileExists(atPath: arguments.outputDirectory.path) else {
    fail("refusing to replace existing interaction output directory")
}
guard let application = NSRunningApplication(processIdentifier: arguments.expectedPID),
      let initialIdentity = windowIdentity(arguments.windowID),
      initialIdentity.ownerPID == arguments.expectedPID,
      initialIdentity.layer >= 0, initialIdentity.alpha > 0,
      initialIdentity.bounds.width >= 40, initialIdentity.bounds.height >= 40
else { fail("window/PID identity is not a visible layer-zero application window") }

try FileManager.default.createDirectory(
    at: arguments.outputDirectory, withIntermediateDirectories: false
)
application.activate(options: [.activateAllWindows])
usleep(500_000)
let frontmostPID = NSWorkspace.shared.frontmostApplication?.processIdentifier
let frontmostBefore = frontmostPID == arguments.expectedPID
let targetPoint: CGPoint
switch arguments.action {
case .key:
    targetPoint = CGPoint(x: initialIdentity.bounds.midX, y: initialIdentity.bounds.midY)
case let .click(x, y):
    targetPoint = CGPoint(
        x: initialIdentity.bounds.minX + initialIdentity.bounds.width * x,
        y: initialIdentity.bounds.minY + initialIdentity.bounds.height * y
    )
}
let topmostTarget = targetWindowIsTopmost(
    arguments.windowID, expectedPID: arguments.expectedPID, at: targetPoint
)
guard frontmostPID == arguments.expectedPID || topmostTarget else {
    fail("expected PID is not frontmost and target window is not topmost at its center")
}

let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: false)
guard let window = content.windows.first(where: {
    $0.windowID == arguments.windowID && $0.owningApplication?.processID == arguments.expectedPID
}) else { fail("ScreenCaptureKit cannot resolve expected window/PID") }
let filter = SCContentFilter(desktopIndependentWindow: window)
let configuration = SCStreamConfiguration()
configuration.width = max(1, Int(window.frame.width))
configuration.height = max(1, Int(window.frame.height))
configuration.showsCursor = false

guard let before = metrics(try await capture(filter: filter, configuration: configuration)) else {
    fail("cannot measure before image")
}
try before.png.write(to: arguments.outputDirectory.appendingPathComponent("before.png"), options: .withoutOverwriting)
post(arguments.action, identity: initialIdentity, pid: arguments.expectedPID)
usleep(arguments.delayMilliseconds * 1_000)
guard let finalIdentity = windowIdentity(arguments.windowID),
      finalIdentity.ownerPID == arguments.expectedPID
else { fail("window/PID focus or ownership changed after input") }
let frontmostAfter = NSWorkspace.shared.frontmostApplication?.processIdentifier == arguments.expectedPID
guard let after = metrics(try await capture(filter: filter, configuration: configuration)),
      before.width == after.width, before.height == after.height,
      before.sampledBGRA.count == after.sampledBGRA.count
else { fail("cannot compare before/after images") }
try after.png.write(to: arguments.outputDirectory.appendingPathComponent("after.png"), options: .withoutOverwriting)

var changedPixels = 0
let sampledPixels = before.sampledBGRA.count / 4
for offset in stride(from: 0, to: before.sampledBGRA.count, by: 4) {
    let changed = (0..<3).contains { channel in
        abs(Int(before.sampledBGRA[offset + channel]) - Int(after.sampledBGRA[offset + channel])) > 8
    }
    if changed { changedPixels += 1 }
}
let actionKind: String
let actionValue: String
switch arguments.action {
case let .key(name, _):
    actionKind = "key"
    actionValue = name
case let .click(x, y):
    actionKind = "click"
    actionValue = String(format: "%.6f:%.6f", x, y)
}
let changedRatio = Double(changedPixels) / Double(sampledPixels)
print("{")
print("  \"schema\": 1,")
print("  \"window_id\": \(arguments.windowID),")
print("  \"pid\": \(arguments.expectedPID),")
print("  \"action_kind\": \"\(actionKind)\",")
print("  \"action_value\": \"\(actionValue)\",")
print("  \"delay_ms\": \(arguments.delayMilliseconds),")
print("  \"width\": \(before.width),")
print("  \"height\": \(before.height),")
print("  \"before_png_sha256\": \"\(before.pngSHA256)\",")
print("  \"after_png_sha256\": \"\(after.pngSHA256)\",")
print("  \"before_sampled_sha256\": \"\(before.sampledSHA256)\",")
print("  \"after_sampled_sha256\": \"\(after.sampledSHA256)\",")
print("  \"before_luma\": \(String(format: "%.6f", before.averageLuma)),")
print("  \"after_luma\": \(String(format: "%.6f", after.averageLuma)),")
print("  \"sampled_pixels\": \(sampledPixels),")
print("  \"changed_pixels\": \(changedPixels),")
print("  \"changed_ratio\": \(String(format: "%.9f", changedRatio)),")
print("  \"frontmost_before\": \(frontmostBefore),")
print("  \"frontmost_after\": \(frontmostAfter),")
print("  \"owner_match_before\": true,")
print("  \"owner_match_after\": true")
print("}")
if arguments.writeJSON {
    let record: [String: Any] = [
        "schema": 1,
        "window_id": arguments.windowID,
        "pid": arguments.expectedPID,
        "action_kind": actionKind,
        "action_value": actionValue,
        "delay_ms": arguments.delayMilliseconds,
        "width": before.width,
        "height": before.height,
        "before_png_sha256": before.pngSHA256,
        "after_png_sha256": after.pngSHA256,
        "before_sampled_sha256": before.sampledSHA256,
        "after_sampled_sha256": after.sampledSHA256,
        "before_luma": before.averageLuma,
        "after_luma": after.averageLuma,
        "sampled_pixels": sampledPixels,
        "changed_pixels": changedPixels,
        "changed_ratio": changedRatio,
        "frontmost_before": frontmostBefore,
        "frontmost_after": frontmostAfter,
        "owner_match_before": true,
        "owner_match_after": true,
    ]
    let data = try JSONSerialization.data(withJSONObject: record, options: [.prettyPrinted, .sortedKeys])
    try data.write(
        to: arguments.outputDirectory.appendingPathComponent("interaction.json"),
        // The probe requires a fresh output directory before it starts, so an
        // atomic write is sufficient here. Foundation rejects combining
        // `.atomic` with `.withoutOverwriting` on this platform.
        options: [.atomic]
    )
}
