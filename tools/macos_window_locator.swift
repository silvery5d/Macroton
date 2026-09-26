import AppKit
import CoreGraphics
import Foundation

private struct Candidate {
    let id: CGWindowID
    let width: Int
    let height: Int

    var area: Int { width * height }
}

private func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("error: \(message)\n".utf8))
    exit(2)
}

let values = CommandLine.arguments
guard (values.count >= 2 && values.count <= 4),
      let expectedPID = Int32(values[1]), expectedPID > 0
else {
    fail("usage: macos_window_locator PID [WINDOW_ID] [--activate|--recover|--any]")
}
let explicitWindowID: CGWindowID? = values.count >= 3 && !values[2].hasPrefix("--")
    ? UInt32(values[2])
    : nil
let option = values.dropFirst(explicitWindowID == nil ? 2 : 3).first
guard values.dropFirst(explicitWindowID == nil ? 2 : 3).allSatisfy({
    ["--activate", "--recover", "--any"].contains($0)
})
else {
    fail("usage: macos_window_locator PID [WINDOW_ID] [--activate|--recover|--any]")
}
let shouldActivate = option == "--activate" || option == "--recover"
let shouldRecover = option == "--recover"
let includeOffscreen = option == "--any"

if shouldActivate {
    guard let application = NSRunningApplication(processIdentifier: expectedPID) else {
        fail("PID \(expectedPID) is not an NSRunningApplication")
    }
    let activated = application.activate(options: [.activateAllWindows])
    let accessibilityApplication = AXUIElementCreateApplication(expectedPID)
    var windowValue: CFTypeRef?
    let windowResult = AXUIElementCopyAttributeValue(
        accessibilityApplication,
        kAXWindowsAttribute as CFString,
        &windowValue
    )
    var raisedWindows = 0
    if windowResult == .success, let accessibilityWindows = windowValue as? [AXUIElement] {
        let targetBounds: CGRect? = if let id = explicitWindowID,
                                       let targetInfo = (CGWindowListCopyWindowInfo(
                                           [.optionIncludingWindow, .excludeDesktopElements], id
                                       ) as? [[String: Any]])?.first,
                                       let boundsDictionary = targetInfo[kCGWindowBounds as String] as? NSDictionary
        {
            CGRect(dictionaryRepresentation: boundsDictionary)
        } else {
            nil
        }
        for accessibilityWindow in accessibilityWindows {
            var positionValue: CFTypeRef?
            var sizeValue: CFTypeRef?
            AXUIElementCopyAttributeValue(accessibilityWindow, kAXPositionAttribute as CFString, &positionValue)
            AXUIElementCopyAttributeValue(accessibilityWindow, kAXSizeAttribute as CFString, &sizeValue)
            var position = CGPoint.zero
            var size = CGSize.zero
            let matchesTarget = targetBounds.map { bounds in
                if let positionValue, let sizeValue,
                   AXValueGetValue(positionValue as! AXValue, .cgPoint, &position),
                   AXValueGetValue(sizeValue as! AXValue, .cgSize, &size)
                {
                    return abs(position.x - bounds.minX) < 2 && abs(position.y - bounds.minY) < 2
                        && abs(size.width - bounds.width) < 2 && abs(size.height - bounds.height) < 2
                }
                return false
            } ?? true
            guard matchesTarget else { continue }
            AXUIElementSetAttributeValue(accessibilityWindow, kAXMinimizedAttribute as CFString, kCFBooleanFalse)
            AXUIElementSetAttributeValue(accessibilityWindow, kAXMainAttribute as CFString, kCFBooleanTrue)
            if AXUIElementPerformAction(accessibilityWindow, kAXRaiseAction as CFString) == .success {
                raisedWindows += 1
            }
            if shouldRecover {
                let visibleFrame = NSScreen.main?.visibleFrame ??
                    CGRect(x: 0, y: 0, width: 1440, height: 900)
                let width = min(max(960, visibleFrame.width * 0.75), visibleFrame.width)
                let height = min(max(640, visibleFrame.height * 0.75), visibleFrame.height)
                var position = CGPoint(
                    x: visibleFrame.midX - width / 2,
                    y: visibleFrame.midY - height / 2
                )
                var positionValue: CFTypeRef?
                var sizeValue: CFTypeRef?
                if let createdPosition = AXValueCreate(.cgPoint, &position) {
                    positionValue = createdPosition
                }
                var size = CGSize(width: width, height: height)
                if let createdSize = AXValueCreate(.cgSize, &size) {
                    sizeValue = createdSize
                }
                if let positionValue {
                    AXUIElementSetAttributeValue(
                        accessibilityWindow,
                        kAXPositionAttribute as CFString,
                        positionValue
                    )
                }
                if let sizeValue {
                    AXUIElementSetAttributeValue(
                        accessibilityWindow,
                        kAXSizeAttribute as CFString,
                        sizeValue
                    )
                }
            }
        }
    }
    FileHandle.standardError.write(
        Data(
            (
                "activation requested=\(activated) recover=\(shouldRecover) " +
                "accessibility=\(windowResult.rawValue) raised=\(raisedWindows)\n"
            ).utf8
        )
    )
    usleep(500_000)
}

