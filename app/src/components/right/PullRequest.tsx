import type { Scope } from "@/lib/git"

/** Pull request for the current branch (filled in with `gh` next). */
export function PullRequest({ scope }: { scope: Scope }) {
  void scope
  return <div className="m-3 border border-dashed p-3 text-[12px] text-muted-foreground">Pull requests for this branch show up here.</div>
}
