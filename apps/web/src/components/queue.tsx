import { useEffect, useState } from 'react'
import type { QueueItem } from '@beton/sdk'
import { client } from '@/lib/client'
import { authorLabel, moved } from '@/lib/queue'
import { cn } from '@/lib/utils'
import { harnessName } from './harness'

/**
 * Serverseitige Queue über dem Composer (WEB-005, Screen `session-stream`): Zeilen
 * „Eingereiht n“ mit „Jetzt lenken“ (Steer), „Bearbeiten“ und „Entfernen“; die Reihenfolge
 * lässt sich per Drag & Drop ändern. Den Autor zeigt die Zeile, sobald mehrere Personen
 * eingereiht haben (`collab-codrive`). Änderungen gehen an den Server und kommen als
 * `queue.updated` bei allen Clients an.
 */
export function QueueList({
  sessionId,
  items,
  harness,
  steering,
  running,
  onError,
}: {
  sessionId: string
  items: QueueItem[]
  harness: string
  /** Capability `steering` des Harness (HAR-002). */
  steering: boolean
  running: boolean
  onError: (message: string) => void
}) {
  // Sofortige Anzeige nach dem Ablegen, bis der Server-Stand eintrifft.
  const [order, setOrder] = useState<QueueItem[] | undefined>()
  useEffect(() => setOrder(undefined), [items])
  const [dragging, setDragging] = useState<string | undefined>()
  const [editing, setEditing] = useState<{ id: string; text: string } | undefined>()
  const shown = order ?? items
  if (shown.length === 0) return null
  const authors = new Set(shown.map((i) => i.author))
  const session = client.session(sessionId)
  const fail = (e: unknown) => onError(e instanceof Error ? e.message : 'Aktion fehlgeschlagen')
  const steerHint = steering
    ? undefined
    : `${harnessName(harness)} kann nicht in einen laufenden Turn eingreifen. Der Eintrag läuft, sobald der Turn fertig ist.`
  const drop = (target: number) => {
    const id = dragging
    setDragging(undefined)
    if (!id) return
    const from = shown.findIndex((i) => i.id === id)
    if (from < 0 || from === target) return
    setOrder(moved(shown, id, target))
    session.moveQueued(id, target).catch(fail)
  }
  const save = () => {
    if (!editing) return
    const { id, text } = editing
    setEditing(undefined)
    if (text.trim()) session.editQueued(id, text.trim()).catch(fail)
  }
  return (
    <div className="mb-2 flex flex-col gap-1" aria-label="Eingereihte Nachrichten" data-testid="queue">
      {shown.map((it, i) => (
        <div
          key={it.id}
          data-testid="queue-row"
          data-item={it.id}
          draggable={!editing}
          onDragStart={(e) => {
            setDragging(it.id)
            e.dataTransfer.effectAllowed = 'move'
            e.dataTransfer.setData('text/plain', it.id)
          }}
          onDragEnd={() => setDragging(undefined)}
          onDragOver={(e) => {
            e.preventDefault()
            e.dataTransfer.dropEffect = 'move'
          }}
          onDrop={(e) => {
            e.preventDefault()
            drop(i)
          }}
          className={cn(
            'flex items-center gap-2 rounded-md border border-dashed border-border px-2 py-1 text-[12px]',
            !editing && 'cursor-grab',
            dragging === it.id && 'opacity-50',
          )}
        >
          <span className="shrink-0 text-muted-foreground">Eingereiht {i + 1}</span>
          {editing?.id === it.id ? (
            <input
              autoFocus
              aria-label="Eingereihte Nachricht bearbeiten"
              value={editing.text}
              onChange={(e) => setEditing({ id: it.id, text: e.target.value })}
              onKeyDown={(e) => {
                if (e.key === 'Enter') save()
                if (e.key === 'Escape') {
                  e.stopPropagation()
                  setEditing(undefined)
                }
              }}
              onBlur={save}
              className="min-w-0 flex-1 rounded-sm border border-input bg-card px-1 outline-none"
            />
          ) : (
            <span className="min-w-0 truncate" title={it.text}>
              {authors.size > 1 && <span className="text-muted-foreground">{authorLabel(it.author)}: </span>}
              {it.text}
            </span>
          )}
          <span className="ml-auto flex shrink-0 gap-1">
            <span title={steerHint}>
              <button
                className="rounded-sm px-1.5 text-[11px] hover:bg-accent disabled:cursor-default disabled:opacity-50 disabled:hover:bg-transparent"
                disabled={!steering || !running}
                aria-describedby={steering ? undefined : `steer-hint-${it.id}`}
                onClick={() => session.steerQueued(it.id).catch(fail)}
              >
                Jetzt lenken
              </button>
              {steerHint && (
                <span id={`steer-hint-${it.id}`} className="sr-only">
                  {steerHint}
                </span>
              )}
            </span>
            <button className="rounded-sm px-1.5 text-[11px] hover:bg-accent" onClick={() => setEditing({ id: it.id, text: it.text })}>
              Bearbeiten
            </button>
            <button className="rounded-sm px-1.5 text-[11px] hover:bg-accent" onClick={() => session.deleteQueued(it.id).catch(fail)}>
              Entfernen
            </button>
          </span>
        </div>
      ))}
    </div>
  )
}
