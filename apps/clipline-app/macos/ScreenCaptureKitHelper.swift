import CoreGraphics
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit

let version = "clipline-sck-helper 1"
let streamMagic = [UInt8]("CLNV".utf8)
let frameMagic = [UInt8]("FRAM".utf8)
let protocolVersion: UInt16 = 1

struct Options {
    var fps: Int = 60
    var maxHeight: Int?
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func parseOptions(_ args: [String]) -> Options {
    var options = Options()
    var i = 1
    while i < args.count {
        switch args[i] {
        case "--fps":
            guard i + 1 < args.count, let fps = Int(args[i + 1]), fps > 0 else {
                fail("--fps requires a positive integer")
            }
            options.fps = fps
            i += 2
        case "--max-height":
            guard i + 1 < args.count, let maxHeight = Int(args[i + 1]), maxHeight >= 2 else {
                fail("--max-height requires an integer >= 2")
            }
            options.maxHeight = maxHeight
            i += 2
        case "--version":
            i += 1
        default:
            fail("unknown argument \(args[i])")
        }
    }
    return options
}

func writeBytes(_ bytes: [UInt8]) {
    FileHandle.standardOutput.write(Data(bytes))
}

func writeLittleEndian<T: FixedWidthInteger>(_ value: T) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { raw in
        FileHandle.standardOutput.write(Data(raw))
    }
}

func outputDimensions(sourceWidth: Int, sourceHeight: Int, maxHeight: Int?) -> (width: Int, height: Int) {
    var width = max(2, sourceWidth)
    var height = max(2, sourceHeight)
    if let maxHeight, height > maxHeight {
        let scale = Double(maxHeight) / Double(height)
        width = Int((Double(width) * scale).rounded())
        height = maxHeight
    }
    width = max(2, width - (width % 2))
    height = max(2, height - (height % 2))
    return (width, height)
}

func compactNV12(from pixelBuffer: CVPixelBuffer, width: Int, height: Int) -> [UInt8]? {
    guard CVPixelBufferGetPlaneCount(pixelBuffer) >= 2 else {
        return nil
    }
    CVPixelBufferLockBaseAddress(pixelBuffer, .readOnly)
    defer { CVPixelBufferUnlockBaseAddress(pixelBuffer, .readOnly) }

    let yStride = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 0)
    let uvStride = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 1)
    let yRows = CVPixelBufferGetHeightOfPlane(pixelBuffer, 0)
    let uvRows = CVPixelBufferGetHeightOfPlane(pixelBuffer, 1)
    guard yStride >= width, uvStride >= width, yRows >= height, uvRows >= height / 2 else {
        return nil
    }
    guard
        let yBase = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 0),
        let uvBase = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 1)
    else {
        return nil
    }

    var payload = [UInt8]()
    payload.reserveCapacity(width * height * 3 / 2)
    for row in 0..<height {
        let src = yBase.advanced(by: row * yStride).assumingMemoryBound(to: UInt8.self)
        payload.append(contentsOf: UnsafeBufferPointer(start: src, count: width))
    }
    for row in 0..<(height / 2) {
        let src = uvBase.advanced(by: row * uvStride).assumingMemoryBound(to: UInt8.self)
        payload.append(contentsOf: UnsafeBufferPointer(start: src, count: width))
    }
    return payload
}

final class FrameOutput: NSObject, SCStreamOutput {
    let width: Int
    let height: Int

    init(width: Int, height: Int) {
        self.width = width
        self.height = height
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen else {
            return
        }
        guard CMSampleBufferIsValid(sampleBuffer), let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else {
            return
        }
        guard CVPixelBufferGetPixelFormatType(pixelBuffer) == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange else {
            fail("ScreenCaptureKit delivered unexpected pixel format")
        }
        guard let payload = compactNV12(from: pixelBuffer, width: width, height: height) else {
            fail("ScreenCaptureKit delivered incompatible NV12 frame")
        }
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        let seconds = CMTimeGetSeconds(pts)
        let nanos = UInt64(max(0, seconds.isFinite ? seconds * 1_000_000_000 : 0))
        writeBytes(frameMagic)
        writeLittleEndian(nanos)
        writeLittleEndian(UInt32(payload.count))
        FileHandle.standardOutput.write(Data(payload))
    }
}

final class StreamDelegate: NSObject, SCStreamDelegate {
    func stream(_ stream: SCStream, didStopWithError error: Error) {
        fail("ScreenCaptureKit stream stopped: \(error)")
    }
}

final class CaptureController {
    let options: Options
    var stream: SCStream?
    var output: FrameOutput?
    var delegate: StreamDelegate?

    init(options: Options) {
        self.options = options
    }

    func run() async throws {
        let content = try await SCShareableContent.excludingDesktopWindows(
            false,
            onScreenWindowsOnly: true
        )
        guard let display = content.displays.first(where: { $0.displayID == CGMainDisplayID() })
            ?? content.displays.first
        else {
            fail("ScreenCaptureKit found no displays")
        }

        let sourceWidth = max(2, CGDisplayPixelsWide(display.displayID))
        let sourceHeight = max(2, CGDisplayPixelsHigh(display.displayID))
        let dimensions = outputDimensions(
            sourceWidth: sourceWidth,
            sourceHeight: sourceHeight,
            maxHeight: options.maxHeight
        )

        let configuration = SCStreamConfiguration()
        configuration.width = dimensions.width
        configuration.height = dimensions.height
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(options.fps))
        configuration.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        configuration.queueDepth = 3
        configuration.showsCursor = true
        configuration.capturesAudio = false
        configuration.colorMatrix = CGDisplayStream.yCbCrMatrix_ITU_R_709_2

        let filter = SCContentFilter(display: display, excludingWindows: [])
        let delegate = StreamDelegate()
        let output = FrameOutput(width: dimensions.width, height: dimensions.height)
        let stream = SCStream(filter: filter, configuration: configuration, delegate: delegate)
        let queue = DispatchQueue(label: "clipline.sck.frames")
        try stream.addStreamOutput(output, type: .screen, sampleHandlerQueue: queue)

        self.delegate = delegate
        self.output = output
        self.stream = stream

        writeBytes(streamMagic)
        writeLittleEndian(protocolVersion)
        writeLittleEndian(UInt32(dimensions.width))
        writeLittleEndian(UInt32(dimensions.height))
        writeLittleEndian(UInt32(options.fps))

        try await stream.startCapture()
    }
}

if CommandLine.arguments.contains("--version") {
    FileHandle.standardOutput.write(Data((version + "\n").utf8))
    exit(0)
}

let controller = CaptureController(options: parseOptions(CommandLine.arguments))
Task {
    do {
        try await controller.run()
    } catch {
        fail("ScreenCaptureKit capture failed: \(error)")
    }
}
RunLoop.main.run()
