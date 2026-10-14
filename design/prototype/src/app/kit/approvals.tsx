import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { Check, ChevronRight, Clock, FileText, MessageCircleQuestion, Plug, ShieldCheck, SquareTerminal, X } from 'lucide-react'
import { Diff } from '@/app/stream'
import { cn } from '@/lib/utils'
import type { DiffLine } from '@/mock/data'

/**
 * Freigabe-Leiste (WEB-007) und ihre Einträge.
 *
 * Offene Freigaben (`approval.requested`) und Fragen des Agents (`ask_user`) einer Session stehen in einer
 * fixierten Leiste direkt über dem Composer. Die oberste ist ausgeklappt und hat die Aktionen; weitere warten
 * gestapelt darunter. Entscheidet ein anderer Client, verschwindet der Eintrag mit Vermerk „entschieden von …“.
 * Im Verlauf bleibt die Karte als Inline-Darstellung erhalten (WEB-018, `ApprovalCard`).
 *
 * Tastatur (auch aus dem Composer heraus):
 * - `⌘↵` erlaubt die oberste Freigabe bzw. sendet die Antwort auf die oberste Frage.
 * - `⌘⇧↵` erlaubt für diese Session (nur wenn alle beteiligten Regeln `remember: session` zulassen).
 * - `⌘⌫` öffnet „Ablehnen“ mit optionalem Grund, ein zweites `⌘⌫` oder `↵` lehnt ab, `Esc` bricht ab.
 *   In einem Eingabefeld mit Text bleibt `⌘⌫` normales Löschen.
 * - `⌘⇧A` springt in die Leiste; dort `↑`/`↓` zwischen den gestapelten Einträgen, `Tab` durch die Aktionen,
 *   `Esc` zurück in den Composer.
 */

export type ApprovalReason = {
  /** Regel-ID, z. B. `confirm-destructive-shell`. */
  rule: string
  /** Herkunft, z. B. „Projekt · git-and-budget“. */
  origin: string
  text: string
  severity?: 'low' | 'medium' | 'high'
}

export type BarItem = {
  id: string
  kind: 'approval' | 'question'
  /** Werkzeugklasse für das Symbol. */
  tool?: 'shell' | 'file' | 'mcp' | 'session'
  /** z. B. „Shell-Kommando“, „Datei bearbeiten“, „MCP-Tool github/create_pull_request“. */
  title: string
  /** Kommando, Pfad oder Frage. */
  summary: string
  /** Wer fragt: Harness, ggf. Agent. */
  requestedBy: string
  reasons?: ApprovalReason[]
  /** Normalisierte Tool-Argumente als JSON (ausklappbar). */
  args?: string
  /** Bei Datei-Edits: Vorschau der Änderung (ausklappbar, statt der Argumente). */
  diff?: { file: string; lines: DiffLine[]; stat: string }
  /** z. B. „läuft in 27 Min. ab · dann ablehnen“. */
  timeout: string
  /** Weniger als 5 Minuten bis zum Ablauf: Rost („wird knapp“). */
  urgent?: boolean
  /** `true`: „Für diese Session erlauben“ anbieten; Text: Erklärung, warum nicht. */
  remember?: true | string
  /** Antwortvorschläge des Agents bei Fragen. */
  options?: string[]
  /** Zusatzhinweis unter den Gründen. */
  note?: ReactNode
  /** Nur Leserecht: statt Aktionen eine Erklärung. */
  readOnly?: ReactNode
}

/** Vermerk über einen Eintrag, der gerade die Leiste verlassen hat. */
export type BarNotice =
  | { kind: 'decided'; summary: string; decision: 'allow' | 'allow_session' | 'deny' | 'answer'; by: string; where: string; comment?: string }
  | { kind: 'expired'; summary: string; after: string; applied: string }

const toolIcon = { shell: SquareTerminal, file: FileText, mcp: Plug, session: ShieldCheck }

function ItemIcon({ item, className }: { item: BarItem; className?: string }) {
  const Icon = item.kind === 'question' ? MessageCircleQuestion : toolIcon[item.tool ?? 'shell']
  return <Icon className={cn('size-4 shrink-0', className)} aria-hidden />
}

function Keys({ children }: { children: ReactNode }) {
  return <kbd className="rounded-sm border border-foreground/20 px-1 font-mono text-[10.5px] leading-4">{children}</kbd>
}

