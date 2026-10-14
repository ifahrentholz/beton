import { useEffect, useMemo, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import type { InboxItemView, InboxPage, ResolveApprovalRequest } from '@beton/sdk'
import { Bot, Check, Gauge, Inbox as InboxIcon, Menu, MessageCircleQuestion, ShieldCheck } from 'lucide-react'
import { client } from '@/lib/client'
import { resolutionFrom, resolutionText, timeLeft } from '@/lib/approvals'
import { ago } from '@/lib/format'
import { approvalItem, type ApprovalItem } from '@/lib/timeline'
import { cn } from '@/lib/utils'
import { ApprovalDetails, ApprovalReasons } from './approval-bar'
import { useLayout } from './layout-state'

/*
 * Inbox (UX-001) nach dem abgenommenen Entwurf (`design/prototype`, Screen `inbox-list`): Freigaben und
 * Fragen aller Sessions oben („Du bist dran“, gelb mit Fase), direkt beantwortbar – auch für pausierte
 * Sessions ohne Zuschauer –, darunter Hinweise. Tastatur: `j`/`k` wechseln, `a` erlauben, `d` ablehnen,
 * `↵` Session öffnen.
 */

/** Abstand der Abfragen: neue Einträge erscheinen binnen 1 s (UX-001 AC1). */
export const INBOX_POLL_MS = 1000

export type InboxFilter = 'all' | 'open' | 'approval' | 'question' | 'notice' | 'done'

const FILTERS: { id: InboxFilter; label: string }[] = [
  { id: 'all', label: 'Alles Neue' },
  { id: 'open', label: 'Offen' },
  { id: 'approval', label: 'Freigaben' },
  { id: 'question', label: 'Fragen' },
  { id: 'notice', label: 'Hinweise' },
  { id: 'done', label: 'Erledigt' },
]

function query(filter: InboxFilter): Parameters<typeof client.inbox.list>[0] {
  switch (filter) {
    case 'open':
      return { status: 'open', limit: 200 }
    case 'approval':
      return { kind: 'approval', status: 'open', limit: 200 }
    case 'question':
      return { kind: 'question', status: 'open', limit: 200 }
    case 'notice':
      return { kind: 'notice', limit: 200 }
    case 'done':
      return { status: 'done', limit: 200 }
    default:
      return { limit: 200 }
  }
}

/** Zähler der Inbox für die Navigation (UX-001 AC3). */
export function useInboxCount(): number {
  const q = useQuery({
    queryKey: ['inbox', 'count'],
    queryFn: () => client.inbox.list({ status: 'open', limit: 1 }),
    refetchInterval: INBOX_POLL_MS,
  })
  return q.data?.open_count ?? 0
}

/** Eine Freigabe bzw. Frage der Inbox als Card-Daten. */
function asApproval(item: InboxItemView): ApprovalItem | undefined {
  if (item.kind === 'notice' || !item.approval_id) return undefined
  const d = (item.detail ?? {}) as Record<string, unknown>
  return approvalItem(item.id, {
    approval_id: item.approval_id,
    kind: typeof d.approval_kind === 'string' ? d.approval_kind : item.kind,
    subject: d.subject,
    options: Array.isArray(d.options) ? (d.options as string[]) : [],
    ...(typeof d.expires_at === 'string' ? { expires_at: d.expires_at } : {}),
    ...(typeof d.on_timeout === 'string' ? { on_timeout: d.on_timeout } : {}),
  })
}

const KIND: Record<string, { label: string; icon: typeof InboxIcon }> = {
  approval: { label: 'Freigabe', icon: ShieldCheck },
  question: { label: 'Frage', icon: MessageCircleQuestion },
  notice: { label: 'Hinweis', icon: Gauge },
  agent: { label: 'Agent', icon: Bot },
}

function typing(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable)
}

