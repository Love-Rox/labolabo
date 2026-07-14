// swift-tools-version: 6.0
import PackageDescription

// LaboLaboStore（GRDB）は Ubuntu 標準の libsqlite3（SQLITE_ENABLE_SNAPSHOT 無効ビルド）だと
// GRDB の WAL スナップショット API（sqlite3_snapshot_*）がリンクできない。SwiftPM の Linux
// テストランナーは全 testTarget を 1 つの実行ファイルへまとめてリンクするため、対象を
// `swift test --filter` で絞ってもこのリンクエラーは避けられない。そのため Linux では
// LaboLaboStore/LaboLaboStoreTests と GRDB 依存そのものをマニフェストから外し、
// エンジン層（LaboLaboEngine/LaboLaboEngineTests）だけを対象に `swift test` できるようにする。
// macOS 側の対象・依存・挙動は変えない。
#if os(Linux)
let products: [Product] = [
    .library(name: "LaboLaboEngine", targets: ["LaboLaboEngine"])
]
let dependencies: [Package.Dependency] = []
let targets: [Target] = [
    .target(
        name: "LaboLaboEngine",
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
]
#else
let products: [Product] = [
    .library(name: "LaboLaboEngine", targets: ["LaboLaboEngine"]),
    .library(name: "LaboLaboStore", targets: ["LaboLaboStore"]),
]
let dependencies: [Package.Dependency] = [
    .package(url: "https://github.com/groue/GRDB.swift", from: "7.0.0"),
]
let targets: [Target] = [
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
#endif

let package = Package(
    name: "LaboLabo",
    platforms: [
        .macOS(.v14)
    ],
    products: products,
    dependencies: dependencies,
    targets: targets
)
