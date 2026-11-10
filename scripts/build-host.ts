#!/usr/bin/env bun
// Builds oah-host for the SSH hosts oah supports (static musl, so any Linux runs it) and puts
// the binaries where the desktop app bundles them: app/src-tauri/host/oah-host-<arch>-linux.
//
//   bun scripts/build-host.ts
//
// Needs cargo-zigbuild and zig (`pip install cargo-zigbuild` brings both), and the two Rust
// targets (added here when rustup is there).

import { $ } from "bun"
import { copyFileSync, mkdirSync, statSync } from "node:fs"
import { join } from "node:path"

const root = (await $`git rev-parse --show-toplevel`.text()).trim()
process.chdir(root)

const targets: [string, string][] = [
  ["x86_64-unknown-linux-musl", "x86_64-linux"],
  ["aarch64-unknown-linux-musl", "aarch64-linux"],
]

if ((await $`cargo zigbuild --help`.quiet().nothrow()).exitCode !== 0) {
  console.error("build-host: cargo-zigbuild isn't installed (pip install cargo-zigbuild)")
  process.exit(1)
}
await $`rustup target add ${targets.map(([t]) => t)}`.nothrow()
await $`cargo zigbuild --release --locked -p oah-host ${targets.flatMap(([t]) => ["--target", t])}`

const out = join(root, "app/src-tauri/host")
mkdirSync(out, { recursive: true })
for (const [target, platform] of targets) {
  const from = join(root, "target", target, "release", "oah-host")
  const to = join(out, `oah-host-${platform}`)
  copyFileSync(from, to)
  console.log(`${to} (${Math.round(statSync(to).size / 1024)} KiB)`)
}
