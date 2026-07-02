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
                // Swift 6 language mode (strict concurrency). エンジン層は actor /
                // Sendable で組んであり、プロセス実行の drain は DataBox 経由で安全化済み。
                .swiftLanguageMode(.v6)
            ]
        ),
        .target(
            name: "LaboLaboStore",
            dependencies: [
                .product(name: "GRDB", package: "GRDB.swift")
            ],
            swiftSettings: [
                .swiftLanguageMode(.v6)
            ]
        ),
        .testTarget(
            name: "LaboLaboEngineTests",
            dependencies: ["LaboLaboEngine"],
            swiftSettings: [
                .swiftLanguageMode(.v6)
            ]
        ),
        .testTarget(
            name: "LaboLaboStoreTests",
            dependencies: ["LaboLaboStore"],
            swiftSettings: [
                .swiftLanguageMode(.v6)
            ]
        ),
    ]
)
