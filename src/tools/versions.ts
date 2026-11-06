import { createHash } from "node:crypto"
import { readFileSync, realpathSync, statSync } from "node:fs"
import { dirname, join, basename } from "node:path"

export function canonicalPath(path: string): string {
  try { return realpathSync(path) } catch {
    const parent = dirname(path)
    return parent === path ? path : join(canonicalPath(parent), basename(path))
  }
}
export const digest = (bytes: string | Buffer) => createHash("sha256").update(bytes).digest("hex")
export function fileVersion(path: string): string | null {
  try {
    if (!statSync(path).isFile()) return "!non-file"
    return digest(readFileSync(path))
  } catch (e: any) {
    return e.code === "ENOENT" || e.code === "ENOTDIR" ? null : `!unreadable:${e.code ?? "unknown"}`
  }
}
export function assertFresh(path: string, versions?: Map<string, string | null>) {
  const key = canonicalPath(path)
  const current = fileVersion(path)
  if ([path, key].some((p) => versions?.has(p) && versions.get(p) !== current)) throw Error("file changed on disk since you read it; re-read before editing (no files changed)")
}
export function rememberVersion(path: string, versions?: Map<string, string | null>, bytes?: Buffer | string) {
  const hash = bytes === undefined ? fileVersion(path) : digest(bytes)
  versions?.set(path, hash)
  versions?.set(canonicalPath(path), hash)
}