guard let windows = CGWindowListCopyWindowInfo(
    [.optionAll, .excludeDesktopElements], kCGNullWindowID
) as? [[String: Any]] else {
    fail("WindowServer did not return an on-screen window list")
}

private let candidates = windows.compactMap { info -> Candidate? in
    guard (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == expectedPID,
          {
              let layer = (info[kCGWindowLayer as String] as? NSNumber)?.intValue ?? -1
              // Wine fullscreen game surfaces may be exposed at a positive
              // WindowServer layer (21/27 on current macOS) instead of layer 0.
              return layer >= 0
          }(),
          (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 0 > 0,
          (includeOffscreen || (info[kCGWindowIsOnscreen as String] as? NSNumber)?.boolValue == true),
          let number = info[kCGWindowNumber as String] as? NSNumber,
          let boundsDictionary = info[kCGWindowBounds as String] as? NSDictionary,
          let bounds = CGRect(dictionaryRepresentation: boundsDictionary)
    else { return nil }
    let width = Int(bounds.width.rounded(.down))
    let height = Int(bounds.height.rounded(.down))
    guard width >= 320, height >= 180 else { return nil }
    let candidate = Candidate(id: CGWindowID(number.uint32Value), width: width, height: height)
    if let explicitWindowID, candidate.id != explicitWindowID { return nil }
    return candidate
}.sorted {
    if $0.area != $1.area { return $0.area > $1.area }
    return $0.id < $1.id
}

guard candidates.count == 1, let candidate = candidates.first else {
    if candidates.isEmpty {
        let owned = windows.filter { info in
            (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == expectedPID
        }
        for info in owned {
            let id = (info[kCGWindowNumber as String] as? NSNumber)?.uint32Value ?? 0
            let layer = (info[kCGWindowLayer as String] as? NSNumber)?.intValue ?? -1
            let alpha = (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? -1
            let onScreen = (info[kCGWindowIsOnscreen as String] as? NSNumber)?.boolValue ?? false
            let name = info[kCGWindowName as String] as? String ?? ""
            let bounds = (info[kCGWindowBounds as String] as? NSDictionary)
                .flatMap(CGRect.init(dictionaryRepresentation:)) ?? .zero
            let detail =
                "owned window_id=\(id) layer=\(layer) onscreen=\(onScreen) alpha=\(alpha) " +
                "bounds=\(Int(bounds.minX)),\(Int(bounds.minY)),\(Int(bounds.width))x\(Int(bounds.height)) " +
                "name=\(name)\n"
            FileHandle.standardError.write(Data(detail.utf8))
        }
        if owned.isEmpty {
            for info in windows {
                let owner = info[kCGWindowOwnerName as String] as? String ?? ""
                let name = info[kCGWindowName as String] as? String ?? ""
                guard owner.localizedCaseInsensitiveContains("wine")
                        || name.localizedCaseInsensitiveContains("portal")
                else { continue }
                let id = (info[kCGWindowNumber as String] as? NSNumber)?.uint32Value ?? 0
                let pid = (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value ?? 0
                let layer = (info[kCGWindowLayer as String] as? NSNumber)?.intValue ?? -1
                let detail = "related window_id=\(id) pid=\(pid) layer=\(layer) owner=\(owner) name=\(name)\n"
                FileHandle.standardError.write(Data(detail.utf8))
            }
        }
        fail("PID \(expectedPID) has no eligible on-screen nonnegative-layer window of at least 320x180")
    }
    FileHandle.standardError.write(
        Data("error: PID \(expectedPID) has \(candidates.count) eligible windows; pass an explicit WINDOW_ID\n".utf8)
    )
    for candidate in candidates {
        FileHandle.standardError.write(
            Data("candidate window_id=\(candidate.id) size=\(candidate.width)x\(candidate.height)\n".utf8)
        )
    }
    exit(2)
}

print(candidate.id)