/** Ausklappbare Details: Argumente als JSON oder Diff bei Datei-Edits. */
export function ApprovalDetails({ item, open, onToggle }: { item: BarItem; open: boolean; onToggle: () => void }) {
  if (!item.args && !item.diff) return null
  const label = item.diff ? `Änderung an ${item.diff.file}` : 'Argumente'
  return (
    <div className="mt-2">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="inline-flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground"
      >
        <ChevronRight className={cn('size-3.5 transition-transform', open && 'rotate-90')} />
        {label}
        {item.diff && <span className="font-mono tabular-nums">{item.diff.stat}</span>}
      </button>
      {open && (
        <div className="mt-1.5 max-h-56 overflow-auto">
          {item.diff ? (
            <Diff file={item.diff.file} lines={item.diff.lines} />
          ) : (
            <pre className="rounded-sm bg-card px-2 py-1.5 font-mono text-[12px] leading-relaxed">{item.args}</pre>
          )}
        </div>
      )}
    </div>
  )
}

function NoticeLine({ notice }: { notice: BarNotice }) {
  if (notice.kind === 'expired') {
    return (
      <p className="flex items-start gap-2 px-1 pb-2 text-[12px] text-muted-foreground">
        <Clock className="mt-0.5 size-3.5 shrink-0 text-deny" aria-hidden />
        <span>
          <code className="font-mono text-foreground">{notice.summary}</code> ist abgelaufen: niemand hat in {notice.after} entschieden.
          Angewendet: <b className="text-deny">{notice.applied}</b>.
        </span>
      </p>
    )
  }
  const verb =
    notice.decision === 'allow'
      ? 'Erlaubt'
      : notice.decision === 'allow_session'
        ? 'Für diese Session erlaubt'
        : notice.decision === 'answer'
          ? 'Beantwortet'
          : 'Abgelehnt'
  const Icon = notice.decision === 'deny' ? X : Check
  return (
    <p className="flex items-start gap-2 px-1 pb-2 text-[12px] text-muted-foreground">
      <Icon className={cn('mt-0.5 size-3.5 shrink-0', notice.decision === 'deny' ? 'text-deny' : 'text-ok')} aria-hidden />
      <span>
        <code className="font-mono text-foreground">{notice.summary}</code> – {verb} von <b className="text-foreground">{notice.by}</b>{' '}
        ({notice.where}){notice.comment && <>: „{notice.comment}“</>}
      </span>
    </p>
  )
}

/**
 * Die fixierte Leiste über dem Composer. Im Prototyp klickbar: Erlauben/Ablehnen nimmt den obersten Eintrag
 * weg und zeigt den nächsten; ein Klick auf einen gestapelten Eintrag holt ihn nach oben.
 */
