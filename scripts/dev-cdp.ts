// Drives oah (dev)'s UI over the DevTools protocol: started with `bun run dev:debug` in app/. Clicks
// and keys go to the WebView itself, never the real mouse or keyboard, so it works while you use
// the PC. It only attaches to the dev server's page (localhost:1420): prod never has the port.
//
// bun scripts/dev-cdp.ts shot <file.png>         screenshot of the window's page
// bun scripts/dev-cdp.ts eval <js>               runs <js> in the page, prints the result
// bun scripts/dev-cdp.ts click <css selector>    clicks the first match's centre
// bun scripts/dev-cdp.ts click-text <text>       clicks a button, link, menu item or list item
//                                                whose text is exactly <text>
// bun scripts/dev-cdp.ts type <text>             types into the focused element (a pane: click
//                                                `.xterm-screen` first); \n presses Enter
// bun scripts/dev-cdp.ts key <Key>               presses one key: Enter, Escape, Tab, ArrowDown…,
//                                                Ctrl+C (any Ctrl+<letter>)
// bun scripts/dev-cdp.ts drag <x,y> <x,y> [hold]  presses at the first point, moves to the second
//                                                and lets go (`hold`: stays pressed, for a shot)
// bun scripts/dev-cdp.ts release <x,y>           lets go of a held drag there
// bun scripts/dev-cdp.ts hover <x,y>             moves the pointer there, nothing pressed
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const port = process.env.OAH_CDP_PORT ?? "9223";
const [cmd, ...rest] = process.argv.slice(2);
const arg = rest.join(" ");
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Target = { type: string; url: string; webSocketDebuggerUrl: string };
let targets: Target[];
try {
  targets = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()) as Target[];
} catch {
  console.error(`Nothing on 127.0.0.1:${port}. Start oah (dev) with \`bun run dev:debug\`.`);
  process.exit(1);
}
const page = targets.find((t) => t.type === "page" && t.url.includes("localhost:1420"));
if (!page) {
  console.error(`No oah (dev) page on ${port} (${targets.map((t) => t.url).join(", ")}).`);
  process.exit(1);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let id = 0;
type Reply = { resolve: (v: Record<string, unknown>) => void; reject: (e: Error) => void };
const pending = new Map<number, Reply>();
ws.addEventListener("message", (e) => {
  const m = JSON.parse(String(e.data)) as {
    id?: number;
    result?: Record<string, unknown>;
    error?: { message: string };
  };
  const reply = m.id ? pending.get(m.id) : undefined;
  if (!reply) return;
  pending.delete(m.id!);
  if (m.error) reply.reject(new Error(m.error.message));
  else reply.resolve(m.result ?? {});
});
const send = (method: string, params: object = {}) =>
  new Promise<Record<string, unknown>>((resolve, reject) => {
    pending.set(++id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
const evaluate = async (expression: string) => {
  const r = (await send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  })) as { result?: { value?: unknown }; exceptionDetails?: { text: string } };
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.text);
  return r.result?.value;
};
const clickAt = async (find: string, what: string) => {
  const at = (await evaluate(
    `(() => { const r = (${find})?.getBoundingClientRect(); return r && [r.x + r.width / 2, r.y + r.height / 2]; })()`,
  )) as [number, number] | undefined;
  if (!at) throw new Error(`Nothing matches ${what}`);
  for (const type of ["mousePressed", "mouseReleased"])
    await send("Input.dispatchMouseEvent", {
      type,
      x: at[0],
      y: at[1],
      button: "left",
      clickCount: 1,
    });
};
const point = (s?: string) => {
  const [x, y] = (s ?? "").split(",").map(Number);
  if (!Number.isFinite(x) || !Number.isFinite(y)) throw new Error(`Not a point: ${s} (x,y)`);
  return { x, y };
};
const mouse = (type: string, at: { x: number; y: number }, buttons = 1) =>
  send("Input.dispatchMouseEvent", { type, ...at, button: "left", buttons, clickCount: 1 });
const KEYS: Record<string, [string, number, string?]> = {
  Enter: ["Enter", 13, "\r"],
  Escape: ["Escape", 27],
  Tab: ["Tab", 9, "\t"],
  Backspace: ["Backspace", 8],
  ArrowUp: ["ArrowUp", 38],
  ArrowDown: ["ArrowDown", 40],
  ArrowLeft: ["ArrowLeft", 37],
  ArrowRight: ["ArrowRight", 39],
};
const key = async (name: string) => {
  // Ctrl+<letter> (Ctrl+C stops what runs in a pane): modifiers 2 is Ctrl.
  const ctrl = /^Ctrl\+([A-Za-z])$/.exec(name);
  if (ctrl) {
    const letter = ctrl[1].toLowerCase();
    const base = {
      key: letter,
      code: `Key${letter.toUpperCase()}`,
      windowsVirtualKeyCode: letter.toUpperCase().charCodeAt(0),
      modifiers: 2,
    };
    await send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
    await send("Input.dispatchKeyEvent", {
      type: "char",
      text: String.fromCharCode(letter.charCodeAt(0) - 96),
      ...base,
    });
    await send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    return;
  }
  const k = KEYS[name];
  if (!k) throw new Error(`Unknown key ${name} (${Object.keys(KEYS).join(", ")}, Ctrl+<letter>)`);
  const [code, vk, text] = k;
  const base = { key: code, code, windowsVirtualKeyCode: vk };
  await send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
  if (text) await send("Input.dispatchKeyEvent", { type: "char", text, ...base });
  await send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
};

switch (cmd) {
  case "shot": {
    const file = arg || "dev.png";
    const r = (await send("Page.captureScreenshot", { format: "png" })) as { data: string };
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, Buffer.from(r.data, "base64"));
    console.log(file);
    break;
  }
  case "eval":
    console.log(JSON.stringify(await evaluate(arg), null, 2));
    break;
  case "click":
    await clickAt(`document.querySelector(${JSON.stringify(arg)})`, arg);
    break;
  case "click-text":
    await clickAt(
      `[...document.querySelectorAll("button, a, li, [role=button], [role=menuitem], [role=tab]")].find((e) => e.textContent.trim() === ${JSON.stringify(arg)})`,
      `"${arg}"`,
    );
    break;
  case "type": {
    const lines = arg.replace(/\\n/g, "\n").split("\n");
    for (const [i, line] of lines.entries()) {
      if (line) await send("Input.insertText", { text: line });
      if (i < lines.length - 1) await key("Enter");
    }
    break;
  }
  case "key":
    await key(arg);
    break;
  case "drag": {
    const [from, to] = [point(rest[0]), point(rest[1])];
    await mouse("mousePressed", from);
    for (let i = 1; i <= 12; i++) {
      const f = i / 12;
      await mouse("mouseMoved", {
        x: from.x + (to.x - from.x) * f,
        y: from.y + (to.y - from.y) * f,
      });
      await sleep(16);
    }
    if (rest[2] !== "hold") await mouse("mouseReleased", to, 0);
    break;
  }
  case "hover":
    await mouse("mouseMoved", point(rest[0]), 0);
    break;
  case "release":
    await mouse("mouseReleased", point(rest[0]), 0);
    break;
  default:
    console.error(
      "Usage: bun scripts/dev-cdp.ts shot|eval|click|click-text|type|key|drag|release|hover <arg>",
    );
    process.exit(2);
}
// Let the page react before the next command looks at it.
await sleep(400);
ws.close();
process.exit(0);
