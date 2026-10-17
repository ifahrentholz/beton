import { useEffect, useId, useMemo, useRef, useState, type ComponentType, type ReactNode } from 'react'
import { useNavigate, useRouterState } from '@tanstack/react-router'
import { Search } from 'lucide-react'
import { available, commandHint, commandTitle, commands as allCommands, shortcutOf, useKeymap, useRegistry, type Command, type PaletteGroup } from '@/lib/commands'
import { fuzzyScore } from '@/lib/fuzzy'
import { useT, type MessageKey } from '@/lib/i18n'
import { chordText } from '@/lib/keys'
import { useMru, mruOrder } from '@/lib/mru'
import { PLACEHOLDER } from '@/lib/title'
import { cn } from '@/lib/utils'
import { useSessions } from '@/store/sessions'
import { listStatus, StatusMark } from './harness'
import { Overlay } from './overlay'
import { useUi } from './ui-state'

/** Sessions in „Gehe zu“ ohne Suchbegriff; mit Suche alle Treffer bis zu dieser Zahl. */
const SESSIONS_IDLE = 3
const SESSIONS_MAX = 30

const GROUPS: { id: PaletteGroup; key: MessageKey }[] = [
  { id: 'session', key: 'palette.group.session' },
  { id: 'general', key: 'palette.group.general' },
  { id: 'goto', key: 'palette.group.goto' },
]

/** Ein Eintrag der Palette: Befehl aus der Registry oder Session aus der Liste. */
export interface PaletteItem {
  id: string
  group: PaletteGroup
  title: string
  hint?: string | undefined
  icon?: ReactNode
  shortcut?: string | undefined
  score: number
  order: number
  run: () => void
}

/**
 * Sichtbare Einträge (UX-002): verfügbare Befehle (`when` erfüllt, AC2) und Sessions, gefiltert
 * und sortiert nach Treffergüte. Gruppen erscheinen in fester Reihenfolge, bei einer Suche nach
 * ihrem besten Treffer; der erste Eintrag ist der beste Treffer (AC1).
 */
export function paletteItems(query: string, items: Omit<PaletteItem, 'score'>[], keywords: (id: string) => readonly string[] = () => []): { group: PaletteGroup; items: PaletteItem[] }[] {
  const q = query.trim()
  const scored = items
    .map((it) => ({ ...it, score: fuzzyScore(q, it.title, keywords(it.id)) }))
    .filter((it) => it.score > 0)
  const groups = GROUPS.map((g) => ({
    group: g.id,
    items: scored.filter((it) => it.group === g.id).sort((a, b) => (q ? b.score - a.score : 0) || a.order - b.order),
  })).filter((g) => g.items.length > 0)
  if (q) groups.sort((a, b) => (b.items[0]?.score ?? 0) - (a.items[0]?.score ?? 0))
  return groups
}

function iconOf(c: Command): ReactNode {
  const Icon = c.icon as ComponentType<{ className?: string }> | undefined
  return Icon ? <Icon /> : null
}

export function CommandPalette() {
  const t = useT()
  const open = useUi((s) => s.palette)
  const set = useUi((s) => s.set)
  if (!open) return null
  return <PaletteDialog onClose={() => set({ palette: false })} t={t} />
}

