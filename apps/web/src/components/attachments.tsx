import { useEffect, useState } from 'react'
import type { Attachment } from '@beton/sdk'
import { FileText, Image, LoaderCircle, X } from 'lucide-react'
import { client } from '@/lib/client'
import { caption, kindOf, type AttachmentKind } from '@/lib/composer'

/** Ein Anhang im Composer: lokal ausgewählt, hochgeladen oder gescheitert. */
export interface DraftAttachment {
  key: string
  name: string
  mime: string
  size: number
  kind: AttachmentKind
  /** Vorschau eines Bildes (Object-URL der lokalen Datei). */
  preview?: string | undefined
  /** Referenz nach dem Upload. */
  uploaded?: Attachment | undefined
  error?: string | undefined
}

/** Abgelehnte Datei mit Grund (Prototyp: „… nicht angehängt: …“). */
export interface Rejected {
  name: string
  reason: string
}

function Remove({ name, onRemove }: { name: string; onRemove: () => void }) {
  return (
    <button
      type="button"
      onClick={onRemove}
      className="absolute top-1 right-1 flex size-4 items-center justify-center rounded-full bg-foreground text-background"
      aria-label={`Anhang ${name} entfernen`}
      title="Anhang entfernen"
    >
      <X className="size-2.5" />
    </button>
  )
}

/** Vorschauen über dem Eingabefeld (WEB-006, Prototyp `workspace-composer`). */
export function AttachmentStrip({
  items,
  rejected,
  maxFiles,
  onRemove,
}: {
  items: DraftAttachment[]
  rejected: Rejected[]
  maxFiles: number
  onRemove: (key: string) => void
}) {
  if (items.length === 0 && rejected.length === 0) return null
  return (
    <div className="flex flex-wrap items-end gap-2 px-1 pb-2" data-testid="attachment-strip">
      {items.map((a, i) => {
        const last = i === items.length - 1
        const busy = !a.uploaded && !a.error
        return (
          <div key={a.key} data-testid="attachment" data-state={a.error ? 'error' : busy ? 'uploading' : 'ready'}>
            {a.kind === 'image' ? (
              <div className="relative">
                <div className="flex h-16 w-24 items-center justify-center overflow-hidden rounded-md border border-border bg-sunken">
                  {a.preview ? <img src={a.preview} alt={`Vorschau ${a.name}`} className="size-full object-cover" /> : <Image className="size-5 text-muted-foreground" />}
                </div>
                <Remove name={a.name} onRemove={() => onRemove(a.key)} />
                <div className="mt-0.5 flex max-w-48 items-center gap-1 text-[11px] text-muted-foreground">
                  {busy ? <LoaderCircle className="size-3 shrink-0 animate-spin" /> : <Image className="size-3 shrink-0" />}
                  <span className="truncate">{caption('image', a.name, a.size)}</span>
                </div>
                {last && items.length > 1 && <div className="text-[11px] text-muted-foreground">{`${items.length} von ${maxFiles} Dateien`}</div>}
              </div>
            ) : (
              <div className="relative">
                <div className="flex h-16 w-40 items-center gap-2 rounded-md border border-border bg-card px-2">
                  {busy ? <LoaderCircle className="size-5 shrink-0 animate-spin text-muted-foreground" /> : <FileText className="size-5 shrink-0 text-muted-foreground" />}
                  <span className="min-w-0 text-[12px]">
                    <span className="block truncate">{a.name}</span>
                    <span className="text-[11px] text-muted-foreground">{caption(a.kind, a.name, a.size)}</span>
                  </span>
                </div>
                <Remove name={a.name} onRemove={() => onRemove(a.key)} />
                {last && <div className="mt-0.5 text-[11px] text-muted-foreground">{`${items.length} von ${maxFiles} Dateien`}</div>}
              </div>
            )}
            {a.error && (
              <div className="mt-1 max-w-xs border-l-2 border-deny bg-deny-soft px-2 py-1 text-[12px]" role="alert">
                <span className="font-semibold">{a.name} nicht angehängt:</span> {a.error}
              </div>
            )}
          </div>
        )
      })}
      {rejected.map((r) => (
        <div key={r.name} className="mb-4 max-w-xs border-l-2 border-deny bg-deny-soft px-2 py-1 text-[12px]" role="alert">
          <span className="font-semibold">{r.name} nicht angehängt:</span> {r.reason}
        </div>
      ))}
    </div>
  )
}

/** Anhang einer gesendeten Nachricht: Bild als Vorschau aus dem Blob, sonst als Datei. */
export function SentAttachment({ sessionId, attachment }: { sessionId: string; attachment: Attachment }) {
  const kind = kindOf(attachment.mime) ?? 'text'
  const [url, setUrl] = useState<string | undefined>()
  useEffect(() => {
    if (kind !== 'image') return
    let revoke: string | undefined
    let cancelled = false
    client
      .session(sessionId)
      .blobData(attachment.blob)
      .then((b) => {
        if (cancelled) return
        revoke = URL.createObjectURL(new Blob([b], { type: attachment.mime }))
        setUrl(revoke)
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
      if (revoke) URL.revokeObjectURL(revoke)
    }
  }, [sessionId, attachment.blob, attachment.mime, kind])
  if (kind === 'image') {
    return (
      <div data-testid="sent-attachment">
        <div className="flex h-16 w-24 items-center justify-center overflow-hidden rounded-md border border-border bg-sunken">
          {url ? <img src={url} alt={attachment.name} className="size-full object-cover" /> : <Image className="size-5 text-muted-foreground" />}
        </div>
        <div className="mt-0.5 flex max-w-48 items-center gap-1 text-[11px] text-muted-foreground">
          <Image className="size-3 shrink-0" />
          <span className="truncate">{caption('image', attachment.name, Number(attachment.size))}</span>
        </div>
      </div>
    )
  }
  return (
    <div data-testid="sent-attachment" className="flex h-16 w-40 items-center gap-2 rounded-md border border-border bg-card px-2">
      <FileText className="size-5 shrink-0 text-muted-foreground" />
      <span className="min-w-0 text-[12px]">
        <span className="block truncate">{attachment.name}</span>
        <span className="text-[11px] text-muted-foreground">{caption(kind, attachment.name, Number(attachment.size))}</span>
      </span>
    </div>
  )
}
