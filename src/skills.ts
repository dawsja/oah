// Agent Skills: folders with a SKILL.md (front matter `name`, `description`).
// Only names/descriptions go in the system prompt; `load_skill` (or /<name>)
// loads the body. Search order (first name wins): ./.oah/skills,
// ./.agents/skills, ./.claude/skills, ~/.config/oah/skills, ~/.agents/skills,
// ~/.claude/skills.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs"
import { homedir } from "node:os"
import { basename, dirname, join, relative } from "node:path"
import { paths } from "./config"

export interface Skill {
  name: string
  description: string
  path: string
  scope: "project" | "user"
}

export function parseFrontMatter(text: string): { meta: Record<string, string>; body: string } {
  const t = text.replace(/^﻿/, "")
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(t)
  if (!m) return { meta: {}, body: t }
  const meta: Record<string, string> = {}
  const lines = m[1]!.split("\n")
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!
    const kv = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line)
    if (!kv) continue
    let v = kv[2]!.trim()
    if ([">", "|", ">-", "|-"].includes(v)) {
      const parts: string[] = []
      while (i + 1 < lines.length && (lines[i + 1]!.startsWith(" ") || !lines[i + 1]!.trim())) parts.push(lines[++i]!.trim())
      v = parts.join(" ").trim()
    }
    meta[kv[1]!] = v.replace(/^["']|["']$/g, "")
  }
  return { meta, body: m[2]! }
}

export class Skills {
  list: Skill[] = []

  constructor(private cwd: string) {
    this.reload()
  }

  /** Re-scan the skill folders (after installing or removing one). */
  reload() {
    const cwd = this.cwd
    this.list = []
    const roots: [string, Skill["scope"]][] = [
      [join(cwd, ".oah/skills"), "project"],
      [join(cwd, ".agents/skills"), "project"],
      [join(cwd, ".claude/skills"), "project"],
      [paths.skills, "user"],
      [join(homedir(), ".agents/skills"), "user"],
      [join(homedir(), ".claude/skills"), "user"],
    ]
    for (const [root, scope] of roots) {
      let dirs: string[] = []
      try {
        dirs = readdirSync(root).map((d) => join(root, d)).filter((d) => statSync(d).isDirectory()).sort()
      } catch {
        continue
      }
      for (const dir of dirs) {
        const file = join(dir, "SKILL.md")
        if (!existsSync(file)) continue
        const { meta, body } = parseFrontMatter(readFileSync(file, "utf8"))
        const name = meta.name ?? basename(dir)
        if (this.list.some((s) => s.name === name)) continue
        const description = meta.description ?? body.split("\n").find((l) => l.trim() && !l.startsWith("#"))?.trim() ?? ""
        this.list.push({ name, description, path: file, scope })
      }
    }
  }

  get(name: string): Skill | undefined {
    const n = name.trim().replace(/^\//, "").toLowerCase()
    return this.list.find((s) => s.name.toLowerCase() === n)
  }

  load(name: string): string | undefined {
    const s = this.get(name)
    if (!s) return undefined
    const { body } = parseFrontMatter(readFileSync(s.path, "utf8"))
    const dir = dirname(s.path)
    const files: string[] = []
    const visit = (d: string, depth: number) => {
      if (depth > 3 || files.length >= 40) return
      for (const e of readdirSync(d, { withFileTypes: true })) {
        const p = join(d, e.name)
        if (e.isDirectory()) visit(p, depth + 1)
        else if (p !== s.path) files.push(relative(dir, p))
      }
    }
    try {
      visit(dir, 0)
    } catch {}
    let out = `# Skill: ${s.name}\nBase directory: ${dir}\n\n${body.trim()}`
    if (files.length) out += `\n\nBundled files (relative to the base directory):\n${files.map((f) => `- ${f}`).join("\n")}`
    return out
  }

  promptSection(): string {
    if (!this.list.length) return ""
    return (
      "# Skills\nSkills are packaged instructions for specific tasks. When a task matches a skill's description, call `load_skill` with its name first and follow it.\n" +
      this.list.map((s) => `- ${s.name}: ${s.description}`).join("\n") +
      "\n"
    )
  }
}