function PaletteDialog({ onClose, t }: { onClose: () => void; t: ReturnType<typeof useT> }) {
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState(0)
  const navigate = useNavigate()
  const version = useRegistry((s) => s.version)
  const overrides = useKeymap((s) => s.overrides)
  const byId = useSessions((s) => s.byId)
  const mru = useMru((s) => s.ids)
  const path = useRouterState({ select: (s) => s.location.pathname })
  const currentId = /^\/s\/([^/]+)/.exec(path)?.[1]
  const listId = useId()
  const listRef = useRef<HTMLDivElement>(null)

  const run = (fn: () => void) => {
    onClose()
    // Nach dem Schließen: Der Fokus ist zurück, der Befehl kann ihn weitergeben.
    setTimeout(fn, 0)
  }

  const groups = useMemo(() => {
    const cmds = allCommands().filter((c) => c.palette !== false && c.group && available(c))
    const kw = new Map(cmds.map((c) => [c.id, c.keywords ?? []]))
    const items: Omit<PaletteItem, 'score'>[] = cmds.map((c) => ({
      id: c.id,
      group: c.group!,
      title: commandTitle(c, t),
      hint: commandHint(c, t),
      icon: iconOf(c),
      shortcut: shortcutOf(c, overrides),
      order: c.order ?? 50,
      run: () => run(() => c.run?.({})),
    }))
    // Andere Sessions (die geöffnete steht nicht unter „Gehe zu“).
    const sessions = mruOrder(
      Object.values(byId).filter((s) => !s.archived && s.id !== currentId),
      mru,
    )
    const q = query.trim()
    sessions.forEach((s, i) => {
      if (!q && i >= SESSIONS_IDLE) return
      items.push({
        id: `session:${s.id}`,
        group: 'goto',
        title: s.title || PLACEHOLDER,
        icon: <StatusMark status={listStatus(s.status)} />,
        order: 100 + i,
        run: () => run(() => void navigate({ to: '/s/$sessionId', params: { sessionId: s.id } })),
      })
    })
    const out = paletteItems(query, items, (id) => kw.get(id) ?? [])
    // Lange Session-Listen kürzen.
    for (const g of out) if (g.group === 'goto') g.items = g.items.slice(0, SESSIONS_MAX)
    return out
    // `version`: Registry geändert (Befehle kamen oder gingen, Bedingungen wechselten).
  }, [query, version, overrides, byId, mru, t, currentId])

  const flat = groups.flatMap((g) => g.items)
  const current = Math.min(selected, Math.max(0, flat.length - 1))
  useEffect(() => setSelected(0), [query])
  useEffect(() => {
    listRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' })
  }, [current, query])

  const optionId = (i: number) => `${listId}-o${i}`
  let index = -1
  return (
    <Overlay label={t('palette.label')} onClose={onClose} testId="command-palette" className="w-[600px]">
      <div className="flex flex-col overflow-hidden rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-2xl">
        <div className="p-1 pb-0">
          <div className="relative flex h-8 w-full min-w-0 items-center rounded-lg border border-input/30 bg-input/30 transition-colors has-[input:focus-visible]:border-ring has-[input:focus-visible]:ring-3 has-[input:focus-visible]:ring-ring/50">
            <span className="flex items-center justify-center py-1.5 pl-2 text-muted-foreground">
              <Search aria-hidden className="size-4 shrink-0 opacity-50" />
            </span>
            <input
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t('palette.placeholder')}
              aria-label={t('palette.label')}
              role="combobox"
              aria-expanded="true"
              aria-controls={listId}
              aria-autocomplete="list"
              aria-activedescendant={flat.length ? optionId(current) : undefined}
              autoComplete="off"
              spellCheck={false}
              className="w-full bg-transparent pl-1.5 text-sm outline-hidden placeholder:text-muted-foreground"
              onKeyDown={(e) => {
                if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                  e.preventDefault()
                  if (flat.length === 0) return
                  const step = e.key === 'ArrowDown' ? 1 : -1
                  setSelected((current + step + flat.length) % flat.length)
                } else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                  e.preventDefault()
                  flat[current]?.run()
                } else if (e.key === 'Tab') {
                  // Nur ein Eingabefeld: Tab bleibt in der Palette.
                  e.preventDefault()
                }
              }}
            />
          </div>
        </div>
        <div ref={listRef} id={listId} role="listbox" aria-label={t('palette.label')} className="max-h-[420px] scroll-py-1 overflow-x-hidden overflow-y-auto">
          {flat.length === 0 && (
            <div className="py-6 text-center text-sm" role="presentation">
              <div className="text-[13px]">{t('palette.empty', { q: query })}</div>
              <div className="mt-1 text-[12px] text-muted-foreground">{t('palette.emptyHint')}</div>
            </div>
          )}
          {groups.map((g) => {
            const heading = `${listId}-${g.group}`
            return (
              <div key={g.group} role="group" aria-labelledby={heading} className="overflow-hidden p-1 text-foreground">
                <div id={heading} role="presentation" className="px-2 py-1.5 text-xs font-medium text-muted-foreground">
                  {t(GROUPS.find((x) => x.id === g.group)!.key)}
                </div>
                {g.items.map((it) => {
                  index += 1
                  const i = index
                  const active = i === current
                  return (
                    <div
                      key={it.id}
                      id={optionId(i)}
                      role="option"
                      aria-selected={active}
                      data-selected={active}
                      data-command={it.id}
                      onMouseMove={() => !active && setSelected(i)}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => it.run()}
                      className={cn(
                        'group/item relative flex cursor-default items-center gap-2 rounded-sm px-2 py-1.5 text-sm select-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*="size-"])]:size-4',
                        active && 'bg-muted text-foreground',
                      )}
                    >
                      {it.icon}
                      <span className="truncate">{it.title}</span>
                      {it.hint && <span className="truncate text-muted-foreground">{it.hint}</span>}
                      {it.shortcut && (
                        <span className={cn('ml-auto shrink-0 text-xs tracking-widest text-muted-foreground', active && 'text-foreground')}>{chordText(it.shortcut)}</span>
                      )}
                    </div>
                  )
                })}
              </div>
            )
          })}
        </div>
        <div className="flex flex-wrap gap-x-4 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
          <span>{t('palette.select')}</span>
          <span>{t('palette.run')}</span>
          <span>{t('palette.close')}</span>
          <span className="ml-auto">{t('palette.global', { key: chordText(paletteChord(overrides)) })}</span>
        </div>
      </div>
    </Overlay>
  )
}

function paletteChord(overrides: Record<string, string>): string {
  return overrides['palette.open'] ?? 'Mod+K'
}
