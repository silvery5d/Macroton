import AppKit
import AVFoundation
import CoreGraphics
import CoreMedia
import Foundation
import ScreenCaptureKit

private struct Arguments {
    let windowID: CGWindowID
    let expectedPID: pid_t
    let durationSeconds: Double
    let silenceThresholdDBFS: Double
}

private func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("error: \(message)\n".utf8))
    exit(2)
}

private func parseArguments() -> Arguments {
    let values = CommandLine.arguments
    guard (4...5).contains(values.count),
          let windowID = UInt32(values[1]),
          let expectedPID = Int32(values[2]),
          let durationSeconds = Double(values[3]), (0.5...600).contains(durationSeconds)
    else {
        fail("usage: macos_audio_probe WINDOW_ID PID DURATION_SECONDS [SILENCE_DBFS]")
    }
    let silenceThresholdDBFS: Double
    if values.count == 5 {
        guard let value = Double(values[4]), (-120 ... -1).contains(value) else {
            fail("SILENCE_DBFS must be between -120 and -1")
        }
        silenceThresholdDBFS = value
    } else {
        silenceThresholdDBFS = -50
    }
    return Arguments(
        windowID: CGWindowID(windowID),
        expectedPID: pid_t(expectedPID),
        durationSeconds: durationSeconds,
        silenceThresholdDBFS: silenceThresholdDBFS
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

private func decibels(_ amplitude: Double) -> Double {
    20 * log10(max(amplitude, 1e-12))
}

private func startCapture(_ stream: SCStream) async throws {
    let operation: () async throws -> Void = stream.startCapture
    try await operation()
}

private func stopCapture(_ stream: SCStream) async throws {
    let operation: () async throws -> Void = stream.stopCapture
    try await operation()
}

private struct AudioSummary {
    let buffers: Int
    let frames: Int64
    let samples: Int64
    let sampleRate: Double
    let channels: UInt32
    let rms: Double
    let peak: Double
    let nonSilentBuffers: Int
    let invalidBuffers: Int
    let formatChanges: Int
}

private final class AudioSampler: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    private let silenceAmplitude: Double
    private let lock = NSLock()
    private var buffers = 0
    private var frames: Int64 = 0
    private var samples: Int64 = 0
    private var sumSquares = 0.0
    private var peak = 0.0
    private var nonSilentBuffers = 0
    private var invalidBuffers = 0
    private var sampleRate = 0.0
    private var channels: UInt32 = 0
    private var formatChanges = 0

    init(silenceThresholdDBFS: Double) {
        silenceAmplitude = pow(10, silenceThresholdDBFS / 20)
    }

    func stream(
        _: SCStream,
        didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of outputType: SCStreamOutputType
    ) {
        guard outputType == .audio else { return }
        guard sampleBuffer.isValid,
              let formatDescription = sampleBuffer.formatDescription,
              let streamDescription = CMAudioFormatDescriptionGetStreamBasicDescription(
                  formatDescription
              ),
              streamDescription.pointee.mFormatID == kAudioFormatLinearPCM,
              streamDescription.pointee.mBitsPerChannel == 32,
              streamDescription.pointee.mFormatFlags & kAudioFormatFlagIsFloat != 0,
              let format = AVAudioFormat(streamDescription: streamDescription)
        else {
            lock.lock()
            invalidBuffers += 1
            lock.unlock()
            return
        }

        let frameCount = sampleBuffer.numSamples
        guard frameCount > 0,
              let audio = AVAudioPCMBuffer(
                  pcmFormat: format,
                  frameCapacity: AVAudioFrameCount(frameCount)
              )
        else { return }
        audio.frameLength = AVAudioFrameCount(frameCount)
        let copyStatus = CMSampleBufferCopyPCMDataIntoAudioBufferList(
            sampleBuffer,
            at: 0,
            frameCount: Int32(frameCount),
            into: audio.mutableAudioBufferList
        )
        guard copyStatus == noErr, let channelData = audio.floatChannelData else {
            lock.lock()
            invalidBuffers += 1
            lock.unlock()
            return
        }

        let channelCount = Int(format.channelCount)
        var bufferSquares = 0.0
        var bufferPeak = 0.0
        for channel in 0 ..< channelCount {
            let values = channelData[channel]
            for frame in 0 ..< frameCount {
                let value = Double(values[frame])
                bufferSquares += value * value
                bufferPeak = max(bufferPeak, abs(value))
            }
        }
        let bufferSamples = frameCount * channelCount
        let bufferRMS = sqrt(bufferSquares / Double(max(1, bufferSamples)))

        lock.lock()
        if sampleRate != 0,
           (sampleRate != format.sampleRate || channels != format.channelCount)
        {
            formatChanges += 1
        }
        sampleRate = format.sampleRate
        channels = format.channelCount
        buffers += 1
        frames += Int64(frameCount)
        samples += Int64(bufferSamples)
        sumSquares += bufferSquares
        peak = max(peak, bufferPeak)
        if bufferRMS >= silenceAmplitude { nonSilentBuffers += 1 }
        lock.unlock()
    }

    func stream(_: SCStream, didStopWithError error: any Error) {
        FileHandle.standardError.write(Data("stream error: \(error)\n".utf8))
        lock.lock()
        invalidBuffers += 1
        lock.unlock()
    }

    func summary() -> AudioSummary {
        lock.lock()
        defer { lock.unlock() }
        return AudioSummary(
            buffers: buffers,
            frames: frames,
            samples: samples,
            sampleRate: sampleRate,
            channels: channels,
            rms: sqrt(sumSquares / Double(max(Int64(1), samples))),
            peak: peak,
            nonSilentBuffers: nonSilentBuffers,
            invalidBuffers: invalidBuffers,
            formatChanges: formatChanges
        )
    }
}

private let arguments = parseArguments()
guard windowOwnerPID(arguments.windowID) == arguments.expectedPID else {
    fail("window owner does not match expected PID")
}
guard NSRunningApplication(processIdentifier: arguments.expectedPID) != nil else {
    fail("expected PID is not a running application")
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
configuration.width = 2
configuration.height = 2
configuration.minimumFrameInterval = CMTime(value: 1, timescale: 1)
configuration.queueDepth = 3
configuration.showsCursor = false
configuration.capturesAudio = true
configuration.excludesCurrentProcessAudio = true
configuration.sampleRate = 48_000
configuration.channelCount = 2

private let sampler = AudioSampler(silenceThresholdDBFS: arguments.silenceThresholdDBFS)
private let stream = SCStream(filter: filter, configuration: configuration, delegate: sampler)
private let queue = DispatchQueue(label: "macroton.audio-probe", qos: .userInitiated)
try stream.addStreamOutput(sampler, type: .audio, sampleHandlerQueue: queue)

try await startCapture(stream)
try await Task.sleep(for: .seconds(arguments.durationSeconds))
try await stopCapture(stream)

private let summary = sampler.summary()
private let ownerMatches = windowOwnerPID(arguments.windowID) == arguments.expectedPID
private let ratio = summary.buffers == 0
    ? 0
    : Double(summary.nonSilentBuffers) / Double(summary.buffers)
print("{")
print("  \"schema\": 1,")
print("  \"window_id\": \(arguments.windowID),")
print("  \"pid\": \(arguments.expectedPID),")
print("  \"duration_seconds\": \(arguments.durationSeconds),")
print("  \"sample_rate\": \(summary.sampleRate),")
print("  \"channels\": \(summary.channels),")
print("  \"buffers\": \(summary.buffers),")
print("  \"frames\": \(summary.frames),")
print("  \"rms_dbfs\": \(decibels(summary.rms)),")
print("  \"peak_dbfs\": \(decibels(summary.peak)),")
print("  \"silence_threshold_dbfs\": \(arguments.silenceThresholdDBFS),")
print("  \"non_silent_buffers\": \(summary.nonSilentBuffers),")
print("  \"non_silent_ratio\": \(ratio),")
print("  \"invalid_buffers\": \(summary.invalidBuffers),")
print("  \"format_changes\": \(summary.formatChanges),")
print("  \"owner_match\": \(ownerMatches)")
print("}")

if summary.buffers == 0 || summary.nonSilentBuffers == 0 || summary.invalidBuffers != 0
    || summary.formatChanges != 0 || !ownerMatches
{
    exit(1)
}
