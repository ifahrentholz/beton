import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { CircleAlert, Keyboard, Search, X } from 'lucide-react'
import { commandTitle, commands as allCommands, rebind, shortcutOf, suspendShortcuts, swapShortcut, useKeymap, useRegistry, type Command, type Section } from '@/lib/commands'
import { fuzzyScore } from '@/lib/fuzzy'
import { useT, type MessageKey, type T } from '@/lib/i18n'
import { assignable, chordText, isModifierKey, strokeOf } from '@/lib/keys'
import { cn } from '@/lib/utils'
import { Keys, rich } from './kbd'
import { Overlay } from './overlay'
import { useUi } from './ui-state'

const SECTIONS: { id: Section; key: MessageKey }[] = [
  { id: 'general', key: 'shortcuts.section.general' },
  { id: 'session', key: 'shortcuts.section.session' },
  { id: 'approvals', key: 'shortcuts.section.approvals' },
  { id: 'inbox', key: 'shortcuts.section.inbox' },
]

/** Zeile des Overlays: jeder Befehl der Registry mit Kürzel (UX-004 AC1). */
export interface ShortcutRow {
  id: string
  section: Section
  title: string
  chord: string
  local: boolean
}

export function shortcutRows(cmds: Command[], overrides: Record<string, string>, t: T, query = ''): ShortcutRow[] {
  // Reihenfolge der Deklaration (`lib/keymap`), wie im Entwurf.
  return cmds
    .filter((c) => c.shortcut && c.section)
    .map((c) => ({ id: c.id, section: c.section!, title: commandTitle(c, t), chord: shortcutOf(c, overrides) ?? '', local: !!c.local }))
    .filter((r) => !query.trim() || fuzzyScore(query, `${r.title} ${chordText(r.chord)}`) > 0)
}

/**
 * Tastenkürzel (UX-004, Entwurf `inbox-shortcuts`): alle Kürzel der Registry, gruppiert und
 * durchsuchbar. „Ändern“ nimmt die nächste Tastenkombination auf; kollidiert sie, fragt das
 * Overlay, ob getauscht werden soll (AC2). Feste Tasten (z. B. `↵` Senden) lassen sich nicht ändern.
 */
export function ShortcutsOverlay() {
  const open = useUi((s) => s.shortcuts)
  const set = useUi((s) => s.set)
  if (!open) return null
  return <ShortcutsDialog onClose={() => set({ shortcuts: false })} />
}

