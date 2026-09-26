import AudioToolbox
import CoreAudio
import Foundation

func deviceName(_ device: AudioDeviceID) -> String {
    var address = AudioObjectPropertyAddress(
        mSelector: kAudioDevicePropertyDeviceNameCFString,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var name: CFString = "" as CFString
    var size = UInt32(MemoryLayout<CFString>.size)
    let status = withUnsafeMutablePointer(to: &name) { pointer in
        AudioObjectGetPropertyData(device, &address, 0, nil, &size, pointer)
    }
    return status == noErr ? String(name) : "unavailable(status=\(status))"
}

for (label, selector) in [
    ("default_output", kAudioHardwarePropertyDefaultOutputDevice),
    ("system_output", kAudioHardwarePropertyDefaultSystemOutputDevice),
] {
    var address = AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var device = AudioDeviceID(0)
    var size = UInt32(MemoryLayout<AudioDeviceID>.size)
    let status = AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device
    )
    guard status == noErr && device != 0 else {
        fputs("cannot read macOS \(label): \(status)\n", stderr)
        exit(1)
    }
    print("\(label)_id=\(device)")
    print("\(label)_name=\(deviceName(device))")
}
