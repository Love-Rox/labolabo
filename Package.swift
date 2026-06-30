// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "LaboLabo",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .library(name: "LaboLaboEngine", targets: ["LaboLaboEngine"]),
    ],
    targets: [
        .target(
            name: "LaboLaboEngine",
            swiftSettings: [
                // Spike-phase: keep Swift 5 language mode to avoid being blocked by
                // strict-concurrency churn. Tighten to .v6 once the engine settles.
                .swiftLanguageMode(.v5)
            ]
        ),
        .testTarget(
            name: "LaboLaboEngineTests",
            dependencies: ["LaboLaboEngine"],
            swiftSettings: [
                .swiftLanguageMode(.v5)
            ]
        ),
    ]
)
