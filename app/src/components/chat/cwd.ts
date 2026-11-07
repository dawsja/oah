import { createContext, useContext } from "react"
import { relative } from "@/lib/chat"

/** The open chat's working folder; paths under it are shown relative. */
export const CwdContext = createContext<string | null>(null)

export function useRelative() {
  const cwd = useContext(CwdContext)
  return (text: string) => relative(text, cwd)
}
