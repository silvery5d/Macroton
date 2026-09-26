import AppKit
import CoreGraphics
import CryptoKit
import Foundation
import ImageIO
import ScreenCaptureKit
import UniformTypeIdentifiers

private struct Arguments {
    let windowID: CGWindowID
    let expectedPID: pid_t
    let frames: Int
    let intervalMilliseconds: UInt32
    let outputDirectory: URL
}

private func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("error: \(message)\n".utf8))
    exit(2)
}

private func parseArguments() -> Arguments {
    let values = CommandLine.arguments
    guard values.count == 6,
          let windowID = UInt32(values[1]),
          let expectedPID = Int32(values[2]),
          let frames = Int(values[3]), frames > 0,
          let intervalMilliseconds = UInt32(values[4]), intervalMilliseconds > 0
    else {
        fail("usage: macos_focus_capture WINDOW_ID PID FRAMES INTERVAL_MS OUTPUT_DIR")
    }
    return Arguments(
        windowID: CGWindowID(windowID),
        expectedPID: pid_t(expectedPID),
        frames: frames,
        intervalMilliseconds: intervalMilliseconds,
        outputDirectory: URL(fileURLWithPath: values[5], isDirectory: true)
    )
}

private func windowOwnerPID(_ windowID: CGWindowID) -> pid_t? {
    guard let windows = CGWindowListCopyWindowInfo(
        [.optionIncludingWindow, .excludeDesktopElements],
        windowID
    ) as? [[String: Any]] else {
        return nil
    }
    return windows.first.flatMap { info in
        (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value
    }
}

private func focusWindowByTitleBarClick(
    _ windowID: CGWindowID,
    expectedPID: pid_t
) -> Bool {
    guard let windows = CGWindowListCopyWindowInfo(
        [.optionIncludingWindow, .excludeDesktopElements],
        windowID
    ) as? [[String: Any]],
          let info = windows.first,
          (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == expectedPID,
          {
              let layer = (info[kCGWindowLayer as String] as? NSNumber)?.intValue ?? -1
              return layer >= 0
          }(),
          (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 0 > 0,
          let boundsDictionary = info[kCGWindowBounds as String] as? NSDictionary,
          let bounds = CGRect(dictionaryRepresentation: boundsDictionary),
          bounds.width >= 40,
          bounds.height >= 40
    else {
        return false
    }

    let point = CGPoint(x: bounds.midX, y: bounds.minY + 10)
    guard let down = CGEvent(
        mouseEventSource: nil,
        mouseType: .leftMouseDown,
        mouseCursorPosition: point,
        mouseButton: .left
    ),
          let up = CGEvent(
              mouseEventSource: nil,
              mouseType: .leftMouseUp,
              mouseCursorPosition: point,
              mouseButton: .left
          )
    else {
        return false
    }
    down.post(tap: .cghidEventTap)
    usleep(80_000)
    up.post(tap: .cghidEventTap)
    return true
}

private func focusApplicationWithSystemEvents(_ pid: pid_t) -> Bool {
    let source = "tell application \"System Events\" to set frontmost of first application process whose unix id is \(pid) to true"
    guard let script = NSAppleScript(source: source) else { return false }
    var errorInfo: NSDictionary?
    script.executeAndReturnError(&errorInfo)
    return errorInfo == nil
}

private func pngData(for image: CGImage) -> Data? {
    let data = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(
        data,
        UTType.png.identifier as CFString,
        1,
        nil
    ) else {
        return nil
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else {
        return nil
    }
    return data as Data
}

private func averageLuma(_ image: CGImage) -> Double? {
    let width = image.width
    let height = image.height
    guard width > 0, height > 0 else { return nil }

    let sampleWidth = min(width, 320)
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
    ) else {
        return nil
    }
    context.interpolationQuality = .low
    context.draw(image, in: CGRect(x: 0, y: 0, width: sampleWidth, height: sampleHeight))

    var total = 0.0
    for offset in stride(from: 0, to: pixels.count, by: 4) {
        total += 0.2126 * Double(pixels[offset])
            + 0.7152 * Double(pixels[offset + 1])
            + 0.0722 * Double(pixels[offset + 2])
    }
    return total / Double(sampleWidth * sampleHeight)
}

private func captureWindowImage(
    filter: SCContentFilter,
    configuration: SCStreamConfiguration
) async throws -> CGImage {
    try await withCheckedThrowingContinuation { continuation in
        SCScreenshotManager.captureImage(
            contentFilter: filter,
            configuration: configuration
        ) { image, error in
            if let error {
                continuation.resume(throwing: error)
            } else if let image {
                continuation.resume(returning: image)
            } else {
                continuation.resume(throwing: CocoaError(.fileReadUnknown))
            }
        }
    }
}

private let arguments = parseArguments()
try FileManager.default.createDirectory(
    at: arguments.outputDirectory,
    withIntermediateDirectories: true
)

guard windowOwnerPID(arguments.windowID) == arguments.expectedPID else {
    fail("window owner does not match expected PID")
}
guard let application = NSRunningApplication(processIdentifier: arguments.expectedPID) else {
    fail("expected PID is not a running application")
}
application.activate(options: [.activateAllWindows])
usleep(300_000)
if NSWorkspace.shared.frontmostApplication?.processIdentifier != arguments.expectedPID {
    guard focusApplicationWithSystemEvents(arguments.expectedPID)
        || focusWindowByTitleBarClick(arguments.windowID, expectedPID: arguments.expectedPID)
    else {
        fail("cannot focus expected window")
    }
    usleep(500_000)
}

let shareableContent = try await SCShareableContent.excludingDesktopWindows(
    true,
    onScreenWindowsOnly: false
)
guard let window = shareableContent.windows.first(where: {
    $0.windowID == arguments.windowID
        && $0.owningApplication?.processID == arguments.expectedPID
}) else {
    fail("ScreenCaptureKit cannot resolve the expected window")
}
let contentFilter = SCContentFilter(desktopIndependentWindow: window)
let streamConfiguration = SCStreamConfiguration()
streamConfiguration.width = max(1, Int(window.frame.width))
streamConfiguration.height = max(1, Int(window.frame.height))
streamConfiguration.showsCursor = false

// Keep the stream identical to Macroton's verifier schema so no manual CSV
// post-processing is needed. PNG dimensions and paths are deterministic from
// the output directory and frame number.
print("frame,elapsed_ms,frontmost,average_luma,sampled_sha256,window_id,pid")
let started = DispatchTime.now().uptimeNanoseconds
var focusedFrames = 0
var capturedFrames = 0

for frame in 0..<arguments.frames {
    let now = DispatchTime.now().uptimeNanoseconds
    let elapsedMilliseconds = Double(now - started) / 1_000_000.0
    let frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier
        == arguments.expectedPID
    let ownerMatches = windowOwnerPID(arguments.windowID) == arguments.expectedPID
    if frontmost { focusedFrames += 1 }

    do {
        guard ownerMatches else { throw CocoaError(.fileNoSuchFile) }
        let image = try await captureWindowImage(
            filter: contentFilter,
            configuration: streamConfiguration
        )
        guard let data = pngData(for: image), let luma = averageLuma(image) else {
            throw CocoaError(.fileWriteUnknown)
        }
        let name = String(format: "frame-%04d.png", frame)
        let destination = arguments.outputDirectory.appendingPathComponent(name)
        try data.write(to: destination, options: .atomic)
        capturedFrames += 1
        let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        print(
            "\(frame),\(String(format: "%.3f", elapsedMilliseconds)),\(frontmost)," +
                "\(String(format: "%.6f", luma)),\(digest),\(arguments.windowID),\(arguments.expectedPID)"
        )
    } catch {
        print("\(frame),\(String(format: "%.3f", elapsedMilliseconds)),\(frontmost),0,,\(arguments.windowID),\(arguments.expectedPID)" )
    }
    if frame + 1 < arguments.frames {
        usleep(arguments.intervalMilliseconds * 1_000)
    }
}

FileHandle.standardError.write(
    Data("summary: captured=\(capturedFrames)/\(arguments.frames) focused=\(focusedFrames)/\(arguments.frames)\n".utf8)
)
if capturedFrames != arguments.frames || focusedFrames != arguments.frames {
    exit(1)
}
