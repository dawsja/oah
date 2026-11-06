// Credentials for the three providers.
//
// ChatGPT plan usage follows OpenAI's open-source "Sign in with ChatGPT" flow
// (developers.openai.com/siwc/token-sharing-open-source):
//  * a stable opaque `ext_agent_host_id` (urn:uuid) is created once per host
//  * first sign-in registers a user-defined agent (`client_id=dynamic_agent_client`,
//    `agent_name_hint`), and the issued client id is kept for every later sign-in
//  * PKCE S256, fresh state + nonce, loopback redirect on 127.0.0.1 (1455 to register)
//  * the ID token is verified against OpenAI's JWKS (iss, aud, exp, nonce) and the
//    granted scopes must include `chatgpt.tokens.use.direct`
//  * hourly access tokens refresh automatically; sign-out revokes the refresh token
import { randomBytes, createHash, randomUUID } from "node:crypto"
import { readFileSync } from "node:fs"
import { paths, readJson, writeFileAtomic, type Provider } from "../config"

export const ISSUER = "https://auth.openai.com"
const AUTHORIZE_URL = "https://auth.openai.com/api/accounts/authorize"
const TOKEN_URL = "https://auth.openai.com/api/accounts/oauth/token"
const DISCOVERY_URL = "https://auth.openai.com/.well-known/openid-configuration"
const RESOURCE = "https://api.openai.com/v1"
const REGISTRATION_CLIENT_ID = "dynamic_agent_client"
const PLAN_SCOPE = "chatgpt.tokens.use.direct"
const SCOPES = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"
const REGISTRATION_PORT = 1455
export const USAGE_URL = "https://chatgpt.com/settings/usage"

export interface ChatGptCredentials {
  email?: string
  issuer: string
  subject: string
  client_id: string
  ext_agent_host_id: string
  id_token: string
  access_token: string
  refresh_token?: string
  token_type: string
  expires_in: number
  scopes: string[]
  saved_at: number
}

interface AuthStore {
  chatgpt?: ChatGptCredentials
  openaiKey?: string
  anthropicKey?: string
  /** Issued client id, kept across sign-out for reauthorization. */
  clientId?: string
  /** Last ID token, used as `id_token_hint` to skip account selection. */
  lastIdToken?: string
}

export interface AuthStatus {
  chatgpt?: { email?: string; planUsage: boolean }
  openai?: string
  anthropic?: string
}

let store: AuthStore = readJson<AuthStore>(paths.auth, {})
const save = () => writeFileAtomic(paths.auth, JSON.stringify(store, null, 2), true)
const now = () => Math.floor(Date.now() / 1000)
const b64url = (b: Buffer) => b.toString("base64url")

export function status(): AuthStatus {
  const env = (k: string) => (process.env[k] ? k : undefined)
  return {
    chatgpt: store.chatgpt ? { email: store.chatgpt.email, planUsage: store.chatgpt.scopes.includes(PLAN_SCOPE) } : undefined,
    openai: store.openaiKey ? "saved key" : env("OPENAI_API_KEY"),
    anthropic: store.anthropicKey ? "saved key" : env("ANTHROPIC_API_KEY"),
  }
}

export function has(s: AuthStatus, p: Provider): boolean {
  if (p === "chatgpt") return !!s.chatgpt?.planUsage
  return !!s[p]
}

export function available(s = status()): Provider[] {
  return (["chatgpt", "anthropic", "openai"] as Provider[]).filter((p) => has(s, p))
}

export function label(s: AuthStatus, p: Provider): string {
  if (p === "chatgpt") return s.chatgpt?.planUsage ? "Using ChatGPT plan" : s.chatgpt ? "ChatGPT plan usage off · /auth" : "not signed in · /auth"
  if (has(s, p)) return p === "openai" ? "OpenAI API" : "Anthropic API"
  return `no ${p === "openai" ? "OpenAI" : "Anthropic"} key · /auth`
}

export function setKey(p: "openai" | "anthropic", key: string | undefined) {
  if (p === "openai") store.openaiKey = key?.trim() || undefined
  else store.anthropicKey = key?.trim() || undefined
  save()
}

