// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "SynctrMenuBar",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "SynctrMenuBar", targets: ["SynctrMenuBar"]),
    ],
    targets: [
        .executableTarget(
            name: "SynctrMenuBar",
            path: "Sources/SynctrMenuBar"
        ),
    ]
)
