import AppKit
import AVFoundation
import Foundation

private func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("error: \(message)\n".utf8))
    exit(2)
}

guard CommandLine.arguments.count == 2,
      let duration = Double(CommandLine.arguments[1]),
      (2 ... 60).contains(duration)
else {
    fail("usage: macos_audio_fixture DURATION_SECONDS")
}

let application = NSApplication.shared
application.setActivationPolicy(.regular)

let window = NSWindow(
    contentRect: NSRect(x: 200, y: 200, width: 480, height: 180),
    styleMask: [.titled, .closable],
    backing: .buffered,
    defer: false
)
window.title = "Macroton Audio Fixture"
let label = NSTextField(labelWithString: "Macroton audio-probe fixture: 440 Hz")
label.alignment = .center
label.frame = NSRect(x: 20, y: 65, width: 440, height: 30)
window.contentView?.addSubview(label)
window.makeKeyAndOrderFront(nil)
application.activate(ignoringOtherApps: true)

let engine = AVAudioEngine()
let player = AVAudioPlayerNode()
engine.attach(player)
let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2)!
engine.connect(player, to: engine.mainMixerNode, format: format)
engine.mainMixerNode.outputVolume = 0.025

let frameCount = AVAudioFrameCount(48_000)
guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frameCount),
      let channels = buffer.floatChannelData
else {
    fail("could not allocate fixture audio buffer")
}
buffer.frameLength = frameCount
for frame in 0 ..< Int(frameCount) {
    let sample = Float(sin(2 * Double.pi * 440 * Double(frame) / 48_000)) * 0.2
    channels[0][frame] = sample
    channels[1][frame] = sample
}

do {
    try engine.start()
} catch {
    fail("could not start fixture audio engine: \(error)")
}
player.scheduleBuffer(buffer, at: nil, options: .loops)
player.play()

let identity = "{\"pid\":\(ProcessInfo.processInfo.processIdentifier),\"window_id\":\(window.windowNumber)}\n"
FileHandle.standardOutput.write(Data(identity.utf8))

Timer.scheduledTimer(withTimeInterval: duration, repeats: false) { _ in
    player.stop()
    engine.stop()
    application.terminate(nil)
}
application.run()