export function ApprovalBar({
  items: initial,
  expanded: initialExpanded = false,
  denying: initialDenying = false,
  answer: initialAnswer = '',
  notice: initialNotice,
}: {
  items: BarItem[]
  /** Argumente bzw. Diff des obersten Eintrags ausgeklappt. */
  expanded?: boolean
  /** Ablehnen mit Grund geöffnet (Text vorbelegt). */
  denying?: boolean | string
  /** Vorbelegte Antwort auf eine Frage. */
  answer?: string
  /** Vermerk über einen gerade entschiedenen oder abgelaufenen Eintrag. */
  notice?: BarNotice
}) {
  const [queue, setQueue] = useState(initial)
  const [expanded, setExpanded] = useState(initialExpanded)
  const [denying, setDenying] = useState(!!initialDenying)
  const [reason, setReason] = useState(typeof initialDenying === 'string' ? initialDenying : '')
  const [answer, setAnswer] = useState(initialAnswer)
  const [notice, setNotice] = useState<BarNotice | undefined>(initialNotice)
  const barRef = useRef<HTMLElement>(null)
  const reasonRef = useRef<HTMLInputElement>(null)
  const titleId = useId()
  const descId = useId()

  const top = queue[0]
  const rest = queue.slice(1)

  function resolve(decision: 'allow' | 'allow_session' | 'deny' | 'answer') {
    if (!top) return
    setNotice({
      kind: 'decided',
      summary: top.kind === 'question' ? top.title : top.summary.split('\n')[0],
      decision,
      by: 'dir',
      where: 'gerade eben',
      comment: decision === 'deny' ? reason || undefined : decision === 'answer' ? answer || undefined : undefined,
    })
    setQueue((q) => q.slice(1))
    setExpanded(false)
    setDenying(false)
    setReason('')
    setAnswer('')
  }

  function bringToFront(id: string) {
    setQueue((q) => [q.find((i) => i.id === id)!, ...q.filter((i) => i.id !== id)])
    setExpanded(false)
    setDenying(false)
  }

  useEffect(() => {
    if (denying) reasonRef.current?.focus()
  }, [denying])

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey
      if (!top || top.readOnly) return
      const target = e.target as HTMLElement | null
      const inText = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
      if (mod && e.key === 'Enter') {
        e.preventDefault()
        if (top.kind === 'question') {
          if (answer.trim()) resolve('answer')
        } else if (e.shiftKey) {
          if (top.remember === true) resolve('allow_session')
        } else {
          resolve('allow')
        }
      } else if (mod && e.key === 'Backspace' && top.kind === 'approval') {
        if (inText && target !== reasonRef.current && (target as HTMLInputElement).value) return
        e.preventDefault()
        if (denying) resolve('deny')
        else setDenying(true)
      } else if (mod && e.shiftKey && e.key.toLowerCase() === 'a') {
        e.preventDefault()
        barRef.current?.focus()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  if (!top && !notice) return null

  return (
    <section
      ref={barRef}
      tabIndex={-1}
      role={top ? 'alertdialog' : undefined}
      aria-labelledby={top ? titleId : undefined}
      aria-describedby={top ? descId : undefined}
      aria-label={top ? undefined : 'Freigaben'}
      onKeyDown={(e) => {
        if (e.target !== barRef.current) return
        if (e.key === 'ArrowDown' && rest[0]) {
          e.preventDefault()
          setQueue((q) => [...q.slice(1), q[0]])
        } else if (e.key === 'ArrowUp' && rest.length) {
          e.preventDefault()
          setQueue((q) => [q[q.length - 1], ...q.slice(0, -1)])
        }
      }}
      className="shrink-0 border-t border-border bg-background px-3 pt-2.5 outline-none focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring"
    >
      <div aria-live="polite">{notice && <NoticeLine notice={notice} />}</div>

      {top && (
        <div className={cn('border-l-4 p-3', top.readOnly ? 'rounded-r-md border-border bg-card' : 'chamfer border-signal bg-signal-soft')}>
          <div className="flex items-center gap-2">
            <ItemIcon item={top} />
            <h3 id={titleId} className="text-[13px] font-semibold">
              {top.kind === 'question' ? 'Frage' : 'Freigabe nötig'}: {top.title}
            </h3>
            <span className="text-[12px] text-muted-foreground">· {top.requestedBy}</span>
            <span className="ml-auto flex shrink-0 items-center gap-3 text-[11px] whitespace-nowrap text-muted-foreground">
              {queue.length > 1 && (
                <span className="tabular-nums" aria-label={`1 von ${queue.length} offenen Einträgen`}>
                  1 von {queue.length}
                </span>
              )}
              <span className={cn('inline-flex items-center gap-1', top.urgent && 'font-semibold text-warn')}>
                <Clock className="size-3" aria-hidden /> {top.timeout}
              </span>
            </span>
          </div>

          <div id={descId}>
            {top.kind === 'question' ? (
              <p className="mt-1.5 text-[14px]">{top.summary}</p>
            ) : (
              <pre className="mt-1.5 overflow-x-auto rounded-sm bg-card px-2 py-1 font-mono text-[12px] whitespace-pre-wrap">{top.summary}</pre>
            )}
            {top.reasons && top.reasons.length > 0 && (
              <ul className="mt-2 space-y-1" aria-label="Gründe">
                {top.reasons.map((r) => (
                  <li key={r.rule} className="flex items-baseline gap-2 text-[13px]">
                    <span className="text-muted-foreground" aria-hidden>
                      ·
                    </span>
                    <span className="min-w-0 flex-1">{r.text}</span>
                    <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                      {r.origin}/{r.rule}
                    </span>
                    {r.severity === 'high' && (
                      <span className="rounded-sm border border-border px-1 text-[11px] leading-4 text-muted-foreground">hoch</span>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
          {top.note}

          <ApprovalDetails item={top} open={expanded} onToggle={() => setExpanded((v) => !v)} />

          {top.readOnly ? (
            <div className="mt-2.5 flex items-start gap-2 text-[12px] text-muted-foreground">{top.readOnly}</div>
          ) : top.kind === 'question' ? (
            <div className="mt-2.5 flex flex-col gap-2">
              {top.options && (
                <div className="flex flex-wrap gap-2">
                  {top.options.map((o) => (
                    <button
                      key={o}
                      type="button"
                      onClick={() => setAnswer(o)}
                      className={cn('rounded-md border px-3 py-1.5 text-[13px]', answer === o ? 'border-foreground bg-card font-medium' : 'border-foreground/30 bg-card')}
                    >
                      {o}
                    </button>
                  ))}
                </div>
              )}
              <div className="flex items-center gap-2">
                <input
                  value={answer}
                  onChange={(e) => setAnswer(e.target.value)}
                  placeholder="Eigene Antwort …"
                  aria-label="Antwort an den Agent"
                  className="h-8 flex-1 rounded-md border border-input bg-card px-2 text-[13px] outline-none focus-visible:outline-2 focus-visible:outline-ring"
                />
                <button
                  type="button"
                  disabled={!answer.trim()}
                  onClick={() => resolve('answer')}
                  className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background disabled:bg-muted disabled:text-muted-foreground"
                >
                  Antworten
                </button>
                <span className="text-[11px] text-muted-foreground">
                  <Keys>⌘↵</Keys>
                </span>
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
                    resolve('deny')
                  } else if (e.key === 'Escape') {
                    e.preventDefault()
                    setDenying(false)
                  }
                }}
                placeholder="Grund für den Agent (optional)"
                aria-label="Grund für die Ablehnung, geht an den Agent"
                className="h-8 min-w-60 flex-1 rounded-md border border-input bg-card px-2 text-[13px] outline-none focus-visible:outline-2 focus-visible:outline-ring"
              />
              <button type="button" onClick={() => resolve('deny')} className="rounded-md border border-deny/60 bg-card px-3 py-1.5 text-[13px] text-deny">
                Ablehnen
              </button>
              <button type="button" onClick={() => setDenying(false)} className="rounded-md px-2 py-1.5 text-[13px] text-muted-foreground hover:text-foreground">
                Zurück
              </button>
              <span className="ml-auto text-[11px] text-muted-foreground">
                <Keys>↵</Keys> oder <Keys>⌘⌫</Keys> ablehnen · <Keys>Esc</Keys> zurück
              </span>
            </div>
          ) : (
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <button type="button" onClick={() => resolve('allow')} className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">
                Erlauben
              </button>
              {top.remember === true && (
                <button type="button" onClick={() => resolve('allow_session')} className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px]">
                  Für diese Session erlauben
                </button>
              )}
              <button type="button" onClick={() => setDenying(true)} className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px]">
                Ablehnen …
              </button>
              {typeof top.remember === 'string' && <span className="text-[11px] text-muted-foreground">{top.remember}</span>}
              <span className="ml-auto flex gap-2 text-[11px] text-muted-foreground">
                <span>
                  <Keys>⌘↵</Keys> erlauben
                </span>
                {top.remember === true && (
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
        </div>
      )}

      {rest.length > 0 && (
        <div className="mt-1 mb-0.5">
          <div className="px-1 pt-1 pb-0.5 text-[11px] text-muted-foreground">Danach · {rest.length} weitere</div>
          <ul className="flex flex-col">
            {rest.map((it) => (
              <li key={it.id}>
                <button
                  type="button"
                  onClick={() => bringToFront(it.id)}
                  className="flex w-full items-center gap-2 border-l-4 border-signal/50 px-2.5 py-1 text-left text-[12px] hover:bg-accent"
                  title="Nach oben holen"
                >
                  <span className="chamfer-sm size-2 shrink-0 bg-signal" aria-hidden />
                  <ItemIcon item={it} className="size-3.5 text-muted-foreground" />
                  <span className="shrink-0 font-medium">{it.kind === 'question' ? 'Frage' : it.title}</span>
                  <span className={cn('min-w-0 truncate text-muted-foreground', it.kind === 'approval' && 'font-mono text-[11.5px]')}>
                    {it.summary.split('\n')[0]}
                  </span>
                  <span className={cn('ml-auto shrink-0 text-[11px] text-muted-foreground', it.urgent && 'font-semibold text-warn')}>
                    {it.timeout.split(' · ')[0]}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
      {!top && <div className="pb-0.5" />}
    </section>
  )
}
