import { useEffect, useRef, useState } from 'react'
import type { Capabilities } from '@beton/sdk'
import { ArrowUp, Square } from 'lucide-react'
import { cn } from '@/lib/utils'
import { HarnessBadge } from './harness'

const DRAFT_PREFIX = 'beton.draft.'

/** Entwurf je Session in `localStorage` (WEB-004 AC3). */
export function loadDraft(sessionId: string): string {
  try {
    return localStorage.getItem(DRAFT_PREFIX + sessionId) ?? ''
  } catch {
    return ''
  }
}

export function saveDraft(sessionId: string, text: string): void {
  try {
    if (text) localStorage.setItem(DRAFT_PREFIX + sessionId, text)
    else localStorage.removeItem(DRAFT_PREFIX + sessionId)
  } catch {
    // Speicher nicht verfügbar (privates Fenster): Entwurf bleibt nur im Speicher.
  }
}

/** Wählbare Werte eines Pickers laut Capabilities (WEB-004 AC1). */
export function pickerOptions(caps: Capabilities | undefined): { models: string[]; efforts: string[] } {
  if (!caps) return { models: [], efforts: [] }
  return {
    models: caps.models ?? [],
    efforts: caps.effort_switch === 'none' ? [] : (caps.efforts ?? []),
  }
}

export function Picker({
  label,
  value,
  options,
  onChange,
  disabled,
}: {
  label: string
  value: string
  options: string[]
  onChange: (v: string) => void
  disabled?: boolean
}) {
  return (
    <label className="inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-xs text-muted-foreground hover:bg-accent hover:text-foreground">
      <span className="sr-only">{label}</span>
      <select
        aria-label={label}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        className="cursor-pointer bg-transparent font-mono text-[12px] outline-none disabled:cursor-default"
      >
        {options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    </label>
  )
}

/**
 * Eingabe mit Pickern (WEB-004). Senden mit ⏎, Zeilenumbruch mit ⇧⏎. Während eines Turns
 * heißt der Senden-Button „Einreihen“; eingereihte Nachrichten gehen nach dem Turn raus.
 */
export function Composer({
  sessionId,
  harness,
  model,
  capabilities,
  running,
  queued,
  onSend,
  onInterrupt,
  onModel,
}: {
  sessionId: string
  harness: string
  model?: string | null | undefined
  capabilities?: Capabilities | undefined
  running: boolean
  queued: string[]
  onSend: (text: string) => void
  onInterrupt: () => void
  onModel?: ((model: string) => void) | undefined
}) {
  const [text, setText] = useState(() => loadDraft(sessionId))
  const ref = useRef<HTMLTextAreaElement>(null)
  useEffect(() => setText(loadDraft(sessionId)), [sessionId])
  useEffect(() => saveDraft(sessionId, text), [sessionId, text])
  useEffect(() => {
    const el = ref.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`
  }, [text])
  const { models, efforts } = pickerOptions(capabilities)
  const [effort, setEffort] = useState(efforts[0] ?? '')
  const send = () => {
    const t = text.trim()
    if (!t) return
    onSend(t)
    setText('')
  }
  return (
    <div className="shrink-0 border-t border-border p-3">
      {queued.length > 0 && (
        <div className="mb-2 flex flex-col gap-1">
          {queued.map((q, i) => (
            <div key={i} className="flex items-center gap-2 rounded-md border border-dashed border-border px-2 py-1 text-[12px]">
              <span className="text-muted-foreground">Eingereiht {i + 1}</span>
              <span className="truncate">{q}</span>
            </div>
          ))}
        </div>
      )}
      <div className="rounded-md border border-input bg-card focus-within:outline-2 focus-within:outline-ring">
        <textarea
          ref={ref}
          value={text}
          rows={2}
          aria-label="Nachricht"
          placeholder="Nachricht an den Agent"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault()
              send()
            }
          }}
          className="block min-h-[52px] w-full resize-none bg-transparent px-3 pt-2.5 text-[14px] outline-none placeholder:text-muted-foreground"
        />
        <div className="flex flex-wrap items-center gap-0.5 px-1.5 pb-1.5">
          <span className="px-1.5">
            <HarnessBadge harness={harness} />
          </span>
          {models.length > 0 && (
            <Picker
              label="Modell"
              value={model && models.includes(model) ? model : (models[0] ?? '')}
              options={models}
              onChange={(m) => onModel?.(m)}
              disabled={!onModel}
            />
          )}
          {efforts.length > 0 && <Picker label="Effort" value={effort} options={efforts} onChange={setEffort} disabled />}
          <div className="ml-auto flex items-center gap-1">
            {running && (
              <button onClick={onInterrupt} className="flex h-7 items-center gap-1 rounded-md border border-foreground/40 px-2 text-xs" aria-label="Unterbrechen">
                <Square className="size-3 fill-current" /> Stopp
              </button>
            )}
            <button
              onClick={send}
              disabled={!text.trim()}
              className={cn('flex h-7 items-center gap-1 rounded-md bg-foreground px-2 text-xs text-background disabled:opacity-40')}
              aria-label={running ? 'Einreihen' : 'Senden'}
            >
              <ArrowUp className="size-4" />
              <span className="hidden sm:inline">{running ? 'Einreihen' : 'Senden'}</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
