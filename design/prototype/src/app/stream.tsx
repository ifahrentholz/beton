import { useState, type ReactNode } from 'react'
import { Check, ChevronRight, FileText, GitBranch, Lock, Search, ShieldAlert, SquareTerminal, X } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { DiffLine, HarnessId } from '@/mock/data'
import { harnesses } from '@/mock/data'
import { VoiceDot, voiceVar } from './harness'

/** Nachricht eines Menschen. Bei geteilten Sessions mit Autor (Co-Drive). */
export function UserMessage({ author = 'Ingo', children, queued }: { author?: string; children: ReactNode; queued?: boolean }) {
  return (
    <div className={cn('flex gap-3', queued && 'dimmed')}>
      <span className="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full bg-foreground text-[10px] font-semibold text-background">
        {author.slice(0, 2).toUpperCase()}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-xs text-muted-foreground">
          {author}
          {queued && ' · eingereiht'}
        </div>
        <div className="mt-0.5 text-[14px] leading-relaxed">{children}</div>
      </div>
    </div>
  )
}

/** Antwort des Agents mit Stimmfarbe des Harness. */
export function AgentMessage({ harness, children, streaming }: { harness: HarnessId; children: ReactNode; streaming?: boolean }) {
  const h = harnesses[harness]
  return (
    <div className="flex gap-3">
      <span className="mt-1 flex size-6 shrink-0 items-center justify-center">
        <span className="size-3 rounded-[3px]" style={{ background: voiceVar[h.voice] }} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-xs text-muted-foreground">{h.name}</div>
        <div className="mt-0.5 space-y-2 text-[14px] leading-relaxed">
          {children}
          {streaming && <span className="ml-0.5 inline-block h-4 w-1.5 animate-pulse bg-foreground align-text-bottom" />}
        </div>
      </div>
    </div>
  )
}

/** Gedankengang des Modells, standardmäßig eingeklappt. */
export function Reasoning({ summary, children }: { summary: string; children?: ReactNode }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="ml-9 text-[13px] text-muted-foreground">
      <button onClick={() => setOpen((v) => !v)} className="inline-flex items-center gap-1 hover:text-foreground" aria-expanded={open}>
        <ChevronRight className={cn('size-3.5 transition-transform', open && 'rotate-90')} />
        Überlegung: {summary}
      </button>
      {open && <div className="mt-1 border-l border-border pl-3 italic">{children}</div>}
    </div>
  )
}

export type ToolKind = 'read' | 'search' | 'edit' | 'shell' | 'git' | 'other'
const toolIcon: Record<ToolKind, typeof FileText> = {
  read: FileText,
  search: Search,
  edit: FileText,
  shell: SquareTerminal,
  git: GitBranch,
  other: ChevronRight,
}

export type ToolStatus = 'running' | 'ok' | 'failed' | 'denied' | 'waiting'

/** Ein Tool-Call als kompakte Zeile; Ausgabe aufklappbar. */
export function ToolCall({
  kind,
  name,
  target,
  status = 'ok',
  duration,
  sandbox = true,
  policy,
  children,
  defaultOpen,
}: {
  kind: ToolKind
  name: string
  target: string
  status?: ToolStatus
  duration?: string
  sandbox?: boolean
  policy?: string
  children?: ReactNode
  defaultOpen?: boolean
}) {
  const [open, setOpen] = useState(!!defaultOpen)
  const Icon = toolIcon[kind]
  return (
    <div className="ml-9">
      <button
        onClick={() => children && setOpen((v) => !v)}
        aria-expanded={children ? open : undefined}
        className={cn(
          'flex w-full items-center gap-2 rounded-md border border-transparent px-2 py-1 text-left text-[13px] hover:border-border hover:bg-card',
          status === 'waiting' && 'border-signal bg-signal-soft',
        )}
      >
        <Icon className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="font-medium">{name}</span>
        <span className="min-w-0 truncate font-mono text-[12px] text-muted-foreground">{target}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2 text-[11px] text-muted-foreground">
          {policy && <span className="rounded-sm border border-border px-1">{policy}</span>}
          {sandbox && (
            <span title="In der Sandbox ausgeführt" className="inline-flex items-center gap-0.5">
              <Lock className="size-3" />
            </span>
          )}
          {duration && <span className="tabular-nums">{duration}</span>}
          {status === 'running' && <span className="size-1.5 animate-pulse rounded-full bg-ok" />}
          {status === 'ok' && <Check className="size-3.5 text-ok" />}
          {status === 'failed' && <X className="size-3.5 text-deny" />}
          {status === 'denied' && <ShieldAlert className="size-3.5 text-deny" />}
          {status === 'waiting' && <span className="chamfer-sm size-2.5 bg-signal" />}
        </span>
      </button>
      {open && children && <div className="mt-1 mb-2 ml-6">{children}</div>}
    </div>
  )
}

