import { useEffect, useRef, useState, type ReactNode } from 'react'
import type { Capabilities, PermissionMode } from '@beton/sdk'
import { Check, ChevronDown, Lock } from 'lucide-react'
import { cn } from '@/lib/utils'
import { harnessName } from './harness'

/** Effort-Stufen auf Deutsch (Screen `harness-switching`). */
export const EFFORT_LABEL: Record<string, string> = {
  low: 'niedrig',
  medium: 'mittel',
  high: 'hoch',
  xhigh: 'sehr hoch',
}

export function effortLabel(effort: string): string {
  return EFFORT_LABEL[effort] ?? effort
}

/** Texte der Permission-Modes (Screen `harness-switching`, Zustand „Modus wählen“). */
export const MODE_TEXT: Record<PermissionMode, { title: string; sub: string }> = {
  plan: { title: 'Nur planen', sub: 'Liest und plant, ändert nichts.' },
  default: { title: 'Fragen bei Schreibzugriff', sub: 'Standard des Harness; Freigaben laut deinen Policies.' },
  accept_edits: { title: 'Dateien ohne Rückfrage ändern', sub: 'Edits im Worktree laufen durch, Shell-Befehle fragen weiter.' },
  yolo: { title: 'YOLO – keine Rückfragen des Agents', sub: '' },
}

/** Hinweis, solange Tool-Sandbox und Egress-Proxy fehlen (Zustand „YOLO ohne Sandbox“). */
export const YOLO_BLOCKED = 'Nicht verfügbar: Sandbox für Tools ist auf diesem Rechner aus. YOLO startet nur mit Sandbox und Egress-Proxy.'

/** Hinweis während eines Turns (Zustand „Modell wählen“). */
export const RUNNING_HINT = 'Der Agent arbeitet gerade. Ein Wechsel gilt ab dem nächsten Turn; der Verlauf bleibt erhalten.'

const MODE_ORDER: PermissionMode[] = ['plan', 'default', 'accept_edits', 'yolo']

/**
 * Wählbare Werte laut Capabilities (WEB-004 AC1): Modelle, Effort-Stufen (nur mit
 * `effort_switch` ≠ `none`) und Permission-Modes, die der Harness abbilden kann.
 */
export function settingsOptions(caps: Capabilities | undefined): {
  models: string[]
  efforts: string[]
  modes: PermissionMode[]
  modelSwitch: boolean
} {
  if (!caps) return { models: [], efforts: [], modes: [], modelSwitch: false }
  const declared = caps.permission_modes ?? []
  return {
    models: caps.models ?? [],
    efforts: caps.effort_switch === 'none' ? [] : (caps.efforts ?? []),
    modes: MODE_ORDER.filter((m) => declared.includes(m)),
    modelSwitch: caps.model_switch !== 'none',
  }
}

/** Ob YOLO hier startbar ist: erst mit Tool-Sandbox und Egress-Proxy (SBX-006, PRX-008, M2). */
export const YOLO_AVAILABLE = false

/** Änderung, die der Composer per `PATCH /v1/sessions/{id}` schickt. */
export type SettingsPatch = { model?: string; effort?: string; permission_mode?: PermissionMode }

function usePopover() {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const outside = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false)
    }
    const esc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        setOpen(false)
      }
    }
    document.addEventListener('mousedown', outside)
    document.addEventListener('keydown', esc)
    return () => {
      document.removeEventListener('mousedown', outside)
      document.removeEventListener('keydown', esc)
    }
  }, [open])
  return { open, setOpen, ref }
}

function Trigger({ label, children, open, onClick, disabled }: { label: string; children: ReactNode; open: boolean; onClick: () => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-haspopup="menu"
      aria-expanded={open}
      disabled={disabled}
      onClick={onClick}
      className="flex h-7 items-center gap-1 rounded-md px-1.5 text-xs text-muted-foreground hover:bg-accent hover:text-foreground disabled:cursor-default disabled:hover:bg-transparent"
    >
      {children}
      {!disabled && <ChevronDown className="size-3" />}
    </button>
  )
}

function Option({
  title,
  sub,
  selected,
  disabled,
  right,
  onSelect,
}: {
  title: ReactNode
  sub?: ReactNode
  selected?: boolean
  disabled?: boolean
  right?: ReactNode
  onSelect?: () => void
}) {
  return (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={!!selected}
      aria-disabled={disabled || undefined}
      disabled={disabled}
      onClick={onSelect}
      className={cn(
        'flex w-full items-start gap-2 rounded-[3px] px-2 py-1.5 text-left outline-none',
        selected && 'bg-accent',
        disabled ? 'cursor-default text-muted-foreground' : 'hover:bg-accent focus-visible:bg-accent',
      )}
    >
      <span className="mt-0.5 w-3.5 shrink-0">{selected && <Check className="size-3.5" />}</span>
      <span className="min-w-0 flex-1">
        <span className={cn(disabled ? '' : 'font-medium')}>{title}</span>
        {sub && <span className="block text-[12px] text-muted-foreground">{sub}</span>}
      </span>
      {right}
    </button>
  )
}

const POPOVER =
  'absolute bottom-full left-0 z-30 mb-1 rounded-md border border-border bg-popover p-1.5 text-[13px] text-popover-foreground shadow-[0_12px_32px_-12px_rgb(0_0_0/0.35)]'

