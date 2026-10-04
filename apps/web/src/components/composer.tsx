import { useEffect, useRef, useState, type ReactNode } from 'react'
import type { Capabilities, HarnessInfo } from '@beton/sdk'
import { ArrowUp, Square } from 'lucide-react'
import { cn } from '@/lib/utils'
import { HarnessMenu } from './fork'
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
 * heißt der Senden-Button „Einreihen“; die Queue (WEB-005) steht über dem Eingabefeld.
 */
export function Composer({
  sessionId,
  harness,
  model,
  capabilities,
  running,
  queue,
  onSend,
  onInterrupt,
  onModel,
  continueWith,
  onContinue,
}: {
  sessionId: string
  harness: string
  model?: string | null | undefined
  capabilities?: Capabilities | undefined
  running: boolean
  /** Serverseitige Queue über dem Eingabefeld (WEB-005). */
  queue?: ReactNode
  onSend: (text: string) => void
  onInterrupt: () => void
  onModel?: ((model: string) => void) | undefined
  /** Harnesses für „Weiter mit …“ im Harness-Picker (SES-007 AC5). */
  continueWith?: HarnessInfo[] | undefined
  /** Fork ab dem letzten `seq` auf einen anderen Harness. */
  onContinue?: ((harness: string) => void) | undefined
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
      {queue}
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
          {onContinue ? (
            <HarnessMenu harness={harness} targets={continueWith ?? []} onContinue={onContinue} />
          ) : (
            <span className="px-1.5">
              <HarnessBadge harness={harness} />
            </span>
          )}
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
