import { useEffect, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import type { Event, HarnessInfo } from '@beton/sdk'
import { ArrowUpRight, Check, ChevronDown, GitFork } from 'lucide-react'
import { client } from '@/lib/client'
import { payloadOf } from '@/lib/events'
import { useSessions } from '@/store/sessions'
import { HarnessBadge, harnessName, voiceOf, voiceVar } from './harness'

/** Herkunft eines Forks aus `session.forked` (SES-007, PROTO-002). */
export interface ForkOrigin {
  from: string
  harness: string
  fromHarness?: string
  atSeq: number
  /** Übernommene Turns (vor `session.forked` im Log). */
  turns: number
}

/** `session.forked` im Log, sonst `undefined`. */
export function forkOrigin(events: readonly Event[] | undefined): ForkOrigin | undefined {
  let turns = 0
  for (const e of events ?? []) {
    if (e.type === 'turn.started') turns++
    const f = payloadOf(e, 'session.forked')
    if (f) {
      return {
        from: f.from_session,
        harness: f.harness ?? '',
        ...(f.from_harness ? { fromHarness: f.from_harness } : {}),
        atSeq: f.at_seq,
        turns,
      }
    }
  }
  return undefined
}

/**
 * Banner „Fortgesetzt aus <Session> auf <Harness>“ mit Link zur Quelle (SES-007 AC4,
 * Screen `workspace-fork`, Zustand „Fork läuft“).
 */
export function ForkBanner({ origin }: { origin: ForkOrigin }) {
  const known = useSessions((s) => s.byId[origin.from]?.title)
  const fetched = useQuery({
    queryKey: ['session', origin.from],
    queryFn: () => client.sessions.get(origin.from),
    enabled: known === undefined,
    retry: false,
  })
  const title = known ?? fetched.data?.title ?? ''
  return (
    <div
      role="note"
      aria-label="Herkunft des Forks"
      className="flex shrink-0 items-center gap-2 border-b border-l-2 border-border bg-card px-3 py-2 text-[13px]"
      style={{ borderLeftColor: voiceVar[voiceOf(origin.harness)] }}
    >
      <GitFork className="size-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 truncate">
        Fortgesetzt aus <span className="font-medium">„{title || origin.from}“</span> auf {harnessName(origin.harness)}
        {origin.turns > 0 ? `, nach Turn ${origin.turns}` : ''}
      </span>
      <Link
        to="/s/$sessionId"
        params={{ sessionId: origin.from }}
        className="ml-auto inline-flex shrink-0 items-center gap-0.5 text-[12px] underline underline-offset-2"
      >
        Original öffnen <ArrowUpRight className="size-3" />
      </Link>
    </div>
  )
}

/** Harnesses, mit denen eine Session per Fork weiterlaufen kann (SES-007 AC5). */
export function continueTargets(catalog: HarnessInfo[], current: string, visible: (id: string) => boolean): HarnessInfo[] {
  return catalog.filter(
    (h) =>
      h.id !== current &&
      h.probe.installed &&
      !h.incompatible &&
      visible(h.id) &&
      (h.capabilities[0]?.fork_history ?? 'none') !== 'none',
  )
}

function authHint(h: HarnessInfo): string | undefined {
  switch (h.probe.auth_status) {
    case 'logged_in':
      return 'über die CLI angemeldet'
    case 'logged_out':
      return 'nicht angemeldet'
    default:
      return undefined
  }
}

/**
 * Harness-Picker im Composer mit „Weiter mit …“ (SES-007 AC5, Screen `workspace-fork`,
 * Zustand „Weiter mit Codex“): Ein anderer Harness legt einen Fork ab dem letzten `seq` an;
 * die Session selbst bleibt auf ihrem Harness.
 */
export function HarnessMenu({
  harness,
  targets,
  onContinue,
}: {
  harness: string
  targets: HarnessInfo[]
  onContinue: (harness: string) => void
}) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const outside = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false)
    }
    const esc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        setOpen(false)
      }
    }
    document.addEventListener('mousedown', outside)
    document.addEventListener('keydown', esc)
    return () => {
      document.removeEventListener('mousedown', outside)
      document.removeEventListener('keydown', esc)
    }
  }, [open])
  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label="Harness"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="flex h-7 items-center gap-1 rounded-md px-1.5 hover:bg-accent"
      >
        <HarnessBadge harness={harness} />
        <ChevronDown className="size-3 text-muted-foreground" />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Harness wählen"
          className="absolute bottom-full left-0 z-30 mb-1 w-[min(440px,calc(100vw-2rem))] overflow-hidden rounded-lg border border-border bg-popover py-1 text-popover-foreground shadow-[0_16px_40px_-20px_rgb(0_0_0/0.5)]"
        >
          <div className="px-3 pt-2 pb-1 text-[11px] font-medium text-muted-foreground">Harness dieser Session</div>
          <div className="mx-1 flex items-start gap-2 rounded-md px-2 py-1.5 text-[13px]">
            <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center text-muted-foreground">
              <Check className="size-3.5" />
            </span>
            <HarnessBadge harness={harness} />
          </div>
          {targets.length > 0 && (
            <>
              <div className="my-1 h-px bg-border" />
              <div className="px-3 pt-2 pb-1 text-[11px] font-medium text-muted-foreground">
                Weiter mit … (neue Session ab hier, diese bleibt bei {harnessName(harness)})
              </div>
              {targets.map((h) => {
                const hint = authHint(h)
                return (
                  <button
                    key={h.id}
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setOpen(false)
                      onContinue(h.id)
                    }}
                    className="mx-1 flex w-[calc(100%-0.5rem)] items-start gap-2 rounded-md px-2 py-1.5 text-left text-[13px] outline-none hover:bg-accent focus-visible:bg-accent"
                  >
                    <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center text-muted-foreground">
                      <GitFork className="size-3.5" />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate">Weiter mit {harnessName(h.id)}</span>
                      {hint && <span className="block text-[11px] text-muted-foreground">{hint}</span>}
                    </span>
                    <span className="shrink-0 text-[11px] text-muted-foreground">Übergabe</span>
                  </button>
                )
              })}
            </>
          )}
        </div>
      )}
    </div>
  )
}
