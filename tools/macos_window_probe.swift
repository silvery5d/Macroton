import CoreGraphics
import Foundation

struct WindowRecord {
    let id: CGWindowID
    let ownerPID: pid_t
    let ownerName: String
    let title: String
    let bounds: CGRect
    let layer: Int
    let alpha: Double
}

func records(onScreenOnly: Bool) -> [WindowRecord] {
    var options: CGWindowListOption = [.excludeDesktopElements]
    if onScreenOnly {
        options.insert(.optionOnScreenOnly)
    } else {
        options.insert(.optionAll)
    }
    guard let raw = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
        return []
    }
    return raw.compactMap { item in
        guard
            let number = item[kCGWindowNumber as String] as? UInt32,
            let pid = item[kCGWindowOwnerPID as String] as? Int32,
            let owner = item[kCGWindowOwnerName as String] as? String,
            let boundsValue = item[kCGWindowBounds as String],
            let bounds = CGRect(dictionaryRepresentation: boundsValue as! CFDictionary)
        else {
            return nil
        }
        return WindowRecord(
            id: number,
            ownerPID: pid,
            ownerName: owner,
            title: item[kCGWindowName as String] as? String ?? "",
            bounds: bounds,
            layer: item[kCGWindowLayer as String] as? Int ?? -1,
            alpha: item[kCGWindowAlpha as String] as? Double ?? 0
        )
    }
}

var arguments = Array(CommandLine.arguments.dropFirst())
let onScreenOnly = arguments.first != "--all"
if !onScreenOnly {
    arguments.removeFirst()
}
let exactTitle = arguments.first
let matches = records(onScreenOnly: onScreenOnly).filter { record in
    guard let exactTitle else { return true }
    return record.title == exactTitle
}

for record in matches {
    print(
        "id=\(record.id)\tpid=\(record.ownerPID)\towner=\(record.ownerName)\t" +
        "title=\(record.title)\tlayer=\(record.layer)\talpha=\(record.alpha)\t" +
        "bounds=\(Int(record.bounds.origin.x)),\(Int(record.bounds.origin.y))," +
        "\(Int(record.bounds.size.width)),\(Int(record.bounds.size.height))"
    )
}

if exactTitle != nil && matches.isEmpty {
    exit(2)
}