/** API key or (refreshed) ChatGPT access token. */
export async function credential(p: Provider): Promise<string> {
  if (p === "openai") {
    const k = store.openaiKey ?? process.env.OPENAI_API_KEY
    if (!k) throw new Error("No OpenAI API key — run /auth")
    return k
  }
  if (p === "anthropic") {
    const k = store.anthropicKey ?? process.env.ANTHROPIC_API_KEY
    if (!k) throw new Error("No Anthropic API key — run /auth")
    return k
  }
  const c = store.chatgpt
  if (!c) throw new Error("Not signed in to ChatGPT — run /auth")
  if (!c.scopes.includes(PLAN_SCOPE)) throw new Error("ChatGPT plan usage wasn't granted — run /auth to enable it")
  if (c.saved_at + c.expires_in - now() > 120) return c.access_token
  return refresh()
}

let refreshing: Promise<string> | null = null

/** Serialized refresh of the ChatGPT access token. */
export function refresh(): Promise<string> {
  refreshing ??= doRefresh().finally(() => (refreshing = null))
  return refreshing
}

async function doRefresh(): Promise<string> {
  const c = store.chatgpt
  if (!c?.refresh_token) throw new Error("ChatGPT session expired — run /auth")
  const r = await fetch(TOKEN_URL, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ grant_type: "refresh_token", client_id: c.client_id, refresh_token: c.refresh_token, resource: RESOURCE }),
  })
  const body = (await r.json().catch(() => ({}))) as Record<string, any>
  if (!r.ok) {
    const code = String(body.error ?? "")
    const terminal = ["invalid_grant", "invalid_refresh_token", "token_expired", "refresh_token_expired", "refresh_token_invalidated", "refresh_token_reused"]
    if (terminal.includes(code)) {
      store.clientId = c.client_id
      store.chatgpt = undefined
      save()
      throw new Error(`ChatGPT session ended (${code}) — run /auth to sign in again`)
    }
    if (code === "invalid_client") throw new Error("ChatGPT client registration is invalid — run /auth logout, then /auth")
    throw new Error(`ChatGPT token refresh failed (${r.status}): ${body.error_description ?? code}`)
  }
  c.access_token = body.access_token
  if (body.refresh_token) c.refresh_token = body.refresh_token
  if (body.id_token) {
    c.id_token = body.id_token
    store.lastIdToken = body.id_token
  }
  c.expires_in = body.expires_in ?? 3600
  if (body.scope) c.scopes = String(body.scope).split(/\s+/)
  c.saved_at = now()
  save()
  return c.access_token
}

export function hostId(): string {
  try {
    const s = readFileSync(paths.hostId, "utf8").trim()
    if (/^(urn:uuid:|urn:ietf:params:oauth:jwk-thumbprint:|did:key:)/.test(s)) return s
  } catch {}
  const id = `urn:uuid:${randomUUID()}`
  writeFileAtomic(paths.hostId, id, true)
  return id
}

export function redact(url: string): string {
  const u = new URL(url)
  u.searchParams.delete("id_token_hint")
  return u.toString()
}

export interface PendingLogin {
  url: string
  displayUrl: string
  done: Promise<AuthStatus>
  cancel: () => void
}