/**
 * Modell- und Effort-Picker (HAR-017, WEB-004): Modelle des Harness, darunter die Effort-Stufen.
 * Ein Wechsel während eines Turns gilt ab dem nächsten (HAR-017 AC4).
 */
export function ModelPicker({
  harness,
  model,
  effort,
  models,
  efforts,
  modelSwitch,
  running,
  onChange,
}: {
  harness: string
  model?: string | null | undefined
  effort?: string | null | undefined
  models: string[]
  efforts: string[]
  modelSwitch: boolean
  running: boolean
  onChange?: ((patch: SettingsPatch) => void) | undefined
}) {
  const { open, setOpen, ref } = usePopover()
  const current = model ?? undefined
  const pick = (patch: SettingsPatch) => {
    setOpen(false)
    onChange?.(patch)
  }
  const shownEffort = effort ?? undefined
  return (
    <div ref={ref} className="relative flex items-center">
      {(models.length > 0 || current) && (
        <Trigger label="Modell" open={open} onClick={() => setOpen((o) => !o)} disabled={!onChange || (!modelSwitch && efforts.length === 0)}>
          <span className="font-mono text-[12px]">{current ?? models[0] ?? ''}</span>
        </Trigger>
      )}
      {efforts.length > 0 && (
        <Trigger label="Effort" open={open} onClick={() => setOpen((o) => !o)} disabled={!onChange}>
          <span>Effort: {shownEffort ? effortLabel(shownEffort) : 'Standard'}</span>
        </Trigger>
      )}
      {open && (
        <div role="menu" aria-label="Modell und Effort" className={cn(POPOVER, 'w-[min(360px,calc(100vw-2rem))]')}>
          {models.length > 0 && (
            <>
              <div className="px-2 pt-1 pb-1.5 text-[11px] text-muted-foreground">Modell · {harnessName(harness)}</div>
              {models.map((m) => (
                <Option
                  key={m}
                  title={<span className="font-mono text-[12.5px]">{m}</span>}
                  sub={m === current ? 'aktuell' : undefined}
                  selected={m === current}
                  disabled={!modelSwitch && m !== current}
                  onSelect={m === current || !modelSwitch ? undefined : () => pick({ model: m })}
                />
              ))}
            </>
          )}
          {efforts.length > 0 && (
            <>
              {models.length > 0 && <div className="my-1.5 border-t border-border" />}
              <div className="px-2 pb-1 text-[11px] text-muted-foreground">Effort</div>
              <div className="flex flex-wrap gap-1 px-2 pb-1.5" role="group" aria-label="Effort">
                {efforts.map((e) => (
                  <button
                    key={e}
                    type="button"
                    role="menuitemradio"
                    aria-checked={e === shownEffort}
                    onClick={e === shownEffort ? undefined : () => pick({ effort: e })}
                    className={cn(
                      'rounded-[3px] border px-2 py-1 text-xs',
                      e === shownEffort ? 'border-foreground bg-foreground text-background' : 'border-border hover:bg-accent',
                    )}
                  >
                    {effortLabel(e)}
                  </button>
                ))}
              </div>
            </>
          )}
          {running && <div className="mx-2 mb-1 rounded-sm bg-muted px-2 py-1.5 text-[12px] text-muted-foreground">{RUNNING_HINT}</div>}
        </div>
      )}
    </div>
  )
}

/**
 * Permission-Mode-Picker (HAR-027, WEB-004): nur die Modi, die der Harness abbilden kann. YOLO
 * bleibt ohne Tool-Sandbox und Egress-Proxy gesperrt (fail closed, HAR-027 AC1).
 */
export function ModePicker({
  harness,
  mode,
  modes,
  running,
  onChange,
}: {
  harness: string
  mode?: PermissionMode | null | undefined
  modes: PermissionMode[]
  running: boolean
  onChange?: ((patch: SettingsPatch) => void) | undefined
}) {
  const { open, setOpen, ref } = usePopover()
  if (modes.length === 0) return null
  const current: PermissionMode = mode ?? 'default'
  const pick = (m: PermissionMode) => {
    setOpen(false)
    if (m !== current) onChange?.({ permission_mode: m })
  }
  return (
    <div ref={ref} className="relative">
      <Trigger label="Permission-Mode" open={open} onClick={() => setOpen((o) => !o)} disabled={!onChange || modes.length < 2}>
        <span>{MODE_TEXT[current].title}</span>
      </Trigger>
      {open && (
        <div role="menu" aria-label="Permission-Mode" className={cn(POPOVER, 'w-[min(400px,calc(100vw-2rem))]')}>
          <div className="px-2 pt-1 pb-1.5 text-[11px] text-muted-foreground">Wie viel darf der Agent ohne Rückfrage?</div>
          {modes.map((m) =>
            m === 'yolo' && !YOLO_AVAILABLE ? (
              <Option key={m} disabled title={MODE_TEXT.yolo.title} sub={YOLO_BLOCKED} right={<Lock className="mt-0.5 size-3.5" aria-label="gesperrt" />} />
            ) : (
              <Option key={m} title={MODE_TEXT[m].title} sub={MODE_TEXT[m].sub} selected={m === current} onSelect={() => pick(m)} />
            ),
          )}
          <div className="mx-2 mt-1 mb-1 border-t border-border pt-1.5 text-[11px] text-muted-foreground">
            {running ? RUNNING_HINT : `${harnessName(harness)} übernimmt den Modus sofort.`}
          </div>
        </div>
      )}
    </div>
  )
}
