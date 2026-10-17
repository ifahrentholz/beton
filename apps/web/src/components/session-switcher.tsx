import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { useNavigate, useRouterState } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import type { SessionSummary } from '@beton/sdk'
import { Search } from 'lucide-react'
import { client } from '@/lib/client'
import { shortcutOf, command, useKeymap } from '@/lib/commands'
import { ago } from '@/lib/format'
import { fuzzyScore } from '@/lib/fuzzy'
import { useT } from '@/lib/i18n'
import { chordText } from '@/lib/keys'
import { mruOrder, useMru } from '@/lib/mru'
import { PLACEHOLDER } from '@/lib/title'
import { cn } from '@/lib/utils'
import { useSessions } from '@/store/sessions'
import { HarnessBadge, listStatus, StatusMark } from './harness'
import { Overlay } from './overlay'
import { useUi } from './ui-state'

/** Offene Freigabe bzw. Frage je Session aus der Inbox (UX-003 AC2). */
export type Waiting = 'approval' | 'question'

export function waitingOf(items: readonly { session_id: string; kind: string; status: string }[]): Map<string, Waiting> {
  const out = new Map<string, Waiting>()
  for (const it of items) {
    if (it.status !== 'open') continue
    if (it.kind === 'approval') out.set(it.session_id, 'approval')
    else if (it.kind === 'question' && !out.has(it.session_id)) out.set(it.session_id, 'question')
  }
  return out
}

/**
 * Einträge des Switchers: zuletzt genutzte Sessions zuerst (je Gerät, AC3), danach die übrigen
 * nach Aktivität; mit Suche nur Treffer im Titel.
 */
export function switcherRows(sessions: SessionSummary[], mru: string[], query: string): SessionSummary[] {
  const list = mruOrder(
    sessions.filter((s) => !s.archived),
    mru,
  )
  const q = query.trim()
  if (!q) return list
  return list.filter((s) => fuzzyScore(q, s.title || PLACEHOLDER) > 0)
}

/**
 * Session-Switcher (UX-003, Entwurf `inbox-switcher`): MRU-Liste mit Status, offene Freigaben und
 * Fragen markiert, Suche. Der Wechsel zeigt die Session sofort aus dem lokalen Speicher und holt
 * ab der letzten `seq` nach (`useSessionStream`). In der Desktop-App schaltet `Ctrl+Tab` durch:
 * Jeder Tab-Anschlag rückt eins weiter, Loslassen von Ctrl wechselt.
 */
export function SessionSwitcher() {
  const mode = useUi((s) => s.switcher)
  const set = useUi((s) => s.set)
  useCycleKeys()
  if (!mode) return null
  return <SwitcherDialog cycle={mode === 'cycle'} onClose={() => set({ switcher: false })} />
}

/** `Ctrl+Tab` öffnet im Durchschalt-Modus (im Browser fängt der Browser die Taste ab). */
function useCycleKeys() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Tab' || !e.ctrlKey || e.metaKey || e.altKey) return
      if (useUi.getState().switcher) return
      e.preventDefault()
      useUi.getState().set({ palette: false, shortcuts: false, switcher: 'cycle' })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}