function ShortcutsDialog({ onClose }: { onClose: () => void }) {
  const t = useT()
  const version = useRegistry((s) => s.version)
  const overrides = useKeymap((s) => s.overrides)
  const [query, setQuery] = useState('')
  const [editing, setEditing] = useState<string | undefined>()
  const [conflict, setConflict] = useState<{ id: string; chord: string; other: Command } | undefined>()
  const titleId = useId()
  const swapRef = useRef<HTMLButtonElement>(null)
  const rows = useMemo(() => shortcutRows(allCommands(), overrides, t, query), [version, overrides, t, query])

  // Aufnahme: die nächste Tastenkombination mit Modifier bzw. Funktionstaste.
  useEffect(() => {
    if (!editing) return
    const onKey = (e: KeyboardEvent) => {
      if (conflict) {
        // Im Konflikt bedienen Tab und Enter die Knöpfe; Esc bricht ab.
        if (e.key === 'Escape') {
          e.preventDefault()
          e.stopPropagation()
          cancel()
        }
        return
      }
      if (isModifierKey(e.key)) return
      e.preventDefault()
      e.stopPropagation()
      if (e.key === 'Escape') {
        cancel()
        return
      }
      const chord = strokeOf(e)
      if (!assignable(chord)) return
      const res = rebind(editing, chord)
      if (res.ok) cancel()
      else if (res.reason === 'conflict') setConflict({ id: editing, chord, other: res.with })
    }
    const resume = suspendShortcuts()
    window.addEventListener('keydown', onKey, true)
    return () => {
      resume()
      window.removeEventListener('keydown', onKey, true)
    }
  }, [editing, conflict])
  useEffect(() => {
    if (conflict) swapRef.current?.focus()
  }, [conflict])

  const searchRef = useRef<HTMLInputElement>(null)
  const cancel = () => {
    setConflict(undefined)
    setEditing(undefined)
    // Der Konflikt-Hinweis verschwindet: Fokus zurück ins Overlay.
    requestAnimationFrame(() => searchRef.current?.focus())
  }
  const editingCmd = editing ? allCommands().find((c) => c.id === editing) : undefined

  return (
    <Overlay labelledBy={titleId} onClose={onClose} testId="shortcuts-overlay" className="w-[720px]" escape={!editing}>
      <div className="flex max-h-[calc(100dvh-6rem)] flex-col overflow-hidden rounded-lg border border-border bg-popover shadow-2xl">
        <div className="flex items-center gap-3 border-b border-border px-4 py-2.5">
          <Keyboard aria-hidden className="size-4 shrink-0" />
          <h2 id={titleId} className="text-[15px] font-semibold whitespace-nowrap">
            {t('shortcuts.title')}
          </h2>
          <label className="ml-3 flex h-7 min-w-0 flex-1 items-center gap-2 rounded-md border border-input bg-card px-2 text-[12px] text-muted-foreground">
            <Search aria-hidden className="size-3.5 shrink-0" />
            <input
              ref={searchRef}
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t('shortcuts.search')}
              aria-label={t('shortcuts.search')}
              className="min-w-0 flex-1 bg-transparent text-foreground outline-hidden placeholder:text-muted-foreground"
            />
          </label>
          <button onClick={onClose} aria-label={t('shortcuts.close')} className="text-muted-foreground hover:text-foreground">
            <X className="size-4" />
          </button>
        </div>
        <div className="grid min-h-0 gap-x-8 overflow-y-auto px-4 py-3 sm:grid-cols-2" data-testid="shortcut-list">
          {SECTIONS.map((sec) => {
            const items = rows.filter((r) => r.section === sec.id)
            if (items.length === 0) return null
            return (
              <div key={sec.id} className="mb-3" role="group" aria-label={t(sec.key)}>
                <div className="mb-1 text-[12px] font-medium text-muted-foreground">{t(sec.key)}</div>
                {items.map((r) => {
                  const isEditing = editing === r.id
                  const shown = isEditing && conflict ? conflict.chord : r.chord
                  return (
                    <div
                      key={r.id}
                      data-shortcut={r.id}
                      className={cn('group flex items-center gap-2 border-b border-border/60 py-1 text-[13px]', isEditing && 'bg-accent')}
                    >
                      <span className="flex-1">{r.title}</span>
                      {isEditing ? (
                        <span className="rounded-sm border-2 border-ring px-1 font-mono text-[11px]" data-testid="shortcut-recording">
                          {chordText(shown)}
                        </span>
                      ) : (
                        <>
                          {!r.local && (
                            <button
                              onClick={() => {
                                setConflict(undefined)
                                setEditing(r.id)
                              }}
                              aria-label={t('shortcuts.changeLabel', { name: r.title })}
                              className="text-[11px] text-muted-foreground opacity-0 group-hover:opacity-100 hover:text-foreground focus-visible:opacity-100"
                            >
                              {t('shortcuts.change')}
                            </button>
                          )}
                          <Keys chord={r.chord} />
                        </>
                      )}
                    </div>
                  )
                })}
              </div>
            )
          })}
        </div>
        <div aria-live="polite" className="sr-only">
          {editingCmd && !conflict ? t('shortcuts.recording', { name: commandTitle(editingCmd, t) }) : ''}
        </div>
        {conflict && editingCmd && (
          <div className="mx-4 mb-3 flex flex-wrap items-center gap-3 rounded-md border border-border bg-card p-2.5 text-[13px]" role="alert" data-testid="shortcut-conflict">
            <CircleAlert aria-hidden className="size-4 shrink-0" />
            <span className="flex-1">
              {rich(t('shortcuts.conflict'), {
                keys: <Keys chord={conflict.chord} />,
                other: commandTitle(conflict.other, t),
                old: <Keys chord={shortcutOf(editingCmd, overrides) ?? ''} />,
              })}
            </span>
            {!conflict.other.local && (
              <button
                ref={swapRef}
                onClick={() => {
                  swapShortcut(conflict.id, conflict.chord)
                  cancel()
                }}
                className="rounded-md bg-foreground px-3 py-1 text-[12px] font-semibold text-background"
              >
                {t('shortcuts.swap')}
              </button>
            )}
            <button onClick={cancel} className="rounded-md border border-border px-3 py-1 text-[12px]">
              {t('shortcuts.cancel')}
            </button>
          </div>
        )}
        <div className="border-t border-border px-4 py-1.5 text-[11px] text-muted-foreground">{t('shortcuts.footer')}</div>
      </div>
    </Overlay>
  )
}
