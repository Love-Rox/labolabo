// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "LaboLabo",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .library(name: "LaboLaboEngine", targets: ["LaboLaboEngine"]),
        .library(name: "LaboLaboStore", targets: ["LaboLaboStore"]),
    ],
    dependencies: [
        .package(url: "https://github.com/groue/GRDB.swift", from: "7.0.0"),
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
        .target(
            name: "LaboLaboStore",
            dependencies: [
                .product(name: "GRDB", package: "GRDB.swift")
            ],
            swiftSettings: [
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
        .testTarget(
            name: "LaboLaboStoreTests",
            dependencies: ["LaboLaboStore"],
            swiftSettings: [
                .swiftLanguageMode(.v5)
            ]
        ),
    ]
)
