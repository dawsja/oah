// Memory: instruction files plus persistent notes.
//  * Instruction files: ~/.config/oah/AGENTS.md, then for each directory from the
//    repository root down to cwd the first of AGENTS.md, OAH.md, CLAUDE.md.
//  * Notes: ~/.config/oah/memory/global.md and a per-project file, added with
//    `#note`, `/memory add`, or the model's `memory` tool.
import { createHash } from "node:crypto"
import { existsSync, readFileSync, statSync } from "node:fs"
import { basename, dirname, join } from "node:path"
import { paths, shortPath, writeFileAtomic } from "./config"

export function projectRoot(cwd: string): string {
  let d = cwd
  for (;;) {
    if (existsSync(join(d, ".git"))) return d
    const up = dirname(d)
    if (up === d) return cwd
    d = up
  }
}

export class Memory {
  root: string
  globalFile: string
  projectFile: string

  constructor(public cwd: string) {
    this.root = projectRoot(cwd)
    const id = createHash("sha256").update(this.root).digest("hex").slice(0, 16)
    this.globalFile = join(paths.memory, "global.md")
    this.projectFile = join(paths.memory, "projects", `${basename(this.root) || "root"}-${id}.md`)
  }

  private cached = new Map<string, { key: string; text: string }>()

  private content(file: string): string {
    const stat = statSync(file)
    const key = `${stat.mtimeMs}:${stat.ctimeMs}:${stat.size}`
    const hit = this.cached.get(file)
    if (hit?.key === key) return hit.text
    const text = readFileSync(file, "utf8")
    this.cached.set(file, { key, text })
    return text
  }

  instructionFiles(): string[] {
    const out: string[] = []
    if (existsSync(paths.agents)) out.push(paths.agents)
    const chain: string[] = []
    let d = this.cwd
    for (;;) {
      chain.unshift(d)
      if (d === this.root) break
      const up = dirname(d)
      if (up === d) break
      d = up
    }
    for (const dir of chain) {
      const f = ["AGENTS.md", "OAH.md", "CLAUDE.md"].map((n) => join(dir, n)).find(existsSync)
      if (f) out.push(f)
    }
    return out
  }

  private entries(file: string): string[] {
    try {
      return this.content(file)
        .split("\n")
        .filter((l) => l.startsWith("- "))
        .map((l) => l.slice(2).trim())
        .filter(Boolean)
    } catch {
      return []
    }
  }

  private write(file: string, title: string, entries: string[]) {
    writeFileAtomic(file, `# ${title}\n\n${entries.map((e) => `- ${e}`).join("\n")}\n`)
    this.cached.delete(file)
  }

  add(text: string, global = false) {
    const t = text.trim().replace(/\n/g, " ")
    if (!t) throw new Error("nothing to remember")
    const file = global ? this.globalFile : this.projectFile
    const e = this.entries(file)
    if (!e.includes(t)) e.push(t)
    this.write(file, global ? "oah global memory" : `oah memory for ${this.root}`, e)
  }

  remove(needle: string, global = false): number {
    const n = needle.trim().toLowerCase()
    let removed = 0
    const targets: [string, string][] = global
      ? [[this.globalFile, "oah global memory"]]
      : [
          [this.projectFile, `oah memory for ${this.root}`],
          [this.globalFile, "oah global memory"],
        ]
    for (const [file, title] of targets) {
      const e = this.entries(file)
      const kept = e.filter((x) => n && !x.toLowerCase().includes(n))
      if (kept.length !== e.length) {
        removed += e.length - kept.length
        this.write(file, title, kept)
      }
    }
    return removed
  }

  render(): string {
    const g = this.entries(this.globalFile)
    const p = this.entries(this.projectFile)
    let s = ""
    if (g.length) s += `Global:\n${g.map((e) => `- ${e}`).join("\n")}\n`
    if (p.length) s += `This project:\n${p.map((e) => `- ${e}`).join("\n")}\n`
    return s
  }

  promptSection(): string {
    const sections: { heading: string; body: string }[] = []
    const files = this.instructionFiles() // rediscover additions, deletions and precedence changes
    for (const f of files) {
      try { sections.push({ heading: `# Instructions from ${shortPath(f)}`, body: this.content(f).trim() }) } catch {}
    }
    for (const file of this.cached.keys()) if (!files.includes(file) && file !== this.globalFile && file !== this.projectFile) this.cached.delete(file)
    const notes = this.render()
    if (notes) sections.push({ heading: "# Memory (persistent user notes)", body: notes })
    if (!sections.length) return ""
    // Fair shares preserve visibility of every instruction source, including the most-local file.
    const budget = 16_000
    const per = Math.max(0, Math.floor(budget / sections.length) - 200)
    return sections.map(({ heading, body }) => `${heading}\n${body.length > per ? body.slice(0, per) + "\n[Instructions truncated by total 16k-character budget; read the source file for complete rules.]" : body}\n`).join("\n").slice(0, budget)
  }
}
