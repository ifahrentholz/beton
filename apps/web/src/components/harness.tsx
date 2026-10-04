import { cn } from '@/lib/utils'

export type Voice = 'claude' | 'codex' | 'acp' | 'direct' | 'human'

export const voiceVar: Record<Voice, string> = {
  claude: 'var(--voice-claude)',
  codex: 'var(--voice-codex)',
  acp: 'var(--voice-acp)',
  direct: 'var(--voice-direct)',
  human: 'var(--voice-human)',
}

const NAMES: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', fake: 'Fake-Harness' }

/** Stimmfarbe einer Harness-Familie (Design: überall gleich). */
export function voiceOf(harness: string): Voice {
  if (harness === 'claude') return 'claude'
  if (harness === 'codex') return 'codex'
  if (harness.startsWith('acp:')) return 'acp'
  return 'direct'
}

export function harnessName(harness: string): string {
  return NAMES[harness] ?? harness
}

export function VoiceDot({ voice, className }: { voice: Voice; className?: string }) {
  return <span aria-hidden className={cn('inline-block size-2 shrink-0 rounded-full', className)} style={{ background: voiceVar[voice] }} />
}

export function HarnessBadge({ harness, model, className }: { harness: string; model?: string | null; className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-xs', className)}>
      <VoiceDot voice={voiceOf(harness)} />
      <span className="font-medium">{harnessName(harness)}</span>
      {model && <span className="font-mono text-[11px] text-muted-foreground">{model}</span>}
    </span>
  )
}

/** Session-Status der Liste und Kopfzeile (aus `session.status`). */
export type ListStatus = 'running' | 'waiting' | 'idle' | 'failed' | 'stopped'

export function listStatus(status: string): ListStatus {
  switch (status) {
    case 'running':
    case 'starting':
      return 'running'
    case 'waiting_approval':
      return 'waiting'
    case 'failed':
      return 'failed'
    case 'stopped':
    case 'paused':
      return 'stopped'
    default:
      return 'idle'
  }
}

const statusLabel: Record<ListStatus, string> = {
  running: 'läuft',
  waiting: 'wartet auf dich',
  idle: 'bereit',
  failed: 'Fehler',
  stopped: 'gestoppt',
}

/** Status als Form + Farbe + Text (nicht nur Farbe, für Barrierefreiheit). */
export function StatusMark({ status, withLabel }: { status: ListStatus; withLabel?: boolean }) {
  return (
    <span className="inline-flex shrink-0 items-center gap-1.5" title={statusLabel[status]} data-status={status}>
      {status === 'running' && <span className="size-2 animate-pulse rounded-full bg-ok" />}
      {status === 'waiting' && <span className="chamfer-sm size-2.5 bg-signal" />}
      {status === 'idle' && <span className="size-2 rounded-full border border-muted-foreground" />}
      {status === 'failed' && <span className="size-2 rotate-45 bg-deny" />}
      {status === 'stopped' && <span className="size-2 rounded-[1px] bg-muted-foreground/50" />}
      {withLabel ? <span className="text-xs text-muted-foreground">{statusLabel[status]}</span> : <span className="sr-only">{statusLabel[status]}</span>}
    </span>
  )
}
