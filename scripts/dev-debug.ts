// oah in dev mode with WebView2's DevTools port open (127.0.0.1:9223, or OAH_CDP_PORT), so an
// agent can drive and screenshot the UI with scripts/dev-cdp.ts without touching the real mouse
// or keyboard. It's `tauri dev` plus one browser argument. Run from app/: `bun run dev:debug`.
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

const port = process.env.OAH_CDP_PORT ?? "9223";
const app = join(import.meta.dir, "..", "app");
const conf = (await Bun.file(join(app, "src-tauri/tauri.conf.json")).json()) as {
  app: { windows: { additionalBrowserArgs?: string }[] };
};
// A JSON merge patch replaces arrays whole, so every window goes in with its own settings.
const windows = conf.app.windows.map((w) => ({
  ...w,
  additionalBrowserArgs: [
    w.additionalBrowserArgs ?? "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
    `--remote-debugging-port=${port}`,
  ].join(" "),
}));
const patch = join(tmpdir(), "oah-dev-debug.json");
await Bun.write(patch, JSON.stringify({ app: { windows } }));
console.log(`oah (dev), DevTools on 127.0.0.1:${port}`);
// Dev runs keep their data in .dev-home/ (not your real ~/.config/oah) unless OAH_HOME is set.
const env = { ...process.env, OAH_HOME: process.env.OAH_HOME ?? join(app, "..", ".dev-home") };
console.log(`data: ${env.OAH_HOME}`);
const child = spawn("bun", ["x", "tauri", "dev", "--config", patch], {
  cwd: app,
  env,
  stdio: "inherit",
  shell: process.platform === "win32",
});
child.on("exit", (code) => process.exit(code ?? 0));