function InboxRow({
  item,
  selected,
  denying,
  onDenying,
  onResolve,
  onOpen,
  onSelect,
}: {
  item: InboxItemView
  selected: boolean
  denying: boolean
  onDenying: (v: boolean) => void
  onResolve: (req: ResolveApprovalRequest) => void
  onOpen: () => void
  onSelect: () => void
}) {
  const card = asApproval(item)
  const waiting = item.status === 'open'
  const K = KIND[item.kind] ?? KIND.notice!
  const [details, setDetails] = useState(false)
  const [reason, setReason] = useState('')
  const [answer, setAnswer] = useState('')
  const reasonRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (denying) reasonRef.current?.focus()
  }, [denying])
  const left = card ? timeLeft(card.expiresAt, card.onTimeout) : undefined
  return (
    <li
      data-testid="inbox-item"
      data-kind={item.kind}
      data-status={item.status}
      aria-current={selected ? 'true' : undefined}
      onClick={onSelect}
      className="relative list-none"
    >
      {selected && <span aria-hidden className="absolute top-2 bottom-2 -left-2.5 w-1 rounded-full bg-foreground" title="Ausgewählt (j/k)" />}
      <div
        className={cn(
          'border-l-4 px-4 py-3',
          waiting ? 'chamfer border-signal bg-signal-soft' : 'border-transparent',
          selected && !waiting && 'border-foreground bg-accent/60',
        )}
      >
      <div className="flex flex-wrap items-center gap-x-2 text-[12px] text-muted-foreground">
        <K.icon className="size-3.5" aria-hidden />
        <span className={cn(waiting && 'font-medium text-foreground')}>{K.label}</span>
        <span aria-hidden>·</span>
        <span className="truncate">{item.session_title || item.session_id}</span>
        <span className="ml-auto shrink-0">{ago(item.created_at)}</span>
      </div>
      <div className={cn('mt-1 text-[14px]', item.kind !== 'notice' && 'font-medium')}>
        {item.kind === 'approval' ? <code className="font-mono text-[12.5px]">{item.summary}</code> : item.kind === 'question' ? item.summary : item.title}
      </div>
      {card && item.kind === 'approval' && <ApprovalReasons item={card} />}
      {waiting && left && <p className={cn('mt-1 text-[12px] text-muted-foreground', left.urgent && 'font-semibold text-warn')}>{left.text}</p>}
      {waiting && (
        <p className="mt-1 flex items-center gap-1.5 text-[12px]">
          <span className="chamfer-sm size-2 bg-signal" aria-hidden /> Die Session wartet – sie läuft weiter, sobald du antwortest.
        </p>
      )}
      {card && <ApprovalDetails item={card} open={details} onToggle={() => setDetails((v) => !v)} />}
      {item.status === 'done' && resolutionFrom(item.resolution) && (
        <p className="mt-2 flex items-center gap-1.5 text-[13px]" role="status">
          <Check className="size-3.5 text-ok" aria-hidden />
          {resolutionText(resolutionFrom(item.resolution)!, false)}
        </p>
      )}

      {waiting && item.kind === 'approval' && card && (
        <div className="mt-2.5 flex flex-wrap items-center gap-2">
          <button type="button" onClick={() => onResolve({ decision: 'allow' })} className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">
            Erlauben
          </button>
          {card.options.includes('approve_session') && (
            <button type="button" onClick={() => onResolve({ decision: 'allow', remember: 'session' })} className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px]">
              Für diese Session erlauben
            </button>
          )}
          <button type="button" onClick={() => onDenying(true)} className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px]">
            Ablehnen …
          </button>
          <button type="button" onClick={onOpen} className="rounded-md px-2 py-1.5 text-[13px] text-muted-foreground hover:text-foreground">
            Session öffnen
          </button>
          {denying && (
            <span className="flex w-full items-center gap-2 pt-1">
              <input
                ref={reasonRef}
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault()
                    onResolve(reason.trim() ? { decision: 'deny', reason: reason.trim() } : { decision: 'deny' })
                  } else if (e.key === 'Escape') onDenying(false)
                }}
                placeholder="Grund für den Agent (optional)"
                aria-label="Grund für die Ablehnung, geht an den Agent"
                className="h-8 min-w-0 flex-1 rounded-md border border-input bg-card px-2 text-[13px]"
              />
              <button
                type="button"
                onClick={() => onResolve(reason.trim() ? { decision: 'deny', reason: reason.trim() } : { decision: 'deny' })}
                className="rounded-md border border-deny/60 bg-card px-3 py-1.5 text-[13px] text-deny"
              >
                Ablehnen
              </button>
            </span>
          )}
        </div>
      )}
      {waiting && item.kind === 'question' && card && (
        <div className="mt-2.5 flex flex-col gap-2">
          {card.choices.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {card.choices.map((c) => (
                <button
                  key={c}
                  type="button"
                  aria-pressed={answer === c}
                  onClick={() => setAnswer(c)}
                  className={cn('rounded-md border bg-card px-3 py-1.5 text-[13px]', answer === c ? 'border-foreground font-medium' : 'border-foreground/30')}
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
                if (e.key === 'Enter' && answer.trim()) {
                  e.preventDefault()
                  onResolve({ decision: 'answer', answer: answer.trim() })
                }
              }}
              placeholder="Eigene Antwort …"
              aria-label="Antwort an den Agent"
              className="h-8 min-w-0 flex-1 rounded-md border border-input bg-card px-2 text-[13px]"
            />
            <button
              type="button"
              disabled={!answer.trim()}
              onClick={() => onResolve({ decision: 'answer', answer: answer.trim() })}
              className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background disabled:bg-muted disabled:text-muted-foreground"
            >
              Antworten
            </button>
          </div>
        </div>
      )}
      </div>
    </li>
  )
}

