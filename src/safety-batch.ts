import type { LanguageModelMiddleware } from "ai"

export interface ProposedCall { id: string; name: string; args: unknown; invalid?: boolean }

/** Provider-stream barrier: all final calls are known before SDK execution starts.
 * No debounce, execution-time rendezvous, or approvals inferred from argv shapes.
 */
export function reviewBarrier(review: (calls: ProposedCall[]) => Promise<void>): LanguageModelMiddleware {
  return {
    specificationVersion: "v4",
    wrapStream: async ({ doStream }) => {
      const result = await doStream()
      const calls: ProposedCall[] = []
      return { ...result, stream: result.stream.pipeThrough(new TransformStream({
        async transform(part, controller) {
          if (part.type === "tool-call" && !part.providerExecuted) {
            try { calls.push({ id: part.toolCallId, name: part.toolName, args: JSON.parse(part.input) }) } catch { calls.push({ id: part.toolCallId, name: part.toolName, args: undefined, invalid: true }) } // repaired calls get an individual review
          }
          if (part.type === "finish") await review(calls)
          controller.enqueue(part)
        },
      })) }
    },
  }
}
