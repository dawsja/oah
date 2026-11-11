// swift-tools-version:5.3

import PackageDescription

let package = Package(
  name: "tauri-plugin-oah-phone",
  platforms: [
    .macOS(.v10_15),
    .iOS(.v15),
  ],
  products: [
    .library(
      name: "tauri-plugin-oah-phone",
      type: .static,
      targets: ["tauri-plugin-oah-phone"])
  ],
  dependencies: [
    // Copied here by the crate's build (tauri-plugin).
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-oah-phone",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
