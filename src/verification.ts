export interface VerificationState {
  revision: number
  checkedRevision?: number
  status: "not-needed" | "unverified" | "failed" | "checks-passed"
  commands: { command: string; ok: boolean; revision: number }[]
}

/** Recognize actual check invocations, not arbitrary shell text containing 'test'. */
export function isVerificationCommand(command: string): boolean {
  command = command.replace(/\s+2>&1(?=\s|$)/g, "")
  // AND-only chains preserve failure exit codes, but every segment must itself be a check.
  if (command.includes("&&")) return command.split("&&").every((part) => part.trim() && isVerificationCommand(part))
  if (/[\n\r;|&`<>]|\$\(/.test(command)) return false
  if (/(?:^|\s)(?:--?(?:help|version|init|watch|fix|write|update[\w-]*)|-[hwuv])(?:[=\s]|$)/i.test(command)) return false
  return /^(?:bun|npm|pnpm|yarn)\s+(?:(?:run|exec|x)\s+)?(?:test(?::[\w-]+)?|typecheck|check|lint|build|tsc|eslint)(?:\s|$)|^(?:pytest|tsc|eslint|ruff\s+check|go\s+test|cargo\s+(?:test|check)|git\s+diff\s+--check)(?:\s|$)/.test(command.trim())
}
export class Verification {
  state: VerificationState
  constructor(previous?: VerificationState) {
    this.state = previous ? structuredClone(previous) : { revision: 0, status: "not-needed", commands: [] }
  }
  wrote() { this.state.revision++; this.state.status = "unverified" }
  checked(command: string, ok: boolean, revision: number) {
    this.state.commands.push({ command, ok, revision })
    this.state.commands = this.state.commands.slice(-12)
    if (revision !== this.state.revision) return
    this.state.checkedRevision = revision
    const latest = new Map(this.state.commands.filter((c) => c.revision === revision).map((c) => [c.command, c]))
    this.state.status = [...latest.values()].some((c) => !c.ok) ? "failed" : ok ? "checks-passed" : "failed"
  }
  get pending() { return this.state.status === "unverified" || this.state.status === "failed" }
}