/** Start a browser sign-in; resolves when the loopback callback completes. */
export async function beginChatGptLogin(): Promise<PendingLogin> {
  const clientId = store.chatgpt?.client_id ?? store.clientId
  const registering = !clientId
  const host = hostId()
  const state = b64url(randomBytes(32))
  const nonce = b64url(randomBytes(32))
  const verifier = b64url(randomBytes(64))
  const challenge = b64url(createHash("sha256").update(verifier).digest())

  let resolveParams!: (p: URLSearchParams) => void
  let rejectParams!: (e: Error) => void
  const params = new Promise<URLSearchParams>((res, rej) => ((resolveParams = res), (rejectParams = rej)))

  const handler = (req: Request) => {
    const u = new URL(req.url)
    if (u.pathname !== "/auth/callback") return new Response("Not found", { status: 404 })
    const ok = u.searchParams.get("state") === state
    const [title, msg] = !ok
      ? ["Sign-in failed", "The response didn't match this request. Return to nur and try again."]
      : u.searchParams.get("error")
        ? ["Sign-in cancelled", "No changes were made. You can close this tab."]
        : ["Signed in to nur", "You can close this tab and return to your terminal."]
    queueMicrotask(() => (ok ? resolveParams(u.searchParams) : rejectParams(new Error("state mismatch in OAuth callback"))))
    return new Response(
      `<!doctype html><meta charset=utf-8><title>${title}</title><body style="font-family:system-ui;background:#0f1117;color:#e6e8ef;display:grid;place-items:center;height:100vh;margin:0"><div style="text-align:center"><h1 style="font-weight:600">${title}</h1><p style="color:#8a90a6">${msg}</p></div>`,
      { headers: { "Content-Type": "text/html; charset=utf-8" } },
    )
  }
  let server: ReturnType<typeof Bun.serve>
  try {
    server = Bun.serve({ hostname: "127.0.0.1", port: REGISTRATION_PORT, fetch: handler })
  } catch (e) {
    if (registering) throw new Error(`Port ${REGISTRATION_PORT} is busy; first-time ChatGPT registration needs it (${e})`)
    server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: handler })
  }
  const redirectUri = `http://127.0.0.1:${server.port}/auth/callback`
  const url = new URL(AUTHORIZE_URL)
  const q = url.searchParams
  q.set("response_type", "code")
  q.set("client_id", clientId ?? REGISTRATION_CLIENT_ID)
  if (registering) q.set("agent_name_hint", "nur")
  q.set("ext_agent_host_id", host)
  q.set("redirect_uri", redirectUri)
  q.set("scope", SCOPES)
  q.set("resource", RESOURCE)
  q.set("state", state)
  q.set("nonce", nonce)
  q.set("code_challenge", challenge)
  q.set("code_challenge_method", "S256")
  if (store.lastIdToken) q.set("id_token_hint", store.lastIdToken)

  const timer = setTimeout(() => rejectParams(new Error("sign-in timed out")), 10 * 60 * 1000)
  const done = (async () => {
    try {
      const p = await params
      const err = p.get("error")
      if (err === "access_denied") throw new Error("sign-in was cancelled or consent was declined")
      if (err) throw new Error(`sign-in failed: ${err} ${p.get("error_description") ?? ""}`)
      const code = p.get("code")
      if (!code) throw new Error("callback is missing the authorization code")
      const issued = p.get("client_id")
      let cid: string
      if (clientId) {
        if (issued && issued !== clientId) throw new Error("callback returned an unexpected client_id")
        cid = clientId
      } else {
        if (!issued || issued === REGISTRATION_CLIENT_ID) throw new Error("registration incomplete: no issued client_id")
        cid = issued
      }
      return await finishLogin(cid, code, verifier, redirectUri, nonce, host)
    } finally {
      clearTimeout(timer)
      server.stop(true)
    }
  })()
  return { url: url.toString(), displayUrl: redact(url.toString()), done, cancel: () => rejectParams(new Error("cancelled")) }
}

async function finishLogin(clientId: string, code: string, verifier: string, redirectUri: string, nonce: string, host: string): Promise<AuthStatus> {
  const r = await fetch(TOKEN_URL, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ grant_type: "authorization_code", client_id: clientId, code, code_verifier: verifier, redirect_uri: redirectUri, resource: RESOURCE }),
  })
  const tok = (await r.json().catch(() => ({}))) as Record<string, any>
  if (!r.ok) {
    store.clientId = clientId // reauthorize next time instead of re-registering
    save()
    throw new Error(`token exchange failed (${r.status}): ${tok.error_description ?? tok.error ?? ""}`)
  }
  if (!tok.id_token) throw new Error("token response has no id_token")
  const claims = await verifyIdToken(tok.id_token, clientId, nonce)
  store.chatgpt = {
    email: claims.email,
    issuer: ISSUER,
    subject: claims.sub,
    client_id: clientId,
    ext_agent_host_id: host,
    id_token: tok.id_token,
    access_token: tok.access_token,
    refresh_token: tok.refresh_token,
    token_type: tok.token_type ?? "Bearer",
    expires_in: tok.expires_in ?? 3600,
    scopes: String(tok.scope ?? "").split(/\s+/).filter(Boolean),
    saved_at: now(),
  }
  store.clientId = clientId
  store.lastIdToken = tok.id_token
  save()
  return status()
}

