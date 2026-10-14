import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import type { ResolveApprovalRequest } from '@beton/sdk'
import { Check, ChevronRight, Clock, FileText, MessageCircleQuestion, Plug, ShieldCheck, SquareTerminal, X } from 'lucide-react'
import { client } from '@/lib/client'
import { editPreview, resolutionText, scopeLabel, timeLeft } from '@/lib/approvals'
import type { ApprovalItem } from '@/lib/timeline'
import { cn } from '@/lib/utils'

/*
 * Freigabe-Leiste (WEB-007) nach dem abgenommenen Entwurf (`design/prototype/src/app/kit/approvals.tsx`):
 * offene Freigaben und Fragen der Session in einer fixierten Leiste über dem Composer. Die oberste ist
 * ausgeklappt und hat die Aktionen, weitere warten gestapelt darunter. Entscheidet ein anderer Client,
 * verschwindet der Eintrag mit Vermerk „… von …“.
 *
 * Tastatur (auch aus dem Composer): `⌘↵` erlauben bzw. antworten, `⌘⇧↵` für diese Session erlauben,
 * `⌘⌫` ablehnen (Grund optional, zweites `⌘⌫` oder `↵` lehnt ab), `⌘⇧A` in die Leiste springen, dort
 * `↑`/`↓` durch den Stapel, `Esc` zurück in die Eingabe. In einem Feld mit Text bleibt `⌘⌫` Löschen.
 */

function kindIcon(item: ApprovalItem) {
  if (item.approvalKind === 'question') return MessageCircleQuestion
  switch (item.toolKind) {
    case 'shell':
      return SquareTerminal
    case 'file_write':
    case 'file_edit':
      return FileText
    case 'mcp':
      return Plug
    default:
      return ShieldCheck
  }
}

function Keys({ children }: { children: ReactNode }) {
  return <kbd className="rounded-sm border border-foreground/20 px-1 font-mono text-[10.5px] leading-4">{children}</kbd>
}

/** Ausklappbare Details: Argumente als JSON bzw. Diff bei Datei-Edits (WEB-007). */
export function ApprovalDetails({ item, open, onToggle }: { item: ApprovalItem; open: boolean; onToggle: () => void }) {
  if (item.approvalKind === 'question') return null
  const diff = editPreview(item)
  const args = item.argsModified ?? item.args
  if (!diff && (args === undefined || args === null)) return null
  return (
    <div className="mt-2">
      <button type="button" onClick={onToggle} aria-expanded={open} className="inline-flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground">
        <ChevronRight className={cn('size-3.5 transition-transform', open && 'rotate-90')} />
        {diff ? `Änderung an ${diff.file || 'Datei'}` : 'Argumente'}
        {diff && <span className="font-mono tabular-nums">{diff.stat}</span>}
      </button>
      {open && (
        <div className="mt-1.5 max-h-56 overflow-auto" data-testid="approval-details">
          {diff ? (
            <div className="overflow-hidden rounded-md border border-border bg-card font-mono text-[12px]">
              <div className="border-b border-border bg-sunken px-3 py-1 text-[11px] text-muted-foreground">{diff.file}</div>
              {diff.lines.map((l, i) => (
                <div key={i} className={cn('flex', l.kind === 'add' && 'bg-ok-soft', l.kind === 'del' && 'bg-deny-soft')}>
                  <span className="w-5 shrink-0 text-center text-muted-foreground select-none">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''}</span>
                  <span className="whitespace-pre">{l.text}</span>
                </div>
              ))}
            </div>
          ) : (
            <pre className="rounded-sm bg-card px-2 py-1.5 font-mono text-[12px] leading-relaxed">{JSON.stringify(args, null, 2)}</pre>
          )}
          {item.argsModified !== undefined && <p className="mt-1 text-[11px] text-muted-foreground">Von einer Policy geändert; so wird der Aufruf ausgeführt.</p>}
        </div>
      )}
    </div>
  )
}

/** Gründe einer Card mit Herkunft. */
export function ApprovalReasons({ item }: { item: ApprovalItem }) {
  if (item.reasons.length === 0) return null
  return (
    <ul className="mt-2 space-y-1" aria-label="Gründe">
      {item.reasons.map((r, i) => (
        <li key={`${r.ruleId}-${i}`} className="flex flex-wrap items-baseline gap-x-2 text-[13px]">
          <span className="text-muted-foreground" aria-hidden>
            ·
          </span>
          <span className="min-w-0 flex-1">{r.reason}</span>
          {r.ruleId && (
            <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
              {scopeLabel(r.scope, r.set)}/{r.ruleId}
            </span>
          )}
          {r.severity === 'high' && <span className="rounded-sm border border-border px-1 text-[11px] leading-4 text-muted-foreground">hoch</span>}
        </li>
      ))}
    </ul>
  )
}

