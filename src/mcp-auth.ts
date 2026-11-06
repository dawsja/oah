// OAuth for remote MCP servers (the MCP authorization spec, via @ai-sdk/mcp's
// `auth`). Tokens, dynamically registered clients and the authorization-server
// pin are kept per server in ~/.config/oah/mcp-auth.json (0600).
//
// At startup servers connect with a "probe" provider: if the server wants a
// sign-in it is marked "needs authentication" instead of opening a browser.
// `/mcp` then runs the browser flow ("login" provider + a loopback callback).
import { auth, type OAuthClientInformation, type OAuthClientProvider, type OAuthTokens } from "@ai-sdk/mcp"
import { paths, readJson, writeFileAtomic } from "./config"

const CALLBACK_PORT = 19877
const REDIRECT_URL = `http://127.0.0.1:${CALLBACK_PORT}/callback`

interface Entry {
  tokens?: OAuthTokens
  client?: OAuthClientInformation
  dynamic?: boolean
  server?: { issuer?: string; authorizationServerUrl: string; tokenEndpoint: string }
  verifier?: string
  state?: string
}

const key = (name: string, url: string) => `${name} ${url}`
const readAll = () => readJson<Record<string, Entry>>(paths.mcpAuth, {})

function update(name: string, url: string, f: (e: Entry) => void) {
  const all = readAll()
  const e = (all[key(name, url)] ??= {})
  f(e)
  writeFileAtomic(paths.mcpAuth, JSON.stringify(all, null, 2), true)
}

export const hasTokens = (name: string, url: string) => !!readAll()[key(name, url)]?.tokens

export function forget(name: string, url: string): boolean {
  const all = readAll()
  if (!all[key(name, url)]) return false
  delete all[key(name, url)]
  writeFileAtomic(paths.mcpAuth, JSON.stringify(all, null, 2), true)
  return true
}

/** Thrown at startup instead of opening a browser. */
export class NeedsAuth extends Error {
  constructor() {
    super("needs authentication")
  }
}

export class McpOAuth implements OAuthClientProvider {
  /** Set when the server asked for a sign-in. */
  wantsAuth = false

  constructor(
    private name: string,
    private url: string,
    /** Called with the authorization URL during an interactive sign-in; absent → probe mode. */
    private onAuthorize?: (url: string) => void,
  ) {}

  private get entry(): Entry {
    return readAll()[key(this.name, this.url)] ?? {}
  }

  get redirectUrl() {
    return REDIRECT_URL
  }
  get clientMetadata() {
    return {
      redirect_uris: [REDIRECT_URL],
      client_name: "oah",
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      token_endpoint_auth_method: "none",
    }
  }
  tokens() {
    return this.entry.tokens
  }
  saveTokens(tokens: OAuthTokens) {
    update(this.name, this.url, (e) => (e.tokens = tokens))
  }
  clientInformation() {
    return this.entry.client
  }
  saveClientInformation(info: OAuthClientInformation) {
    update(this.name, this.url, (e) => {
      e.client = info
      e.dynamic = true
    })
  }
  isClientInformationDynamicallyRegistered() {
    return !!this.entry.dynamic
  }
  authorizationServerInformation() {
    return this.entry.server
  }
  saveAuthorizationServerInformation(info: NonNullable<Entry["server"]>) {
    update(this.name, this.url, (e) => (e.server = info))
  }
  saveCodeVerifier(v: string) {
    update(this.name, this.url, (e) => (e.verifier = v))
  }
  codeVerifier() {
    const v = this.entry.verifier
    if (!v) throw new Error("no PKCE code verifier saved")
    return v
  }
  state() {
    return crypto.randomUUID()
  }
  saveState(state: string) {
    update(this.name, this.url, (e) => (e.state = state))
  }
  storedState() {
    return this.entry.state
  }
  invalidateCredentials(scope: "all" | "client" | "tokens" | "verifier") {
    update(this.name, this.url, (e) => {
      if (scope === "all" || scope === "tokens") delete e.tokens
      if (scope === "all" || scope === "client") {
        delete e.client
        delete e.dynamic
        delete e.server
      }
      if (scope === "all" || scope === "verifier") delete e.verifier
    })
  }
  redirectToAuthorization(url: URL) {
    this.wantsAuth = true
    if (!this.onAuthorize) throw new NeedsAuth()
    this.onAuthorize(url.toString())
  }
}

/** Browser sign-in for one server: resolves once tokens are saved. */
export async function signIn(name: string, url: string, o: { onUrl: (url: string) => void; signal: AbortSignal }): Promise<void> {
  let resolveCode!: (v: { code: string; state?: string }) => void
  let rejectCode!: (e: Error) => void
  const got = new Promise<{ code: string; state?: string }>((res, rej) => {
    resolveCode = res
    rejectCode = rej
  })
  let server: ReturnType<typeof Bun.serve>
  try {
    server = Bun.serve({
      hostname: "127.0.0.1",
      port: CALLBACK_PORT,
      fetch(req) {
        const u = new URL(req.url)
        if (u.pathname !== "/callback") return new Response("Not found", { status: 404 })
        const code = u.searchParams.get("code")
        const err = u.searchParams.get("error")
        if (code) resolveCode({ code, state: u.searchParams.get("state") ?? undefined })
        else rejectCode(new Error(u.searchParams.get("error_description") ?? err ?? "no authorization code"))
        return new Response(page(code ? `Signed in to ${name}. You can close this tab and return to oah.` : `Sign-in to ${name} failed: ${err ?? "no code"}`), { headers: { "content-type": "text/html; charset=utf-8" } })
      },
    })
  } catch (e: any) {
    throw new Error(`couldn't listen on 127.0.0.1:${CALLBACK_PORT} for the sign-in callback (${e?.message ?? e})`)
  }
  const onAbort = () => rejectCode(new Error("cancelled"))
  o.signal.addEventListener("abort", onAbort)
  try {
    const provider = new McpOAuth(name, url, o.onUrl)
    if ((await auth(provider, { serverUrl: url })) === "AUTHORIZED") return
    const { code, state } = await got
    const r = await auth(provider, { serverUrl: url, authorizationCode: code, callbackState: state })
    if (r !== "AUTHORIZED") throw new Error("the server didn't accept the sign-in")
  } finally {
    o.signal.removeEventListener("abort", onAbort)
    server.stop(true)
  }
}

const page = (msg: string) =>
  `<!doctype html><meta charset="utf-8"><title>oah</title><body style="font:16px system-ui;background:#111;color:#eee;display:grid;place-items:center;height:100vh;margin:0"><p>${msg.replace(/</g, "&lt;")}</p>`