export function InboxView() {
  const [filter, setFilter] = useState<InboxFilter>('all')
  const [selected, setSelected] = useState(0)
  const [denying, setDenying] = useState<string | undefined>()
  const [error, setError] = useState<string | undefined>()
  const toggleList = useLayout((s) => s.toggleList)
  const navigate = useNavigate()
  const qc = useQueryClient()
  const data = useQuery({
    queryKey: ['inbox', filter],
    queryFn: () => client.inbox.list(query(filter)),
    refetchInterval: INBOX_POLL_MS,
  })
  const page: InboxPage | undefined = data.data
  const items = useMemo(() => page?.items ?? [], [page])
  const waiting = items.filter((i) => i.status === 'open')
  const others = items.filter((i) => i.status !== 'open')
  const ordered = [...waiting, ...others]
  const current = ordered[Math.min(selected, Math.max(0, ordered.length - 1))]

  const resolve = async (item: InboxItemView, req: ResolveApprovalRequest) => {
    setError(undefined)
    try {
      await client.inbox.resolve(item.id, req)
      setDenying(undefined)
      await qc.invalidateQueries({ queryKey: ['inbox'] })
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Entscheidung nicht übermittelt')
    }
  }
  const open = (item: InboxItemView) => void navigate({ to: '/s/$sessionId', params: { sessionId: item.session_id } })

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (typing(e.target) || e.metaKey || e.ctrlKey || e.altKey) return
      if (e.key === 'j') setSelected((s) => Math.min(s + 1, ordered.length - 1))
      else if (e.key === 'k') setSelected((s) => Math.max(s - 1, 0))
      else if (e.key === 'a' && current?.status === 'open' && current.kind === 'approval') void resolve(current, { decision: 'allow' })
      else if (e.key === 'd' && current?.status === 'open' && current.kind === 'approval') {
        e.preventDefault()
        setDenying(current.id)
      } else if (e.key === 'Enter' && current) open(current)
      else return
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex min-h-12 flex-wrap items-center gap-x-3 gap-y-1 border-b border-border px-3 py-2 sm:px-6">
        <button onClick={toggleList} className="flex size-8 items-center justify-center rounded-md hover:bg-accent lg:hidden" aria-label="Sessions anzeigen">
          <Menu className="size-4" />
        </button>
        <h1 className="type-wide text-xl font-[700]">Inbox</h1>
        <div className="flex flex-wrap gap-0.5 sm:ml-4" role="tablist" aria-label="Filter">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              role="tab"
              aria-selected={f.id === filter}
              onClick={() => {
                setFilter(f.id)
                setSelected(0)
              }}
              className={cn('rounded-md px-2.5 py-1 text-[13px]', f.id === filter ? 'bg-accent font-medium' : 'text-muted-foreground hover:text-foreground')}
            >
              {f.label}
              {f.id === 'open' && page && <span className="ml-1 text-muted-foreground tabular-nums">{page.open_count}</span>}
            </button>
          ))}
        </div>
      </header>
      {error && (
        <div className="border-b border-deny/40 bg-deny-soft px-4 py-1.5 text-[12px]" role="alert">
          {error}
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {data.isSuccess && items.length === 0 ? (
          <div className="concrete-grain flex h-full flex-col items-center justify-center gap-2 p-8 text-center">
            <Check className="size-6 text-ok" aria-hidden />
            <p className="type-wide text-lg font-[700]">Nichts wartet auf dich</p>
            <p className="max-w-sm text-sm text-muted-foreground">
              Neue Freigaben und Fragen deiner Agents erscheinen hier sofort – auch aus Sessions, die gerade niemand geöffnet hat.
            </p>
          </div>
        ) : (
          <div className="mx-auto flex max-w-3xl flex-col gap-2 px-3 py-4 sm:px-6">
            {waiting.length > 0 && <h2 className="text-[12px] font-medium text-muted-foreground">Du bist dran · {waiting.length}</h2>}
            <ul className="flex flex-col gap-2" aria-label="Wartet auf dich">
              {waiting.map((it, i) => (
                <InboxRow
                  key={it.id}
                  item={it}
                  selected={i === selected}
                  denying={denying === it.id}
                  onDenying={(v) => setDenying(v ? it.id : undefined)}
                  onResolve={(req) => void resolve(it, req)}
                  onOpen={() => open(it)}
                  onSelect={() => setSelected(i)}
                />
              ))}
            </ul>
            {others.length > 0 && (
              <>
                <h2 className="mt-4 text-[12px] font-medium text-muted-foreground">{filter === 'done' ? 'Erledigt' : 'Zur Info'} · {others.length}</h2>
                <ul className="divide-y divide-border rounded-md border border-border" aria-label="Zur Info">
                  {others.map((it, i) => (
                    <InboxRow
                      key={it.id}
                      item={it}
                      selected={waiting.length + i === selected}
                      denying={false}
                      onDenying={() => undefined}
                      onResolve={() => undefined}
                      onOpen={() => open(it)}
                      onSelect={() => setSelected(waiting.length + i)}
                    />
                  ))}
                </ul>
              </>
            )}
          </div>
        )}
      </div>
      <div className="hidden flex-wrap items-center gap-4 border-t border-border px-6 py-1.5 text-[11px] text-muted-foreground sm:flex">
        <span>
          <kbd className="font-mono">j</kbd>/<kbd className="font-mono">k</kbd> wechseln
        </span>
        <span>
          <kbd className="font-mono">a</kbd> erlauben
        </span>
        <span>
          <kbd className="font-mono">d</kbd> ablehnen
        </span>
        <span>
          <kbd className="font-mono">↵</kbd> Session öffnen
        </span>
        <span className="ml-auto">Zähler = offene Freigaben und Fragen</span>
      </div>
    </div>
  )
}
