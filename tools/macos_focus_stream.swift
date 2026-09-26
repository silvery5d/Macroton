import AppKit
import CoreGraphics
import CoreMedia
import CoreVideo
import CryptoKit
import Foundation
import ScreenCaptureKit

private struct Arguments {
    let windowID: CGWindowID
    let expectedPID: pid_t
    let frames: Int
    let framesPerSecond: Int32
}

private func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("error: \(message)\n".utf8))
    exit(2)
}

private func parseArguments() -> Arguments {
    let values = CommandLine.arguments
    guard values.count == 5,
          let windowID = UInt32(values[1]),
          let expectedPID = Int32(values[2]),
          let frames = Int(values[3]), frames > 0,
          let framesPerSecond = Int32(values[4]), (1...120).contains(framesPerSecond)
    else {
        fail("usage: macos_focus_stream WINDOW_ID PID FRAMES FPS")
    }
    return Arguments(
        windowID: CGWindowID(windowID),
        expectedPID: pid_t(expectedPID),
        frames: frames,
        framesPerSecond: framesPerSecond
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

private func focusApplicationWithSystemEvents(_ pid: pid_t) -> Bool {
    let source = "tell application \"System Events\" to set frontmost of first application process whose unix id is \(pid) to true"
    guard let script = NSAppleScript(source: source) else { return false }
    var errorInfo: NSDictionary?
    script.executeAndReturnError(&errorInfo)
    return errorInfo == nil
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

private func sampledMetrics(_ pixelBuffer: CVPixelBuffer) -> (Double, String)? {
    guard CVPixelBufferGetPixelFormatType(pixelBuffer) == kCVPixelFormatType_32BGRA else {
        return nil
    }
    CVPixelBufferLockBaseAddress(pixelBuffer, .readOnly)
    defer { CVPixelBufferUnlockBaseAddress(pixelBuffer, .readOnly) }
    guard let baseAddress = CVPixelBufferGetBaseAddress(pixelBuffer) else { return nil }

    let width = CVPixelBufferGetWidth(pixelBuffer)
    let height = CVPixelBufferGetHeight(pixelBuffer)
    let bytesPerRow = CVPixelBufferGetBytesPerRow(pixelBuffer)
    let bytes = baseAddress.assumingMemoryBound(to: UInt8.self)
    let step = max(1, max(width, height) / 256)
    // ScreenCaptureKit includes the native macOS title bar in a window
    // capture.  A white title bar can otherwise make an entirely black Wine
    // game surface pass the luma gate.  Ignore a bounded 96-pixel top chrome
    // band for both luma and the sampled hash; fullscreen windows simply lose
    // a thin strip of game content, which is preferable to a false pass.
    let contentTop = min(max(0, height - 1), 96)
    var total = 0.0
    var count = 0
    var digestInput = Data()
    digestInput.reserveCapacity(
        ((width / step) + 1) * (((height - contentTop) / step) + 1) * 4
    )

    for y in stride(from: contentTop, to: height, by: step) {
        for x in stride(from: 0, to: width, by: step) {
            let offset = y * bytesPerRow + x * 4
            let blue = bytes[offset]
            let green = bytes[offset + 1]
            let red = bytes[offset + 2]
            let alpha = bytes[offset + 3]
            total += 0.2126 * Double(red) + 0.7152 * Double(green) + 0.0722 * Double(blue)
            digestInput.append(contentsOf: [blue, green, red, alpha])
            count += 1
        }
    }
    guard count > 0 else { return nil }
    let digest = SHA256.hash(data: digestInput).map { String(format: "%02x", $0) }.joined()
    return (total / Double(count), digest)
}

private func startCapture(_ stream: SCStream) async throws {
    let operation: () async throws -> Void = stream.startCapture
    try await operation()
}

private func stopCapture(_ stream: SCStream) async throws {
    let operation: () async throws -> Void = stream.stopCapture
    try await operation()
}

private final class StreamSampler: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    private let expectedPID: pid_t
    private let targetFrames: Int
    private let started = DispatchTime.now().uptimeNanoseconds
    private let lock = NSLock()
    private var receivedFrames = 0
    private var focusedFrames = 0
    private var invalidFrames = 0
    private var completion: CheckedContinuation<(Int, Int, Int), Never>?

    init(expectedPID: pid_t, targetFrames: Int) {
        self.expectedPID = expectedPID
        self.targetFrames = targetFrames
    }

    func waitForCompletion() async -> (Int, Int, Int) {
        await withCheckedContinuation { continuation in
            lock.lock()
            if receivedFrames >= targetFrames {
                let result = (receivedFrames, focusedFrames, invalidFrames)
                lock.unlock()
                continuation.resume(returning: result)
            } else {
                completion = continuation
                lock.unlock()
            }
        }
    }

    func stream(
        _: SCStream,
        didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of outputType: SCStreamOutputType
    ) {
        guard outputType == .screen else { return }

        lock.lock()
        guard receivedFrames < targetFrames else {
            lock.unlock()
            return
        }
        let frame = receivedFrames
        receivedFrames += 1
        lock.unlock()

        let now = DispatchTime.now().uptimeNanoseconds
        let elapsedMilliseconds = Double(now - started) / 1_000_000.0
        let frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier == expectedPID
        let metrics = CMSampleBufferGetImageBuffer(sampleBuffer).flatMap(sampledMetrics)

        lock.lock()
        if frontmost { focusedFrames += 1 }
        if metrics == nil { invalidFrames += 1 }
        let shouldComplete = receivedFrames >= targetFrames
        let result = (receivedFrames, focusedFrames, invalidFrames)
        let continuation = shouldComplete ? completion : nil
        if shouldComplete { completion = nil }
        lock.unlock()

        if let metrics {
            let elapsed = String(format: "%.3f", elapsedMilliseconds)
            let luma = String(format: "%.6f", metrics.0)
            print(
                "\(frame),\(elapsed),\(frontmost)," +
                    "\(luma),\(metrics.1),\(arguments.windowID),\(arguments.expectedPID)"
            )
        } else {
            let elapsed = String(format: "%.3f", elapsedMilliseconds)
            print("\(frame),\(elapsed),\(frontmost),,,\(arguments.windowID),\(arguments.expectedPID)")
        }
        continuation?.resume(returning: result)
    }

    func stream(_: SCStream, didStopWithError error: any Error) {
        FileHandle.standardError.write(Data("stream error: \(error)\n".utf8))
        lock.lock()
        invalidFrames += 1
        let continuation = completion
        completion = nil
        let result = (receivedFrames, focusedFrames, invalidFrames)
        lock.unlock()
        continuation?.resume(returning: result)
    }
}

private let arguments = parseArguments()
guard windowOwnerPID(arguments.windowID) == arguments.expectedPID else {
    fail("window owner does not match expected PID")
}
guard let application = NSRunningApplication(processIdentifier: arguments.expectedPID) else {
    fail("expected PID is not a running application")
}
if NSWorkspace.shared.frontmostApplication?.bundleIdentifier == "com.apple.loginwindow" {
    fail("interactive macOS session is locked; focused capture is impossible")
}
application.activate(options: [.activateAllWindows])
usleep(300_000)
if NSWorkspace.shared.frontmostApplication?.processIdentifier != arguments.expectedPID {
    _ = focusApplicationWithSystemEvents(arguments.expectedPID)
    usleep(300_000)
    if NSWorkspace.shared.frontmostApplication?.processIdentifier != arguments.expectedPID {
        guard focusWindowByTitleBarClick(
            arguments.windowID,
            expectedPID: arguments.expectedPID
        ) else {
            fail("cannot focus expected window")
        }
        usleep(500_000)
    }
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == arguments.expectedPID else {
        fail("cannot focus expected PID")
    }
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

let filter = SCContentFilter(desktopIndependentWindow: window)
let configuration = SCStreamConfiguration()
configuration.width = max(1, Int(window.frame.width))
configuration.height = max(1, Int(window.frame.height))
configuration.minimumFrameInterval = CMTime(value: 1, timescale: arguments.framesPerSecond)
configuration.queueDepth = 8
configuration.pixelFormat = kCVPixelFormatType_32BGRA
configuration.showsCursor = false
configuration.capturesAudio = false

private let sampler = StreamSampler(expectedPID: arguments.expectedPID, targetFrames: arguments.frames)
private let stream = SCStream(filter: filter, configuration: configuration, delegate: sampler)
private let queue = DispatchQueue(label: "macroton.focus-stream", qos: .userInteractive)
try stream.addStreamOutput(sampler, type: .screen, sampleHandlerQueue: queue)

print("frame,elapsed_ms,frontmost,average_luma,sampled_sha256,window_id,pid")
try await startCapture(stream)
let result = await sampler.waitForCompletion()
try await stopCapture(stream)

let ownerMatches = windowOwnerPID(arguments.windowID) == arguments.expectedPID
let summary = "summary: captured=\(result.0)/\(arguments.frames) " +
    "focused=\(result.1)/\(result.0) invalid=\(result.2) owner_match=\(ownerMatches)\n"
FileHandle.standardError.write(
    Data(summary.utf8)
)
if result.0 != arguments.frames || result.1 != result.0 || result.2 != 0 || !ownerMatches {
    exit(1)
}
