// swift-tools-version: 6.2
import PackageDescription

// `scripts/bundle` builds the Rust static library and writes the UniFFI bindings this links:
// `Sources/launcher_ffiFFI` (C header and module map) and `Sources/Launcher/Generated`.
let package = Package(
    name: "Launcher",
    platforms: [.macOS(.v26)],
    targets: [
        .systemLibrary(name: "launcher_ffiFFI", path: "Sources/launcher_ffiFFI"),
        .executableTarget(
            name: "Launcher",
            dependencies: ["launcher_ffiFFI"],
            linkerSettings: [
                .unsafeFlags(["-L", Context.packageDirectory + "/../target/release"]),
                .linkedLibrary("launcher_ffi"),
            ]
        ),
    ]
)
