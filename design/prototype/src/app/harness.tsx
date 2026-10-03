import { cn } from '@/lib/utils'
import { harnesses, type HarnessId, type SessionStatus, type Voice } from '@/mock/data'

export const voiceVar: Record<Voice, string> = {
  claude: 'var(--voice-claude)',
  codex: 'var(--voice-codex)',
  acp: 'var(--voice-acp)',
  direct: 'var(--voice-direct)',
  human: 'var(--voice-human)',
}

/** Farbpunkt einer Stimme (Harness-Familie). */
export function VoiceDot({ voice, className }: { voice: Voice; className?: string }) {
  return <span aria-hidden className={cn('inline-block size-2 shrink-0 rounded-full', className)} style={{ background: voiceVar[voice] }} />
}

/** Harness-Name mit Stimmfarbe; optional mit Modell. */
export function HarnessBadge({ id, model, className }: { id: HarnessId; model?: string; className?: string }) {
  const h = harnesses[id]
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-xs', className)}>
      <VoiceDot voice={h.voice} />
      <span className="font-medium">{h.name}</span>
      {model && <span className="font-mono text-[11px] text-muted-foreground">{model}</span>}
    </span>
  )
}

const statusLabel: Record<SessionStatus, string> = {
  running: 'läuft',
  waiting: 'wartet auf dich',
  idle: 'bereit',
  failed: 'Fehler',
  stopped: 'gestoppt',
}

/** Status einer Session als Form + Farbe (nicht nur Farbe, für Barrierefreiheit). */
export function StatusMark({ status, withLabel }: { status: SessionStatus; withLabel?: boolean }) {
  return (
    <span className="inline-flex items-center gap-1.5" title={statusLabel[status]}>
      {status === 'running' && <span className="size-2 animate-pulse rounded-full bg-ok" />}
      {status === 'waiting' && <span className="chamfer-sm size-2.5 bg-signal" />}
      {status === 'idle' && <span className="size-2 rounded-full border border-muted-foreground" />}
      {status === 'failed' && <span className="size-2 rotate-45 bg-deny" />}
      {status === 'stopped' && <span className="size-2 rounded-[1px] bg-muted-foreground/50" />}
      {withLabel && <span className="text-xs text-muted-foreground">{statusLabel[status]}</span>}
      {!withLabel && <span className="sr-only">{statusLabel[status]}</span>}
    </span>
  )
}
