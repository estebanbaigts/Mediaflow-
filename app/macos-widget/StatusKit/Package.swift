// swift-tools-version:5.9
// Model and formatting shared by the MediaFlow widget. Kept in a package so
// it can be unit-tested with `swift test` without Xcode or signing.
import PackageDescription

let package = Package(
    name: "StatusKit",
    platforms: [.macOS(.v14)],
    products: [.library(name: "StatusKit", targets: ["StatusKit"])],
    targets: [
        .target(name: "StatusKit"),
        .testTarget(name: "StatusKitTests", dependencies: ["StatusKit"]),
    ]
)
