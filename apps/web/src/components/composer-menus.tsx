import type { ReactNode } from 'react'
import { FileText, Sparkles, SquareSlash } from 'lucide-react'
import { highlight } from '@/lib/composer'
import { cn } from '@/lib/utils'

/** Menü über dem Eingabefeld (Prototyp `workspace-composer`). */
export function Menu({ children, className, label }: { children: ReactNode; className?: string; label: string }) {
  return (
    <div
      role="listbox"
      aria-label={label}
      className={cn(
        'z-30 max-h-[50vh] overflow-y-auto rounded-lg border border-border bg-popover py-1 text-popover-foreground shadow-[0_16px_40px_-20px_rgb(0_0_0/0.5)]',
        className,
      )}
    >
      {children}
    </div>
  )
}

export function MenuLabel({ children }: { children: ReactNode }) {
  return <div className="px-3 pt-2 pb-1 text-[11px] font-medium text-muted-foreground">{children}</div>
}

export function MenuSeparator() {
  return <div className="my-1 h-px bg-border" />
}

export function MenuItem({
  id,
  icon,
  children,
  hint,
  active,
  right,
  onPick,
}: {
  id: string
  icon?: ReactNode
  children: ReactNode
  hint?: ReactNode
  active?: boolean
  right?: ReactNode
  onPick: () => void
}) {
  return (
    <div
      id={id}
      role="option"
      aria-selected={active}
      // Auswahl per Maus, ohne dass das Eingabefeld den Fokus verliert.
      onMouseDown={(e) => {
        e.preventDefault()
        onPick()
      }}
      className={cn('mx-1 flex cursor-pointer items-start gap-2 rounded-md px-2 py-1.5 text-[13px] hover:bg-accent/60', active && 'bg-accent')}
    >
      {icon && <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center text-muted-foreground">{icon}</span>}
      <span className="min-w-0 flex-1">
        <span className="block truncate">{children}</span>
        {hint && <span className="block text-[11px] text-muted-foreground">{hint}</span>}
      </span>
      {right && <span className="shrink-0 text-[11px] text-muted-foreground">{right}</span>}
    </div>
  )
}

const de = new Intl.NumberFormat('de-DE')

/** `@`-Dateisuche (WEB-006 AC2). */
export function MentionMenu({
  query,
  paths,
  total,
  ms,
  active,
  onPick,
}: {
  query: string
  paths: string[]
  total?: number | undefined
  ms?: number | undefined
  active: number
  onPick: (path: string) => void
}) {
  const stats = [`${paths.length}${total !== undefined ? ` von ${de.format(total)}` : ''}`, ms !== undefined ? `${Math.round(ms)} ms` : undefined]
    .filter(Boolean)
    .join(' · ')
  return (
    <Menu label="Dateien im Workspace" className="w-[min(460px,calc(100vw-2rem))]">
      <MenuLabel>Dateien im Workspace · {stats}</MenuLabel>
      {paths.length === 0 && <div className="px-3 py-1.5 text-[12px] text-muted-foreground">Keine Datei passt zu „{query}“.</div>}
      {paths.map((p, i) => (
        <MenuItem
          key={p}
          id={`mention-${i}`}
          active={i === active}
          icon={<FileText className="size-3.5" />}
          right={i === active ? '↵ einfügen' : undefined}
          onPick={() => onPick(p)}
        >
          <span className="font-mono text-[12px]">
            {highlight(p, query).map((part, j) =>
              part.hit ? (
                <mark key={j} className="rounded-[2px] bg-foreground/15 font-semibold text-foreground">
                  {part.text}
                </mark>
              ) : (
                <span key={j}>{part.text}</span>
              ),
            )}
          </span>
        </MenuItem>
      ))}
    </Menu>
  )
}

/** Eintrag im Slash-Menü. */
export interface SlashEntry {
  /** Aufruf ohne `/`. */
  name: string
  /** Anzeigename (beton-Befehle) bzw. Name des Skills. */
  title: string
  description: string
  group: 'beton' | 'skill'
}

/** Slash-Menü: beton-Befehle und Skills des Agents (WEB-006 AC3). */
export function SlashMenu({
  entries,
  agent,
  active,
  onPick,
}: {
  entries: SlashEntry[]
  agent?: string | undefined
  active: number
  onPick: (e: SlashEntry) => void
}) {
  const beton = entries.filter((e) => e.group === 'beton')
  const skills = entries.filter((e) => e.group === 'skill')
  const item = (e: SlashEntry) => {
    const i = entries.indexOf(e)
    return (
      <MenuItem
        key={`${e.group}:${e.name}`}
        id={`slash-${i}`}
        active={i === active}
        icon={e.group === 'beton' ? <SquareSlash className="size-3.5" /> : <Sparkles className="size-3.5" />}
        hint={e.description}
        right={`/${e.name}`}
        onPick={() => onPick(e)}
      >
        {e.title}
      </MenuItem>
    )
  }
  return (
    <Menu label="Befehle und Skills" className="w-[min(480px,calc(100vw-2rem))]">
      {beton.length > 0 && <MenuLabel>beton</MenuLabel>}
      {beton.map(item)}
      {beton.length > 0 && skills.length > 0 && <MenuSeparator />}
      {skills.length > 0 && <MenuLabel>{agent ? `Skills des Agents „${agent}“` : 'Skills'}</MenuLabel>}
      {skills.map(item)}
      {entries.length === 0 && <div className="px-3 py-1.5 text-[12px] text-muted-foreground">Kein Befehl und kein Skill passt.</div>}
    </Menu>
  )
}
