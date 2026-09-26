import AppKit
import CoreGraphics
import Foundation

let usage = "usage: macos_key_probe PID <return|escape|up|down|left|right|space>"
guard CommandLine.arguments.count == 3,
      let pid = Int32(CommandLine.arguments[1])
else {
    FileHandle.standardError.write(Data("\(usage)\n".utf8))
    exit(2)
}

let keyCodes: [String: CGKeyCode] = [
    "return": 36,
    "escape": 53,
    "up": 126,
    "down": 125,
    "left": 123,
    "right": 124,
    "space": 49,
]
guard let keyCode = keyCodes[CommandLine.arguments[2]] else {
    FileHandle.standardError.write(Data("\(usage)\n".utf8))
    exit(2)
}
guard let application = NSRunningApplication(processIdentifier: pid) else {
    FileHandle.standardError.write(Data("process \(pid) is not a running application\n".utf8))
    exit(1)
}

application.activate(options: [.activateAllWindows])
Thread.sleep(forTimeInterval: 0.2)
guard let down = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: true),
      let up = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: false)
else {
    FileHandle.standardError.write(Data("could not create keyboard event\n".utf8))
    exit(1)
}
down.postToPid(pid)
Thread.sleep(forTimeInterval: 0.08)
up.postToPid(pid)
