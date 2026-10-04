import { useState } from 'react'
import type { Attachment } from '@beton/sdk'
import { Check, ChevronRight, ShieldAlert, SquareTerminal, X } from 'lucide-react'
import { client } from '@/lib/client'
import { argSummary, seconds } from '@/lib/format'
import type { Item, ToolState } from '@/lib/timeline'
import { cn } from '@/lib/utils'
import { harnessName, voiceOf, voiceVar } from './harness'
import { SentAttachment } from './attachments'
import { Markdown } from './markdown'

/** Nachricht des Menschen, mit Anhängen (WEB-006). */
export function UserMessage({ text, sessionId, attachments = [] }: { text: string; sessionId?: string; attachments?: Attachment[] }) {
  return (
    <div className="flex gap-3">
      <span className="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full bg-foreground text-[10px] font-semibold text-background">
        DU
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-xs text-muted-foreground">Du</div>
        {text && <div className="mt-0.5 text-[14px] leading-relaxed whitespace-pre-wrap break-words">{text}</div>}
        {sessionId && attachments.length > 0 && (
          <div className="mt-1.5 flex flex-wrap items-end gap-2">
            {attachments.map((a) => (
              <SentAttachment key={a.blob + a.name} sessionId={sessionId} attachment={a} />
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

/** Antwort des Agents mit Stimmfarbe des Harness. */
export function AgentMessage({ harness, text, streaming }: { harness: string; text: string; streaming: boolean }) {
  return (
    <div className="flex gap-3" data-testid="agent-message">
      <span className="mt-1 flex size-6 shrink-0 items-center justify-center">
        <span className="size-3 rounded-[3px]" style={{ background: voiceVar[voiceOf(harness)] }} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-xs text-muted-foreground">{harnessName(harness)}</div>
        <div className="mt-0.5 text-[14px] leading-relaxed break-words">
          <Markdown text={text} streaming={streaming} />
          {streaming && <span className="ml-0.5 inline-block h-4 w-1.5 animate-pulse bg-foreground align-text-bottom" />}
        </div>
      </div>
    </div>
  )
}

/** Gedankengang des Modells, standardmäßig eingeklappt. */
export function Reasoning({ text, streaming }: { text: string; streaming: boolean }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="ml-9 text-[13px] text-muted-foreground">
      <button onClick={() => setOpen((v) => !v)} className="inline-flex items-center gap-1 hover:text-foreground" aria-expanded={open}>
        <ChevronRight className={cn('size-3.5 transition-transform', open && 'rotate-90')} />
        {streaming ? 'Überlegt …' : 'Überlegung'}
      </button>
      {open && <div className="mt-1 border-l border-border pl-3 whitespace-pre-wrap italic">{text}</div>}
    </div>
  )
}

const statusText: Record<ToolState, string> = {
  requested: 'angefragt',
  running: 'läuft',
  completed: 'erledigt',
  failed: 'fehlgeschlagen',
  denied: 'abgelehnt',
}

/** Tool-Call als kompakte Zeile, aufklappbar (WEB-002 AC2). */
export function ToolCard({ item }: { item: Extract<Item, { kind: 'tool' }> }) {
  const [open, setOpen] = useState(false)
  const summary = argSummary(item.args)
  return (
    <div className="ml-9" data-testid="tool-card" data-status={item.status}>
      <button
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 rounded-md border border-transparent px-2 py-1 text-left text-[13px] hover:border-border hover:bg-card"
      >
        <SquareTerminal className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="font-medium">{item.tool}</span>
        <span className="min-w-0 truncate font-mono text-[12px] text-muted-foreground">{summary}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2 text-[11px] text-muted-foreground">
          {item.durationMs !== undefined && <span className="tabular-nums">{seconds(item.durationMs)}</span>}
          <span className="sr-only">{statusText[item.status]}</span>
          {(item.status === 'running' || item.status === 'requested') && (
            <span className={cn('size-1.5 rounded-full', item.status === 'running' ? 'animate-pulse bg-ok' : 'bg-muted-foreground')} />
          )}
          {item.status === 'completed' && <Check className="size-3.5 text-ok" />}
          {item.status === 'failed' && <X className="size-3.5 text-deny" />}
          {item.status === 'denied' && <ShieldAlert className="size-3.5 text-deny" />}
          <span aria-hidden>{statusText[item.status]}</span>
        </span>
      </button>
      {open && (
        <div className="mt-1 mb-2 ml-6 space-y-1.5">
          <pre className="overflow-x-auto rounded-md bg-sunken p-2 font-mono text-[12px]">{JSON.stringify(item.args, null, 2)}</pre>
          {item.output && <pre className="max-h-64 overflow-auto rounded-md bg-sunken p-2 font-mono text-[12px]">{item.output}</pre>}
          {item.result !== undefined && item.result !== null && (
            <pre className="max-h-64 overflow-auto rounded-md bg-sunken p-2 font-mono text-[12px]">
              {typeof item.result === 'string' ? item.result : JSON.stringify(item.result, null, 2)}
            </pre>
          )}
        </div>
      )}
    </div>
  )
}

/**
 * Freigabe-Karte (WEB-018): Tool-Name, Argumente als JSON (ausklappbar), „Erlauben“ und
 * „Ablehnen“ mit optionalem Kommentar an das Modell. Die Fase markiert „du bist dran“.
 */
export function ApprovalCard({ sessionId, item }: { sessionId: string; item: Extract<Item, { kind: 'approval' }> }) {
  const [showArgs, setShowArgs] = useState(false)
  const [comment, setComment] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | undefined>()
  const decided = item.decision !== undefined
  const resolve = async (allow: boolean) => {
    setBusy(true)
    setError(undefined)
    try {
      await client.session(sessionId).resolveApproval(item.approvalId, {
        decision: allow ? 'allow' : 'deny',
        ...(!allow && comment.trim() ? { reason: comment.trim() } : {}),
      })
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Entscheidung nicht übermittelt')
      setBusy(false)
    }
  }
  return (
    <div className="ml-9" data-testid="approval-card" data-decided={decided ? 'true' : 'false'}>
      <div className={cn('chamfer border-l-4 p-3', decided ? 'border-border bg-card' : 'border-signal bg-signal-soft')}>
        <div className="flex items-baseline gap-2">
          <span className="text-[13px] font-semibold">
            {decided ? (item.decision === 'allow' ? 'Erlaubt' : 'Abgelehnt') : 'Freigabe nötig'}: {item.tool}
          </span>
          {decided && <span className="text-[11px] text-muted-foreground">entschieden</span>}
        </div>
        <pre className="mt-1.5 overflow-x-auto rounded-sm bg-card px-2 py-1 font-mono text-[12px]">{argSummary(item.args) || item.tool}</pre>
        <button className="mt-1 text-[12px] text-muted-foreground hover:text-foreground" onClick={() => setShowArgs((v) => !v)} aria-expanded={showArgs}>
          {showArgs ? 'Argumente ausblenden' : 'Argumente als JSON'}
        </button>
        {showArgs && <pre className="mt-1 overflow-x-auto rounded-sm bg-card px-2 py-1 font-mono text-[12px]">{JSON.stringify(item.args, null, 2)}</pre>}
        {item.comment && <p className="mt-1.5 text-[12px] text-muted-foreground">Kommentar: {item.comment}</p>}
        {!decided && (
          <>
            <label className="mt-2 block text-[12px] text-muted-foreground">
              Kommentar an das Modell (nur beim Ablehnen)
              <input
                value={comment}
                onChange={(e) => setComment(e.target.value)}
                className="mt-1 block w-full rounded-md border border-input bg-card px-2 py-1 text-[13px] text-foreground"
                placeholder="z. B. Bitte erst die Tests laufen lassen"
              />
            </label>
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <button disabled={busy} onClick={() => void resolve(true)} className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background disabled:opacity-50">
                Erlauben
              </button>
              <button disabled={busy} onClick={() => void resolve(false)} className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px] disabled:opacity-50">
                Ablehnen
              </button>
              {error && <span className="text-[12px] text-deny">{error}</span>}
            </div>
          </>
        )}
      </div>
    </div>
  )
}

/** Abgelehnt durch eine Policy (WEB-002 AC3). */
export function PolicyCard({ rule, reason }: { rule: string; reason: string }) {
  return (
    <div className="ml-9 rounded-md border border-deny/40 bg-deny-soft px-3 py-2 text-[13px]" data-testid="policy-card">
      <div className="flex items-center gap-1.5 font-semibold">
        <ShieldAlert className="size-3.5 text-deny" /> Von Policy abgelehnt
        <span className="font-mono text-[11px] font-normal text-muted-foreground">Regel {rule}</span>
      </div>
      <p className="mt-0.5">{reason}</p>
    </div>
  )
}

export function ErrorCard({ title, detail }: { title: string; detail?: string | undefined }) {
  return (
    <div className="ml-9 rounded-md border border-deny/40 bg-deny-soft px-3 py-2 text-[13px]" role="alert">
      <div className="font-semibold">{title}</div>
      {detail && <pre className="mt-1 font-mono text-[12px] whitespace-pre-wrap">{detail}</pre>}
    </div>
  )
}

export function SystemNote({ text, tone }: { text: string; tone: 'neutral' | 'deny' | 'ok' }) {
  return (
    <div className="flex items-center gap-3 text-[12px] text-muted-foreground">
      <span className="h-px flex-1 bg-border" />
      <span className={cn(tone === 'deny' && 'text-deny', tone === 'ok' && 'text-ok')}>{text}</span>
      <span className="h-px flex-1 bg-border" />
    </div>
  )
}

/** Ausgelagerte Nutzlast (PROTO-001 AC4): Platzhalter, Inhalt auf Wunsch über die Blob-API. */
export function Offloaded({ sessionId, item, harness }: { sessionId: string; item: Extract<Item, { kind: 'offloaded' }>; harness: string }) {
  const [content, setContent] = useState<unknown>()
  const [state, setState] = useState<'idle' | 'loading' | 'failed'>('idle')
  const load = async () => {
    setState('loading')
    try {
      setContent(await client.session(sessionId).blob(item.ref))
      setState('idle')
    } catch {
      setState('failed')
    }
  }
  if (content !== undefined) {
    const c = content as { role?: string; content?: unknown[] }
    if (item.eventType === 'message.completed' && Array.isArray(c.content)) {
      const text = c.content.map((b) => (b && typeof b === 'object' && 'text' in b ? String((b as { text: unknown }).text) : '')).join('')
      return c.role === 'user' ? <UserMessage text={text} /> : <AgentMessage harness={harness} text={text} streaming={false} />
    }
    return (
      <pre className="ml-9 max-h-96 overflow-auto rounded-md bg-sunken p-2 font-mono text-[12px]">{JSON.stringify(content, null, 2)}</pre>
    )
  }
  return (
    <div className="ml-9 flex items-center gap-2 rounded-md border border-dashed border-border px-3 py-2 text-[13px]" data-testid="offloaded">
      <span className="text-muted-foreground">Großer Inhalt ({item.eventType}) ausgelagert</span>
      <button onClick={() => void load()} disabled={state === 'loading'} className="ml-auto rounded-md border border-border px-2 py-0.5 text-[12px] hover:bg-accent">
        {state === 'loading' ? 'Lädt …' : state === 'failed' ? 'Erneut laden' : 'Inhalt laden'}
      </button>
    </div>
  )
}