async function verifyIdToken(token: string, clientId: string, nonce: string): Promise<Record<string, any>> {
  const [h, p, s] = token.split(".")
  if (!h || !p || !s) throw new Error("malformed id_token")
  const header = JSON.parse(Buffer.from(h, "base64url").toString())
  const claims = JSON.parse(Buffer.from(p, "base64url").toString())
  const disco = (await (await fetch(DISCOVERY_URL)).json()) as { jwks_uri?: string }
  const jwks = (await (await fetch(disco.jwks_uri ?? `${ISSUER}/.well-known/jwks.json`)).json()) as { keys: any[] }
  const jwk = jwks.keys.find((k) => k.kid === header.kid) ?? jwks.keys[0]
  if (!jwk) throw new Error("no signing key for id_token")
  const alg = header.alg as string
  const params =
    alg.startsWith("ES") ? { name: "ECDSA", namedCurve: jwk.crv, hash: "SHA-" + alg.slice(2) } : alg.startsWith("PS") ? { name: "RSA-PSS", hash: "SHA-" + alg.slice(2) } : { name: "RSASSA-PKCS1-v1_5", hash: "SHA-" + alg.slice(2) }
  const key = await crypto.subtle.importKey("jwk", jwk, params as any, false, ["verify"])
  const verifyParams = alg.startsWith("ES") ? { name: "ECDSA", hash: "SHA-" + alg.slice(2) } : alg.startsWith("PS") ? { name: "RSA-PSS", saltLength: Number(alg.slice(2)) / 8 } : { name: "RSASSA-PKCS1-v1_5" }
  const ok = await crypto.subtle.verify(verifyParams as any, key, Buffer.from(s, "base64url"), new TextEncoder().encode(`${h}.${p}`))
  if (!ok) throw new Error("id_token signature is invalid")
  const aud = Array.isArray(claims.aud) ? claims.aud : [claims.aud]
  if (claims.iss !== ISSUER) throw new Error("id_token issuer mismatch")
  if (!aud.includes(clientId)) throw new Error("id_token audience mismatch")
  if (typeof claims.exp === "number" && claims.exp + 60 < now()) throw new Error("id_token expired")
  if (claims.nonce !== nonce) throw new Error("id_token nonce mismatch")
  return claims
}

/** Revoke (best effort) and remove ChatGPT credentials; keeps the issued client id. */
export async function logoutChatGpt(): Promise<string> {
  const c = store.chatgpt
  if (!c) return "Not signed in to ChatGPT."
  let note = "Signed out locally. Revocation couldn't be confirmed — disconnect nur in ChatGPT settings if needed."
  try {
    const disco = (await (await fetch(DISCOVERY_URL)).json()) as { revocation_endpoint?: string }
    if (c.refresh_token && disco.revocation_endpoint) {
      for (let i = 0; i < 3; i++) {
        const r = await fetch(disco.revocation_endpoint, {
          method: "POST",
          headers: { "Content-Type": "application/x-www-form-urlencoded" },
          body: new URLSearchParams({ token: c.refresh_token, token_type_hint: "refresh_token", client_id: c.client_id }),
        })
        if (r.ok) {
          note = "Signed out and revoked the ChatGPT session."
          break
        }
        if (r.status < 500) break
        await Bun.sleep(500 * 2 ** i)
      }
    }
  } catch {}
  store.clientId = c.client_id
  store.chatgpt = undefined
  store.lastIdToken = undefined
  save()
  return note
}

export function openBrowser(url: string): boolean {
  const cmds = process.platform === "darwin" ? [["open"]] : [["xdg-open"], ["wslview"], ["sensible-browser"]]
  for (const [c] of cmds) {
    try {
      const p = Bun.spawn([c!, url], { stdout: "ignore", stderr: "ignore", stdin: "ignore" })
      p.unref()
      return true
    } catch {}
  }
  return false
}
