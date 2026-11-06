// Checkpoints: snapshots of the workspace taken right before the agent's first
// file-changing action in a turn, so /undo can put every file back.
//
// Snapshots live in a hidden git repository per project under
// ~/.config/oah/snapshots/<hash>, separate from the project's own .git (which is
// never touched). The project's .gitignore rules apply, so build output and
// node_modules aren't copied. Bash commands are covered too, since the whole
// tree is snapshotted rather than individual files.
import { createHash } from "node:crypto"
import { existsSync, lstatSync, rmSync } from "node:fs"
import { homedir } from "node:os"
import { join, resolve } from "node:path"
import { ensureDir, paths } from "./config"

export interface Checkpoint {
  id: string
  /** git tree id of the workspace before the turn's first change. */
  tree: string
  /** The prompt that started the turn. */
  label: string
  at: number
  /** Files the turn changed (filled in when the turn ends). */
  files: string[]
}

async function run(args: string[], env: Record<string, string>, cwd: string): Promise<{ ok: boolean; out: string; err: string }> {
  try {
    const p = Bun.spawn(["git", ...args], { cwd, env: { ...process.env, ...env }, stdout: "pipe", stderr: "pipe", stdin: "ignore" })
    const [out, err, code] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text(), p.exited])
    return { ok: code === 0, out, err }
  } catch (e: any) {
    return { ok: false, out: "", err: e?.message ?? String(e) }
  }
}

let gitChecked: boolean | null = null
async function hasGit(): Promise<boolean> {
  if (gitChecked === null) gitChecked = (await run(["--version"], {}, process.cwd())).ok
  return gitChecked
}

export class Snapshots {
  readonly gitDir: string
  private ready = false
  /** Why snapshots are unavailable here, if they are. */
  disabled: string | null = null

  constructor(
    readonly cwd: string,
    readonly sessionId: string,
  ) {
    this.cwd = resolve(cwd)
    const id = createHash("sha256").update(this.cwd).digest("hex").slice(0, 16)
    this.gitDir = join(paths.snapshots, id)
    if (this.cwd === resolve(homedir()) || this.cwd === "/") this.disabled = "checkpoints are off in your home directory and at /"
  }

  private env() {
    return {
      GIT_DIR: this.gitDir,
      GIT_WORK_TREE: this.cwd,
      // One index per session, so two oah windows in a project don't collide.
      GIT_INDEX_FILE: join(this.gitDir, `index-${this.sessionId}`),
      GIT_AUTHOR_NAME: "oah",
      GIT_AUTHOR_EMAIL: "oah@localhost",
      GIT_COMMITTER_NAME: "oah",
      GIT_COMMITTER_EMAIL: "oah@localhost",
    }
  }

  private git(args: string[]) {
    return run(["-c", "core.autocrlf=false", "-c", "core.quotepath=false", "-c", "core.hooksPath=/dev/null", ...args], this.env(), this.cwd)
  }

  private async init(): Promise<boolean> {
    if (this.ready) return true
    if (this.disabled) return false
    if (!(await hasGit())) {
      this.disabled = "checkpoints need git installed"
      return false
    }
    if (!existsSync(join(this.gitDir, "HEAD"))) {
      ensureDir(this.gitDir)
      const r = await run(["init", "-q", "--bare", this.gitDir], {}, this.cwd)
      if (!r.ok) {
        this.disabled = `couldn't set up checkpoints: ${r.err.trim()}`
        return false
      }
      await run(["--git-dir", this.gitDir, "config", "core.bare", "false"], {}, this.cwd)
    }
    this.ready = true
    return true
  }

  /** Snapshot the workspace; returns the tree id, or null if unavailable. */
  async snapshot(): Promise<string | null> {
    if (!(await this.init())) return null
    const add = await this.git(["add", "-A", "--", "."])
    if (!add.ok) {
      this.disabled = `couldn't snapshot files: ${add.err.trim().split("\n")[0]}`
      return null
    }
    const tree = await this.git(["write-tree"])
    return tree.ok ? tree.out.trim() : null
  }

  /** Files that differ between two trees. */
  async diff(from: string, to: string): Promise<string[]> {
    const r = await this.git(["diff-tree", "-r", "--name-only", "--no-renames", from, to])
    return r.ok ? r.out.split("\n").filter(Boolean) : []
  }

  /** Put the workspace back to `tree`: restore changed files, delete files created since. Returns the files touched. */
  async restore(tree: string): Promise<string[]> {
    const now = await this.snapshot()
    if (!now) throw new Error(this.disabled ?? "couldn't read the current files")
    const added = await this.git(["diff-tree", "-r", "--name-only", "--no-renames", "--diff-filter=A", tree, now])
    const touched = await this.diff(tree, now)
    const read = await this.git(["read-tree", tree])
    if (!read.ok) throw new Error(read.err.trim())
    const co = await this.git(["checkout-index", "-a", "-f"])
    if (!co.ok) throw new Error(co.err.trim())
    for (const f of added.out.split("\n").filter(Boolean)) {
      const p = join(this.cwd, f)
      if (!p.startsWith(this.cwd + "/")) continue
      try {
        const st = lstatSync(p)
        if (st.isFile() || st.isSymbolicLink()) rmSync(p, { force: true })
      } catch {}
    }
    return touched
  }
}
