import Foundation

let version = "clipline-sck-helper 1"

if CommandLine.arguments.contains("--version") {
    FileHandle.standardOutput.write(Data((version + "\n").utf8))
    exit(0)
}

FileHandle.standardError.write(Data("ScreenCaptureKit capture entrypoint is not wired\n".utf8))
exit(64)
