import { useEffect, useRef, useState } from 'react'
import { useCommand } from '@/lib/commands'
import { PLACEHOLDER, type TitleSource } from '@/lib/title'
import { cn } from '@/lib/utils'

/**
 * Titel im Kopf der Session (UX-009, Prototyp `inbox-titles`): „Neue Session“ als Platzhalter,
 * erzeugte Titel ohne Layout-Sprung, Umbenennen per Doppelklick oder `F2`.
 */
export function SessionTitle({ title, source, onRename }: { title: string; source?: TitleSource | undefined; onRename: (title: string) => Promise<void> }) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(title)
  const [error, setError] = useState<string | undefined>()
  const input = useRef<HTMLInputElement>(null)
  const start = () => {
    setDraft(title)
    setError(undefined)
    setEditing(true)
  }
  // `F2` aus der Befehls-Registry (UX-004); im Eingabefeld gehört die Taste dem Feld.
  useCommand({ id: 'session.rename', when: () => !editing, run: start })
  useEffect(() => {
    if (editing) input.current?.select()
  }, [editing])
  const commit = () => {
    const next = draft.trim()
    if (!next || next === title) {
      setEditing(false)
      return
    }
    onRename(next)
      .then(() => setEditing(false))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : 'Umbenennen fehlgeschlagen'))
  }
  if (editing) {
    return (
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <input
          ref={input}
          value={draft}
          maxLength={200}
          aria-label="Titel der Session"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              commit()
            } else if (e.key === 'Escape') {
              e.preventDefault()
              setEditing(false)
            }
          }}
          onBlur={() => setEditing(false)}
          className="h-8 w-[360px] max-w-full min-w-0 rounded-md border-2 border-ring bg-card px-2 text-[15px] font-semibold outline-none"
        />
        <span className="hidden shrink-0 text-[11px] text-muted-foreground md:inline" role={error ? 'alert' : undefined}>
          {error ?? '↵ übernehmen · Esc abbrechen · danach kein automatischer Titel mehr'}
        </span>
      </div>
    )
  }
  return (
    <div className="flex min-w-0 flex-1 items-center gap-3">
      <h2
        key={title}
        onDoubleClick={start}
        title={title ? `${title} – Doppelklick oder F2 zum Umbenennen` : 'Doppelklick oder F2 zum Umbenennen'}
        className={cn(
          'min-w-0 cursor-text truncate text-[15px] font-semibold',
          !title && 'font-normal text-muted-foreground italic',
          title && source === 'generated' && 'animate-title-in',
        )}
      >
        {title || PLACEHOLDER}
      </h2>
      {title && source === 'generated' && (
        <span className="hidden shrink-0 text-[11px] text-muted-foreground sm:inline" title="Aus der ersten Nachricht erzeugt. Doppelklick oder F2 zum Umbenennen.">
          automatisch
        </span>
      )}
      {title && source === 'user' && <span className="hidden shrink-0 text-[11px] text-muted-foreground sm:inline">von dir benannt</span>}
    </div>
  )
}