export function Diff({ file, lines }: { file: string; lines: DiffLine[] }) {
  return (
    <div className="overflow-hidden rounded-md border border-border bg-card font-mono text-[12px]">
      <div className="border-b border-border bg-sunken px-3 py-1 text-[11px] text-muted-foreground">{file}</div>
      {lines.map((l, i) => (
        <div
          key={i}
          className={cn(
            'flex',
            l.kind === 'add' && 'bg-ok-soft',
            l.kind === 'del' && 'bg-deny-soft',
          )}
        >
          <span className="w-10 shrink-0 pr-2 text-right text-muted-foreground select-none">{l.n}</span>
          <span className="w-4 shrink-0 text-muted-foreground select-none">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''}</span>
          <span className="whitespace-pre">{l.text}</span>
        </div>
      ))}
    </div>
  )
}

export function TerminalOutput({ children }: { children: ReactNode }) {
  return <pre className="overflow-x-auto rounded-md bg-[#16181a] p-3 font-mono text-[12px] leading-relaxed text-[#dcddd8]">{children}</pre>
}

/** Freigabe-Karte: der Agent wartet auf dich. Die Fase markiert „du bist dran“. */
export function ApprovalCard({
  tool,
  command,
  reason,
  rule,
  variant = 'full',
  resolved,
}: {
  tool: string
  command: string
  reason: string
  rule?: string
  /** `minimal` = M0-Karte (WEB-018), `full` = Approval-Bar-Variante mit Policy-Begründung (WEB-007). */
  variant?: 'minimal' | 'full'
  resolved?: 'allowed' | 'denied'
}) {
  return (
    <div className="ml-9">
      <div
        className={cn(
          'chamfer border-l-4 p-3',
          resolved ? 'border-border bg-card' : 'border-signal bg-signal-soft',
        )}
      >
        <div className="flex items-baseline gap-2">
          <span className="text-[13px] font-semibold">
            {resolved === 'allowed' ? 'Erlaubt' : resolved === 'denied' ? 'Abgelehnt' : 'Freigabe nötig'}: {tool}
          </span>
          {rule && variant === 'full' && <span className="font-mono text-[11px] text-muted-foreground">Regel {rule}</span>}
        </div>
        <pre className="mt-1.5 overflow-x-auto rounded-sm bg-card px-2 py-1 font-mono text-[12px]">{command}</pre>
        {variant === 'full' && <p className="mt-1.5 text-[13px]">{reason}</p>}
        {!resolved && (
          <div className="mt-2.5 flex flex-wrap items-center gap-2">
            <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Erlauben</button>
            <button className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px]">Ablehnen</button>
            {variant === 'full' && (
              <button className="rounded-md px-2 py-1.5 text-[13px] text-muted-foreground hover:text-foreground">
                Für diese Session erlauben
              </button>
            )}
            <span className="ml-auto text-[11px] text-muted-foreground">
              <kbd className="font-mono">⌘↵</kbd> erlauben · <kbd className="font-mono">⌘⌫</kbd> ablehnen
            </span>
          </div>
        )}
      </div>
    </div>
  )
}

/** Systemhinweis im Verlauf, z. B. „Session geforkt“ oder „Runner neu gestartet“. */
export function SystemNote({ children, tone = 'neutral' }: { children: ReactNode; tone?: 'neutral' | 'deny' | 'ok' }) {
  return (
    <div className="flex items-center gap-3 text-[12px] text-muted-foreground">
      <span className="h-px flex-1 bg-border" />
      <span className={cn(tone === 'deny' && 'text-deny', tone === 'ok' && 'text-ok')}>{children}</span>
      <span className="h-px flex-1 bg-border" />
    </div>
  )
}

/** Fußzeile eines Turns: Dauer, Tokens, Kosten bzw. Subscription. */
export function TurnFooter({ duration, tokens, cost }: { duration: string; tokens: string; cost: string }) {
  return (
    <div className="ml-9 flex gap-3 text-[11px] text-muted-foreground tabular-nums">
      <span>{duration}</span>
      <span>{tokens} Tokens</span>
      <span>{cost}</span>
    </div>
  )
}

export function HarnessVoiceLabel({ harness }: { harness: HarnessId }) {
  return (
    <span className="inline-flex items-center gap-1">
      <VoiceDot voice={harnesses[harness].voice} />
      {harnesses[harness].name}
    </span>
  )
}
