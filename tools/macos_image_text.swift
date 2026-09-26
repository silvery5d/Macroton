import AppKit
import Foundation
import Vision

guard CommandLine.arguments.count == 2 else {
    fputs("usage: macos-image-text IMAGE\n", stderr)
    exit(64)
}

let path = CommandLine.arguments[1]
guard
    let image = NSImage(contentsOfFile: path),
    let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil)
else {
    fputs("cannot load image: \(path)\n", stderr)
    exit(66)
}

let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = false

do {
    try VNImageRequestHandler(cgImage: cgImage).perform([request])
} catch {
    fputs("text recognition failed: \(error)\n", stderr)
    exit(70)
}

for observation in request.results ?? [] {
    if let candidate = observation.topCandidates(1).first {
        print(candidate.string)
    }
}
