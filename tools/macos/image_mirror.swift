import AppKit
import Foundation

final class MirrorController: NSObject, NSApplicationDelegate {
    private let imagePath: String
    private let imageView = NSImageView()
    private var modificationDate: Date?
    private var timer: Timer?

    init(imagePath: String) {
        self.imagePath = imagePath
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 700, height: 440),
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Macroton — Steam Sign In"
        window.isReleasedWhenClosed = false
        window.center()

        imageView.imageScaling = .scaleProportionallyUpOrDown
        imageView.frame = window.contentView?.bounds ?? .zero
        imageView.autoresizingMask = [.width, .height]
        window.contentView = imageView
        reloadImageIfNeeded()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            self?.reloadImageIfNeeded()
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    private func reloadImageIfNeeded() {
        let attributes = try? FileManager.default.attributesOfItem(atPath: imagePath)
        let currentDate = attributes?[.modificationDate] as? Date
        guard currentDate != modificationDate else { return }
        guard let image = NSImage(contentsOfFile: imagePath) else { return }
        modificationDate = currentDate
        imageView.image = image
    }
}

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: image_mirror IMAGE.png\n".utf8))
    exit(2)
}

let application = NSApplication.shared
application.setActivationPolicy(.regular)
let controller = MirrorController(imagePath: CommandLine.arguments[1])
application.delegate = controller
application.run()
