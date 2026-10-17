import { useRef, useState, type DragEvent, type ReactNode } from 'react'
import { Paperclip } from 'lucide-react'

/** Ereignis an den Composer einer Session: Dateien anhängen (nicht senden). */
export const ATTACH_EVENT = 'beton:attach-files'

export interface AttachFilesDetail {
  sessionId: string
  files: File[]
}

/** Hängt `files` im Composer der Session an; gesendet wird erst, wenn der User sendet. */
export function attachFiles(sessionId: string, files: File[]) {
  window.dispatchEvent(new CustomEvent<AttachFilesDetail>(ATTACH_EVENT, { detail: { sessionId, files } }))
}

const hasFiles = (e: DragEvent) => [...e.dataTransfer.types].includes('Files')

/**
 * Dateien, die auf das Fenster einer Session gezogen werden, landen als Anhang im Composer
 * (DESK-001 AC3, Entwurf `desktop-shell` Zustand „Datei ablegen“). In der Desktop-App kommt
 * der Drop als HTML5-Drop an; im Browser gilt dasselbe.
 */
export function WindowDrop({ sessionId, title, children }: { sessionId: string; title: string; children: ReactNode }) {
  const [over, setOver] = useState<number | undefined>()
  const depth = useRef(0)
  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1"
      onDragEnter={(e) => {
        if (!hasFiles(e)) return
        e.preventDefault()
        depth.current += 1
        setOver(e.dataTransfer.items.length || 1)
      }}
      onDragOver={(e) => {
        if (hasFiles(e)) e.preventDefault()
      }}
      onDragLeave={(e) => {
        if (!hasFiles(e)) return
        depth.current = Math.max(0, depth.current - 1)
        if (depth.current === 0) setOver(undefined)
      }}
      onDrop={(e) => {
        depth.current = 0
        setOver(undefined)
        if (e.dataTransfer.files.length === 0) return
        e.preventDefault()
        attachFiles(sessionId, [...e.dataTransfer.files])
      }}
    >
      {children}
      {over !== undefined && (
        <div className="absolute inset-0 z-40 flex items-center justify-center bg-background/70 p-8" data-testid="window-drop">
          <div className="pointer-events-none flex w-full max-w-md flex-col items-center gap-2 rounded-xl border-2 border-dashed border-foreground/50 bg-card/80 p-8 text-center">
            <Paperclip className="size-6 text-muted-foreground" />
            <p className="text-[15px] font-semibold">Loslassen zum Anhängen</p>
            <p className="text-[12.5px] text-muted-foreground">
              {over === 1 ? 'Die Datei landet' : `${over} Dateien landen`} als Anhang im Composer von „{title}“. Gesendet wird erst, wenn du sendest.
            </p>
          </div>
        </div>
      )}
    </div>
  )
}
