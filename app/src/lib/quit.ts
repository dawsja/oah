// Leaving oah (closing the window, restarting into an update) while agents are working.

import { ipc } from "./ipc"

/** True when it's fine to stop every agent now: none is mid-turn, or the user says so. Then
 *  they're stopped, with what they'd streamed kept. */
export async function stopAgentsForQuit(action: string): Promise<boolean> {
  const [busy, cli] = await Promise.all([ipc.chatBusy().catch(() => [] as string[]), ipc.chatCli().catch(() => [] as string[])])
  if (busy.length) {
    const { confirmAction } = await import("@/components/Prompt")
    const n = busy.length === 1 ? "An agent is" : `${busy.length} agents are`
    const ok = await confirmAction({
      title: `${n} still working`,
      description: `${action} stops ${busy.length === 1 ? "it" : "them"} mid-turn. What they've written so far is kept, and the chats carry on from there next time.${cli.length ? ` The ${cli.length === 1 ? "CLI" : `${cli.length} CLIs`} open in chats end too.` : ""}`,
      confirm: action,
      danger: true,
    })
    if (!ok) return false
  } else if (cli.length) {
    // An agent's CLI may be mid-turn too; only it knows.
    const { confirmAction } = await import("@/components/Prompt")
    const ok = await confirmAction({
      title: cli.length === 1 ? "A chat is open in its agent's CLI" : `${cli.length} chats are open in their agents' CLIs`,
      description: `${action} ends ${cli.length === 1 ? "it" : "them"}, mid-turn if the agent is working there. The chats carry on from what was said next time.`,
      confirm: action,
      danger: true,
    })
    if (!ok) return false
  }
  await ipc.chatsStopAll().catch(() => undefined)
  return true
}
