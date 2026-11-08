import { useEffect, useState } from "react"
import { REGEXP_ONLY_DIGITS_AND_CHARS } from "input-otp"
import { ChevronLeft, Loader2 } from "lucide-react"
import { Mark } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { InputOTP, InputOTPGroup, InputOTPSlot } from "@/components/ui/input-otp"
import { native, phoneName, type NearbyDesktop } from "./rpc"
import { back, go } from "./route"
import { useMobile } from "./store"

export function PairScreen() {
  const hasDesktops = useMobile((s) => s.desktops.length > 0)
  const [address, setAddress] = useState("")
  const [code, setCode] = useState("")
  const [name, setName] = useState(phoneName)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const [nearby, setNearby] = useState<NearbyDesktop[]>([])
  const [scanning, setScanning] = useState(false)
  const [scanError, setScanError] = useState<string | null>(null)
  const [selected, setSelected] = useState<NearbyDesktop | null>(null)
  const [requesting, setRequesting] = useState(false)
  const scan = async () => {
    setScanning(true)
    setScanError(null)
    try { setNearby(await native.discover()) }
    catch (e) { setScanError(String(e)) }
    finally { setScanning(false) }
  }
  useEffect(() => { void scan() }, [])
  const invite = async (desktop: NearbyDesktop) => {
    setAddress(desktop.address)
    setCode("")
    setError(null)
    setRequesting(true)
    try {
      await native.requestPairing(desktop.address, name)
      setSelected(desktop)
    } catch (e) { setError(String(e)) }
    finally { setRequesting(false) }
  }

  const done = async (id: string) => {
    await useMobile.getState().connect(id)
    go("/", true)
  }
  const pair = async () => {
    setError(null)
    setBusy(true)
    try {
      const d = await useMobile.getState().pair({ addresses: address.split(/[\s,]+/), code, name })
      await done(d.id)
    } catch (e) {
      setError(String(e).replace(/^Error: /, ""))
    } finally {
      setBusy(false)
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
            On the computer, open oah, then <span className="text-foreground">Settings → Phone → Pair a phone</span>, or choose a nearby computer below.
          </p>
        </div>

        <div className="mb-6 rounded-md border p-3">
          <div className="mb-2 flex items-center justify-between">
            <span className="text-[13px] font-medium">Nearby computers</span>
            <Button size="sm" variant="ghost" disabled={scanning || requesting || busy} onClick={() => void scan()}>{scanning ? "Searching…" : "Refresh"}</Button>
          </div>
          {nearby.map((d) => <div key={d.address} className="flex items-center justify-between gap-2 py-2">
            <div className="min-w-0"><div className="truncate text-[13px]">{d.name}</div><div className="font-mono text-[11px] text-muted-foreground">{d.address}</div></div>
            <Button size="sm" disabled={requesting || busy} onClick={() => void invite(d)}>Connect</Button>
          </div>)}
          {!scanning && !nearby.length ? <p className="text-[12px] text-muted-foreground">{scanError ?? "No computers found. Turn on phone access in oah on the desktop. You can also enter its address below."}</p> : null}
          <p className="mt-2 text-[11px] text-faint">Ethernet and Wi-Fi can work together. Guest Wi-Fi or a firewall may block discovery.</p>
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
        <InputOTP
          maxLength={8}
          pattern={REGEXP_ONLY_DIGITS_AND_CHARS}
          // Codes have letters: a text keyboard (input-otp defaults to a number pad), capitals,
          // no autocorrect or suggestions.
          inputMode="text"
          autoCapitalize="characters"
          autoCorrect="off"
          autoComplete="off"
          spellCheck={false}
          value={code} onChange={(v) => setCode(v.toUpperCase())} containerClassName="w-full gap-1.5" aria-label="Pairing code">
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

        <Button size="lg" className="mt-6 h-12 w-full text-[14px] disabled:bg-muted disabled:text-muted-foreground disabled:opacity-100" disabled={!canPair} onClick={() => void pair()}>
          {busy ? <Loader2 className="animate-spin" /> : null}
          {canPair || busy ? "Pair" : !address.trim() ? "Type the address and code" : "Type all 8 characters"}
        </Button>

        <p className="mt-auto pt-8 text-center text-[11.5px] leading-relaxed text-faint">
          The phone connects to the computer directly: on the same Wi-Fi, or a VPN like Tailscale. Everything between them is end-to-end encrypted.
        </p>
      </div>
      <Dialog open={!!selected} onOpenChange={(o) => { if (!o && !busy) setSelected(null) }}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>Connect to {selected?.name}</DialogTitle>
            <DialogDescription>Click Connect in the desktop prompt, then enter the code it shows here.</DialogDescription>
          </DialogHeader>
          <Input aria-label="Desktop pairing code" placeholder="XXXX-XXXX" autoCapitalize="characters" autoCorrect="off" value={code} onChange={(e) => setCode(e.target.value.replace(/[^a-z0-9]/gi, "").toUpperCase().slice(0, 8))} />
          {error ? <p className="text-[12px] text-red">{error}</p> : null}
          <Button disabled={!canPair} onClick={() => void pair()}>{busy ? "Connecting…" : "Connect"}</Button>
        </DialogContent>
      </Dialog>
    </div>
  )
}
