import type { LanguageModelUsage } from "ai"

export interface TokenUsage {
  input: number
  output: number
  inputTokenDetails?: LanguageModelUsage["inputTokenDetails"]
}

/** Main-context occupancy is separate from usage billed to auxiliary requests. */
export interface UsageTotals {
  input: number
  output: number
  cacheRead: number
  cacheWrite: number
  cacheReportedInput: number
  requests: number
}

export function addUsage(total: UsageTotals | undefined, u: TokenUsage): UsageTotals {
  const t = total ?? { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cacheReportedInput: 0, requests: 0 }
  const d = u.inputTokenDetails
  return {
    input: t.input + u.input, output: t.output + u.output,
    cacheRead: t.cacheRead + (d?.cacheReadTokens ?? 0),
    cacheWrite: t.cacheWrite + (d?.cacheWriteTokens ?? 0),
    cacheReportedInput: t.cacheReportedInput + (d?.cacheReadTokens !== undefined ? u.input : 0),
    requests: t.requests + 1,
  }
}

export function cacheHitPercent(t?: UsageTotals): number | undefined {
  return t?.cacheReportedInput ? Math.round(100 * t.cacheRead / t.cacheReportedInput) : undefined
}