/** In einem Textfeld? Dann gehören Tasten wie `⌘⌫` dem Feld. */
function typingIn(target: EventTarget | null): HTMLInputElement | HTMLTextAreaElement | undefined {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement ? target : undefined
}

export function ApprovalBar({ sessionId, approvals }: { sessionId: string; approvals: ApprovalItem[] }) {
  const open = approvals.filter((a) => !a.decision)
  // Reihenfolge: älteste zuerst; ein angeklickter gestapelter Eintrag kommt nach vorn.
  const [front, setFront] = useState<string | undefined>()
  const queue = front && open.some((a) => a.approvalId === front) ? [open.find((a) => a.approvalId === front)!, ...open.filter((a) => a.approvalId !== front)] : open
  const top = queue[0]
  const rest = queue.slice(1)

  const [expanded, setExpanded] = useState(false)
  const [denying, setDenying] = useState(false)
  const [reason, setReason] = useState('')
  const [answer, setAnswer] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | undefined>()
  const [notice, setNotice] = useState<ApprovalItem | undefined>()
  const own = useRef(new Set<string>())
  const seen = useRef(new Set<string>())
  const barRef = useRef<HTMLElement>(null)
  const reasonRef = useRef<HTMLInputElement>(null)
  const titleId = useId()
  const descId = useId()
  const [, setTick] = useState(0)

  // Vermerk, sobald ein Eintrag die Leiste verlässt (egal von welchem Client, WEB-007 AC2).
  useEffect(() => {
    for (const a of approvals) {
      if (!a.decision) seen.current.add(a.approvalId)
      else if (seen.current.has(a.approvalId)) {
        seen.current.delete(a.approvalId)
        setNotice(a)
      }
    }
  }, [approvals])

  // Neuer oberster Eintrag: eingeklappt, ohne offenes Ablehnen.
  const topId = top?.approvalId
  useEffect(() => {
    setExpanded(false)
    setDenying(false)
    setReason('')
    setAnswer('')
    setError(undefined)
    setBusy(false)
  }, [topId])

  useEffect(() => {
    if (denying) reasonRef.current?.focus()
  }, [denying])

  // Restzeit aktuell halten.
  useEffect(() => {
    if (!top) return
    const t = window.setInterval(() => setTick((n) => n + 1), 30_000)
    return () => window.clearInterval(t)
  }, [top])

  const resolve = async (item: ApprovalItem, req: ResolveApprovalRequest) => {
    setBusy(true)
    setError(undefined)
    own.current.add(item.approvalId)
    try {
      await client.session(sessionId).resolveApproval(item.approvalId, req)
    } catch (e) {
      own.current.delete(item.approvalId)
      setError(e instanceof Error ? e.message : 'Entscheidung nicht übermittelt')
      setBusy(false)
    }
  }
  const allow = (session = false) => {
    if (!top || busy || top.approvalKind === 'question') return
    if (session && !top.options.includes('approve_session')) return
    void resolve(top, session ? { decision: 'allow', remember: 'session' } : { decision: 'allow' })
  }
  const deny = () => {
    if (!top || busy) return
    void resolve(top, reason.trim() ? { decision: 'deny', reason: reason.trim() } : { decision: 'deny' })
  }
  const sendAnswer = () => {
    if (!top || busy || !answer.trim()) return
    void resolve(top, { decision: 'answer', answer: answer.trim() })
  }

  // Tastatur (WEB-007 AC1, AC3): in der Capture-Phase, damit der Composer `⌘↵` nicht als Senden nimmt.
  useEffect(() => {
    if (!top) return
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (!mod) return
      const field = typingIn(e.target)
      if (e.key === 'Enter') {
        e.preventDefault()
        e.stopPropagation()
        if (top.approvalKind === 'question') sendAnswer()
        else allow(e.shiftKey)
      } else if (e.key === 'Backspace' && top.approvalKind !== 'question') {
        if (field && field !== reasonRef.current && field.value) return
        e.preventDefault()
        e.stopPropagation()
        if (denying) deny()
        else setDenying(true)
      } else if (e.shiftKey && e.key.toLowerCase() === 'a') {
        e.preventDefault()
        e.stopPropagation()
        barRef.current?.focus()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  })

  if (!top && !notice) return null
  const left = top ? timeLeft(top.expiresAt, top.onTimeout) : undefined
  const Icon = top ? kindIcon(top) : ShieldCheck
  const question = top?.approvalKind === 'question'

  return (
    <section
      ref={barRef}
      tabIndex={-1}
      data-testid="approval-bar"
      role={top ? 'alertdialog' : 'region'}
      aria-modal={top ? false : undefined}
      aria-labelledby={top ? titleId : undefined}
      aria-describedby={top ? descId : undefined}
      aria-label={top ? undefined : 'Freigaben'}
      onKeyDown={(e) => {
        if (e.target !== barRef.current) return
        if (e.key === 'ArrowDown' && rest[0]) {
          e.preventDefault()
          setFront(rest[0].approvalId)
        } else if (e.key === 'ArrowUp' && rest.length) {
          e.preventDefault()
          setFront(rest[rest.length - 1]!.approvalId)
        } else if (e.key === 'Escape') {
          e.preventDefault()
          document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Nachricht"]')?.focus()
        }
      }}
      className="shrink-0 border-t border-border bg-background px-3 pt-2.5 outline-none focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring"
    >
      <div aria-live="polite">
        {notice?.resolution && (
          <p className="flex items-start gap-2 px-1 pb-2 text-[12px] text-muted-foreground" data-testid="approval-notice">
            {notice.resolution.via === 'timeout' ? (
              <Clock className="mt-0.5 size-3.5 shrink-0 text-deny" aria-hidden />
            ) : notice.resolution.decision === 'deny' || notice.resolution.decision === 'abort' ? (
              <X className="mt-0.5 size-3.5 shrink-0 text-deny" aria-hidden />
            ) : (
              <Check className="mt-0.5 size-3.5 shrink-0 text-ok" aria-hidden />
            )}
            <span>
              <code className="font-mono text-foreground">{notice.summary}</code> – {resolutionText(notice.resolution, own.current.has(notice.approvalId))}
            </span>
          </p>
        )}
      </div>

      {top && (
        <div className="chamfer border-l-4 border-signal bg-signal-soft p-3" data-testid="approval-top" data-approval-id={top.approvalId}>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <Icon className="size-4 shrink-0" aria-hidden />
            <h3 id={titleId} className="text-[13px] font-semibold">
              {question ? 'Frage' : 'Freigabe nötig'}: {question ? top.summary.slice(0, 80) : top.title}
            </h3>
            {top.requestedBy && <span className="text-[12px] text-muted-foreground">· {top.requestedBy}</span>}
            <span className="ml-auto flex shrink-0 items-center gap-3 text-[11px] whitespace-nowrap text-muted-foreground">
              {queue.length > 1 && <span className="tabular-nums">1 von {queue.length}</span>}
              {left && (
                <span className={cn('inline-flex items-center gap-1', left.urgent && 'font-semibold text-warn')}>
                  <Clock className="size-3" aria-hidden /> {left.text}
                </span>
              )}
            </span>
          </div>

          <div id={descId}>
            {question ? (
              <p className="mt-1.5 text-[14px] whitespace-pre-wrap">{top.question}</p>
            ) : (
              <pre className="mt-1.5 overflow-x-auto rounded-sm bg-card px-2 py-1 font-mono text-[12px] whitespace-pre-wrap">{top.summary}</pre>
            )}
            <ApprovalReasons item={top} />
          </div>
          <ApprovalDetails item={top} open={expanded} onToggle={() => setExpanded((v) => !v)} />

          {question ? (
            <div className="mt-2.5 flex flex-col gap-2">
              {top.choices.length > 0 && (
                <div className="flex flex-wrap gap-2">
                  {top.choices.map((c) => (
                    <button
                      key={c}
                      type="button"
                      onClick={() => setAnswer(c)}
                      aria-pressed={answer === c}
                      className={cn('rounded-md border px-3 py-1.5 text-[13px]', answer === c ? 'border-foreground bg-card font-medium' : 'border-foreground/30 bg-card')}
                    >
                      {c}
                    </button>
                  ))}
                </div>
              )}
              <div className="flex items-center gap-2">
                <input
                  value={answer}
                  onChange={(e) => setAnswer(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && !e.metaKey && !e.ctrlKey) {
                      e.preventDefault()
                      sendAnswer()
                    }
                  }}
                  placeholder="Eigene Antwort …"
                  aria-label="Antwort an den Agent"
                  className="h-8 min-w-0 flex-1 rounded-md border border-input bg-card px-2 text-[13px]"
                />
                <button type="button" disabled={busy || !answer.trim()} onClick={sendAnswer} className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background disabled:bg-muted disabled:text-muted-foreground">
                  Antworten
                </button>
                <button type="button" disabled={busy} onClick={deny} className="rounded-md px-2 py-1.5 text-[13px] text-muted-foreground hover:text-foreground">
                  Nicht beantworten
                </button>
              </div>
            </div>
          ) : denying ? (
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <input
                ref={reasonRef}
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.metaKey && !e.ctrlKey) {
                    e.preventDefault()
                    deny()
                  } else if (e.key === 'Escape') {
                    e.preventDefault()
                    e.stopPropagation()
                    setDenying(false)
                  }
                }}
                placeholder="Grund für den Agent (optional)"
                aria-label="Grund für die Ablehnung, geht an den Agent"
                className="h-8 min-w-48 flex-1 rounded-md border border-input bg-card px-2 text-[13px]"
              />
              <button type="button" disabled={busy} onClick={deny} className="rounded-md border border-deny/60 bg-card px-3 py-1.5 text-[13px] text-deny">
                Ablehnen
              </button>
              <button type="button" onClick={() => setDenying(false)} className="rounded-md px-2 py-1.5 text-[13px] text-muted-foreground hover:text-foreground">
                Zurück
              </button>
              <span className="ml-auto hidden text-[11px] text-muted-foreground sm:inline">
                <Keys>↵</Keys> oder <Keys>⌘⌫</Keys> ablehnen · <Keys>Esc</Keys> zurück
              </span>
            </div>
          ) : (
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <button type="button" disabled={busy} onClick={() => allow()} className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background disabled:opacity-50">
                Erlauben
              </button>
              {top.options.includes('approve_session') && (
                <button type="button" disabled={busy} onClick={() => allow(true)} className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px] disabled:opacity-50">
                  Für diese Session erlauben
                </button>
              )}
              <button type="button" disabled={busy} onClick={() => setDenying(true)} className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px] disabled:opacity-50">
                Ablehnen …
              </button>
              <span className="ml-auto hidden gap-2 text-[11px] text-muted-foreground sm:flex">
                <span>
                  <Keys>⌘↵</Keys> erlauben
                </span>
                {top.options.includes('approve_session') && (
                  <span>
                    <Keys>⌘⇧↵</Keys> für Session
                  </span>
                )}
                <span>
                  <Keys>⌘⌫</Keys> ablehnen
                </span>
              </span>
            </div>
          )}
          {error && (
            <p className="mt-2 text-[12px] text-deny" role="alert">
              {error}
            </p>
          )}
        </div>
      )}

      {rest.length > 0 && (
        <div className="mt-1 mb-0.5">
          <div className="px-1 pt-1 pb-0.5 text-[11px] text-muted-foreground">Danach · {rest.length} weitere</div>
          <ul className="flex flex-col" aria-label="Weitere offene Freigaben">
            {rest.map((it) => {
              const I = kindIcon(it)
              const l = timeLeft(it.expiresAt, it.onTimeout)
              return (
                <li key={it.approvalId}>
                  <button
                    type="button"
                    onClick={() => setFront(it.approvalId)}
                    className="flex w-full items-center gap-2 border-l-4 border-signal/50 px-2.5 py-1 text-left text-[12px] hover:bg-accent"
                    title="Nach oben holen"
                  >
                    <span className="chamfer-sm size-2 shrink-0 bg-signal" aria-hidden />
                    <I className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
                    <span className="shrink-0 font-medium">{it.approvalKind === 'question' ? 'Frage' : it.title}</span>
                    <span className={cn('min-w-0 truncate text-muted-foreground', it.approvalKind !== 'question' && 'font-mono text-[11.5px]')}>
                      {it.summary.split('\n')[0]}
                    </span>
                    {l && <span className={cn('ml-auto shrink-0 text-[11px] text-muted-foreground', l.urgent && 'font-semibold text-warn')}>{l.text.split(' · ')[0]}</span>}
                  </button>
                </li>
              )
            })}
          </ul>
        </div>
      )}
      {!top && <div className="pb-0.5" />}
    </section>
  )
}
