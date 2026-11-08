import { useState } from "react"
import { REGEXP_ONLY_DIGITS_AND_CHARS } from "input-otp"
import { ChevronLeft, Loader2, QrCode } from "lucide-react"
import { Mark } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { InputOTP, InputOTPGroup, InputOTPSlot } from "@/components/ui/input-otp"
import { phoneName } from "./rpc"
import { back, go } from "./route"
import { useMobile } from "./store"

/** Scans with the camera (Android only); null when cancelled or unavailable. */
async function scanQr(): Promise<string | null> {
  const s = await import("@tauri-apps/plugin-barcode-scanner")
  let perm = await s.checkPermissions()
  if (perm !== "granted") perm = await s.requestPermissions()
  if (perm !== "granted") throw new Error("oah needs the camera to scan the code. Allow it in Android's settings, or type the code instead.")
  const r = await s.scan({ windowed: false, formats: [s.Format.QRCode] })
  return r?.content ?? null
}

export function PairScreen() {
  const hasDesktops = useMobile((s) => s.desktops.length > 0)
  const [address, setAddress] = useState("")
  const [code, setCode] = useState("")
  const [name, setName] = useState(phoneName)
  const [busy, setBusy] = useState<"scan" | "code" | null>(null)
  const [error, setError] = useState<string | null>(null)

  const done = async (id: string) => {
    await useMobile.getState().connect(id)
    go("/", true)
  }
  const pairWith = async (how: "scan" | "code") => {
    setError(null)
    setBusy(how)
    try {
      let link: string | undefined
      if (how === "scan") {
        link = (await scanQr()) ?? undefined
        if (!link) return
      }
      const d = await useMobile.getState().pair(link ? { link, name } : { addresses: address.split(/[\s,]+/), code, name })
      await done(d.id)
    } catch (e) {
      setError(String(e).replace(/^Error: /, ""))
    } finally {
      setBusy(null)
    }
  }
  const canPair = address.trim().length > 0 && code.length === 8 && !busy

  return (
    <div className="safe flex h-full flex-col overflow-y-auto bg-background">
      {hasDesktops ? (
        <div className="flex h-12 shrink-0 items-center px-2">
          <button className="rounded-md grid size-10 place-items-center text-muted-foreground" aria-label="Back" onClick={back}>
            <ChevronLeft className="size-5" />
          </button>
        </div>
      ) : (
        <div className="h-12 shrink-0" />
      )}
      <div className="mx-auto flex w-full max-w-sm flex-1 flex-col px-6 pb-8">
        <div className="mb-8 flex flex-col items-center text-center">
          <Mark size={44} label="oah" />
          <h1 className="mt-4 text-[20px] font-semibold tracking-tight">Pair with your computer</h1>
          <p className="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">
            On the computer, open oah, then <span className="text-foreground">Settings → Phone → Pair a phone</span>.
          </p>
        </div>

        <Button size="lg" className="h-12 w-full text-[14px]" disabled={!!busy} onClick={() => void pairWith("scan")}>
          {busy === "scan" ? <Loader2 className="animate-spin" /> : <QrCode />}
          Scan the QR code
        </Button>

        <div className="my-7 flex items-center gap-3 text-[11px] font-medium tracking-wide text-faint uppercase">
          <span className="h-px flex-1 bg-border" />
          or type the code
          <span className="h-px flex-1 bg-border" />
        </div>

        <label className="mb-1.5 text-[12px] font-medium text-muted-foreground" htmlFor="addr">
          Address
        </label>
        <Input
          id="addr"
          inputMode="url"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          placeholder="192.168.1.20"
          className="h-11 font-mono text-[14px]"
          value={address}
          onChange={(e) => setAddress(e.target.value)}
        />
        <p className="mt-1 text-[11.5px] text-faint">As the desktop shows it. The port is optional.</p>

        <label className="mt-5 mb-1.5 text-[12px] font-medium text-muted-foreground">Code</label>
        {/* Eight boxes share the width (about 34px each on a 360px phone), a dash between the halves. */}
        <InputOTP maxLength={8} pattern={REGEXP_ONLY_DIGITS_AND_CHARS} value={code} onChange={(v) => setCode(v.toUpperCase())} containerClassName="w-full gap-1.5" aria-label="Pairing code">
          <InputOTPGroup className="min-w-0 flex-1">
            {[0, 1, 2, 3].map((i) => (
              <InputOTPSlot key={i} index={i} className="h-12 w-auto min-w-0 flex-1 font-mono text-[17px]" />
            ))}
          </InputOTPGroup>
          <span className="w-2.5 shrink-0 border-t-2 border-faint" aria-hidden />
          <InputOTPGroup className="min-w-0 flex-1">
            {[4, 5, 6, 7].map((i) => (
              <InputOTPSlot key={i} index={i} className="h-12 w-auto min-w-0 flex-1 font-mono text-[17px]" />
            ))}
          </InputOTPGroup>
        </InputOTP>

        <label className="mt-5 mb-1.5 text-[12px] font-medium text-muted-foreground" htmlFor="name">
          This phone's name
        </label>
        <Input id="name" className="h-11 text-[14px]" value={name} onChange={(e) => setName(e.target.value)} />

        {error ? <div className="mt-5 rounded-md border border-red/40 bg-red/5 px-3 py-2.5 text-[13px] leading-relaxed text-red">{error}</div> : null}

        <Button size="lg" variant="outline" className="mt-6 h-12 w-full text-[14px] disabled:border-dashed disabled:opacity-60" disabled={!canPair} onClick={() => void pairWith("code")}>
          {busy === "code" ? <Loader2 className="animate-spin" /> : null}
          {canPair || busy ? "Pair" : !address.trim() ? "Type the address and code" : "Type all 8 characters"}
        </Button>

        <p className="mt-auto pt-8 text-center text-[11.5px] leading-relaxed text-faint">
          The phone connects to the computer directly: on the same Wi-Fi, or a VPN like Tailscale. Everything between them is end-to-end encrypted.
        </p>
      </div>
    </div>
  )
}