function SwitcherDialog({ cycle, onClose }: { cycle: boolean; onClose: () => void }) {
  const t = useT()
  const navigate = useNavigate()
  const byId = useSessions((s) => s.byId)
  const mru = useMru((s) => s.ids)
  const overrides = useKeymap((s) => s.overrides)
  const path = useRouterState({ select: (s) => s.location.pathname })
  const currentId = /^\/s\/([^/]+)/.exec(path)?.[1]
  const [query, setQuery] = useState('')
  const inbox = useQuery({ queryKey: ['inbox', 'switcher'], queryFn: () => client.inbox.list({ status: 'open', limit: 200 }) })
  const waiting = useMemo(() => waitingOf(inbox.data?.items ?? []), [inbox.data])
  const rows = useMemo(() => switcherRows(Object.values(byId), mru, query), [byId, mru, query])
  // Ohne Suche ist die aktuelle Session oben; vorausgewählt ist die davor genutzte.
  const initial = !query && rows[0]?.id === currentId && rows.length > 1 ? 1 : 0
  const [selected, setSelected] = useState(initial)
  useEffect(() => setSelected(!query && rows[0]?.id === currentId && rows.length > 1 ? 1 : 0), [query])
  const current = Math.min(selected, Math.max(0, rows.length - 1))
  const listId = useId()
  const listRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    listRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' })
  }, [current])

  const go = (s: SessionSummary | undefined) => {
    onClose()
    if (s && s.id !== currentId) void navigate({ to: '/s/$sessionId', params: { sessionId: s.id } })
  }
  const step = (by: number) => rows.length && setSelected((current + by + rows.length) % rows.length)

  // Durchschalten: Tab/⇧Tab mit gehaltenem Ctrl, Loslassen wechselt.
  const latest = useRef({ rows, current })
  latest.current = { rows, current }
  useEffect(() => {
    if (!cycle) return
    const down = (e: KeyboardEvent) => {
      if (e.key === 'Tab' && e.ctrlKey) {
        e.preventDefault()
        e.stopPropagation()
        const { rows: r, current: c } = latest.current
        if (r.length) setSelected((c + (e.shiftKey ? -1 : 1) + r.length) % r.length)
      }
    }
    const up = (e: KeyboardEvent) => {
      if (e.key === 'Control') go(latest.current.rows[latest.current.current])
    }
    window.addEventListener('keydown', down, true)
    window.addEventListener('keyup', up, true)
    return () => {
      window.removeEventListener('keydown', down, true)
      window.removeEventListener('keyup', up, true)
    }
  }, [cycle])

  const openChord = command('switcher.open')
  const optionId = (i: number) => `${listId}-o${i}`
  return (
    <Overlay label={t('switcher.label')} onClose={onClose} testId="session-switcher" className="w-[560px]">
      <div className="overflow-hidden rounded-lg border border-border bg-popover shadow-2xl">
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <Search aria-hidden className="size-4 text-muted-foreground" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t('switcher.placeholder')}
            aria-label={t('switcher.placeholder')}
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={rows.length ? optionId(current) : undefined}
            autoComplete="off"
            spellCheck={false}
            className="min-w-0 flex-1 bg-transparent text-[14px] outline-hidden placeholder:text-muted-foreground"
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                e.preventDefault()
                step(e.key === 'ArrowDown' ? 1 : -1)
              } else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                e.preventDefault()
                go(rows[current])
              } else if (e.key === 'Tab' && !e.ctrlKey) {
                e.preventDefault()
              }
            }}
          />
          <span className="text-[11px] text-muted-foreground">{t('switcher.recent')}</span>
        </div>
        <div ref={listRef} id={listId} role="listbox" aria-label={t('switcher.label')} className="max-h-[min(420px,60dvh)] overflow-y-auto py-1">
          {rows.length === 0 && query.trim() && (
            // Keine Treffer (Entwurf `inbox-switcher`, Zustand `empty`), wie in der Befehlspalette.
            <div className="px-3 py-6 text-center" role="presentation">
              <div className="text-[13px]">{t('switcher.empty', { q: query.trim() })}</div>
              <div className="mt-1 text-[12px] text-muted-foreground">{t('switcher.emptyHint')}</div>
            </div>
          )}
          {rows.map((s, i) => {
            const wait = waiting.get(s.id) ?? (s.status === 'waiting_approval' ? 'approval' : undefined)
            const active = i === current
            return (
              <div
                key={s.id}
                id={optionId(i)}
                role="option"
                aria-selected={active}
                data-session={s.id}
                data-waiting={wait}
                onMouseMove={() => !active && setSelected(i)}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => go(s)}
                className={cn('flex cursor-default items-center gap-3 border-l-2 px-3 py-2', active ? 'border-signal bg-accent' : 'border-transparent')}
              >
                <StatusMark status={wait ? 'waiting' : listStatus(s.status)} />
                <div className="min-w-0 flex-1">
                  <div className={cn('truncate text-[13px]', s.unread && 'font-semibold', !s.title && 'text-muted-foreground italic')}>{s.title || PLACEHOLDER}</div>
                  <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                    <HarnessBadge harness={s.harness} className="text-[11px]" />
                    {s.kind === 'async' && <span>· {t('switcher.background')}</span>}
                  </div>
                </div>
                {wait && (
                  <span className="chamfer-sm bg-signal px-1.5 py-0.5 text-[11px] font-medium text-signal-foreground">
                    {wait === 'question' ? t('switcher.question') : t('switcher.approval')}
                  </span>
                )}
                {!wait && s.unread && <span className="text-[11px] text-muted-foreground">{t('switcher.unread')}</span>}
                <span className="min-w-16 text-right text-[11px] whitespace-nowrap text-muted-foreground">{ago(s.last_activity_at)}</span>
              </div>
            )
          })}
        </div>
        <div className="flex flex-wrap gap-x-4 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
          <span>{t('switcher.open', { key: chordText((openChord && shortcutOf(openChord, overrides)) ?? 'Mod+Alt+S') })}</span>
          <span>{t('switcher.select')}</span>
          <span>{t('switcher.switch')}</span>
          <span>{t('switcher.cycle')}</span>
          <span className="ml-auto">{t('switcher.cache')}</span>
        </div>
      </div>
    </Overlay>
  )
}
