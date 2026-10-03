import type { CSSProperties, ReactNode } from 'react'
import { BatteryFull, Code2, FolderOpen, Globe, SquareTerminal, Wifi } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'

/*
 * Ein nachgebauter Schreibtisch (macOS) für Desktop-spezifische Screens: Menüleiste mit
 * Tray-Symbol, Dock mit Badge, Fenster und System-Benachrichtigungen.
 * Nur Tokens – keine Systemfarben; die Fensterpunkte bleiben neutral.
 */

/** App-Symbol von beton: ein Betonquader mit „b“. */
export function BetonGlyph({ className }: { className?: string }) {
  return (
    <span
      aria-hidden
      className={cn('type-wide flex items-center justify-center rounded-[22%] bg-foreground font-[800] text-background', className)}
    >
      b
    </span>
  )
}

/** Zähler „du bist dran“ (offene Freigaben + ungelesene Sessions). */
export function CountBadge({ n, className }: { n: number; className?: string }) {
  return (
    <span className={cn('chamfer-sm min-w-4 bg-signal px-1 text-center text-[10px] leading-4 font-semibold text-signal-foreground', className)}>
      {n}
    </span>
  )
}

type SceneProps = {
  /** Name der App im Vordergrund (Menüleiste links). */
  app?: string
  menus?: string[]
  /** Geöffnetes Menü unter einem Menütitel der App. */
  appMenu?: { title: string; content: ReactNode }
  /** Geöffnetes Tray-Menü. */
  tray?: ReactNode
  trayCount?: number
  /** Aufnahme läuft: Tray-Symbol zeigt das. */
  trayRecording?: boolean
  dockCount?: number
  children?: ReactNode
  className?: string
}

export function DesktopScene({
  app = 'beton',
  menus = ['Ablage', 'Bearbeiten', 'Darstellung', 'Session', 'Fenster', 'Hilfe'],
  appMenu,
  tray,
  trayCount = 3,
  trayRecording,
  dockCount = 3,
  children,
  className,
}: SceneProps) {
  return (
    <div className={cn('concrete-grain relative h-full min-h-[680px] overflow-hidden rounded-lg border border-border bg-sunken', className)}>
      {/* Menüleiste */}
      <div className="relative z-40 flex h-7 items-center gap-4 border-b border-border/60 bg-card/85 px-3 text-[12.5px] backdrop-blur">
        <span className="font-semibold">{app}</span>
        {menus.map((m) => (
          <span key={m} className={cn('relative rounded-sm px-1', appMenu?.title === m && 'bg-accent')}>
            {m}
            {appMenu?.title === m && <div className="absolute top-6 left-0 z-50">{appMenu.content}</div>}
          </span>
        ))}
        <div className="ml-auto flex items-center gap-3">
          <F id="DESK-004" as="span" badge="bottom-left" className="relative">
            <span className={cn('flex items-center gap-1 rounded-sm px-1', tray && 'bg-accent')} title="beton im Tray">
              <BetonGlyph className="size-4 text-[10px]" />
              {trayRecording ? (
                <span className="inline-flex items-center gap-1 text-[11px]">
                  <span className="size-1.5 animate-pulse rounded-full bg-deny" /> Aufnahme
                </span>
              ) : trayCount > 0 ? (
                <CountBadge n={trayCount} />
              ) : null}
            </span>
            {tray && <div className="absolute top-6 right-0 z-50">{tray}</div>}
          </F>
          <Wifi className="size-3.5" aria-label="WLAN" />
          <BatteryFull className="size-4" aria-label="Akku" />
          <span className="tabular-nums">Fr. 3. Okt. 09:41</span>
        </div>
      </div>
      {children}
      <Dock count={dockCount} />
    </div>
  )
}

function DockIcon({ children, label, running, className }: { children: ReactNode; label: string; running?: boolean; className?: string }) {
  return (
    <div className="flex flex-col items-center gap-0.5" title={label}>
      <span className={cn('flex size-10 items-center justify-center rounded-[22%] border border-border bg-card', className)}>{children}</span>
      <span className={cn('size-1 rounded-full', running ? 'bg-foreground/70' : 'bg-transparent')} />
    </div>
  )
}

function Dock({ count }: { count: number }) {
  return (
    <div className="absolute bottom-2 left-1/2 z-30 flex -translate-x-1/2 items-end gap-2 rounded-2xl border border-border/70 bg-card/70 px-2.5 pt-1.5 pb-1 backdrop-blur">
      <DockIcon label="Finder" running>
        <FolderOpen className="size-5 text-muted-foreground" />
      </DockIcon>
      <DockIcon label="Editor" running>
        <Code2 className="size-5 text-muted-foreground" />
      </DockIcon>
      <DockIcon label="Terminal" running>
        <SquareTerminal className="size-5 text-muted-foreground" />
      </DockIcon>
      <DockIcon label="Browser">
        <Globe className="size-5 text-muted-foreground" />
      </DockIcon>
      <F id="DESK-004" badge="top-right" className="relative">
        <DockIcon label="beton" running className="border-0 bg-transparent">
          <BetonGlyph className="size-10 text-[20px]" />
        </DockIcon>
        {count > 0 && <CountBadge n={count} className="absolute -top-1 -right-1.5 text-[11px] leading-[18px]" />}
      </F>
    </div>
  )
}

/** Ein Fenster auf dem Schreibtisch. */
export function SceneWindow({
  title,
  children,
  style,
  className,
  focused = true,
}: {
  title: string
  children: ReactNode
  style?: CSSProperties
  className?: string
  focused?: boolean
}) {
  return (
    <div
      className={cn(
        'absolute flex flex-col overflow-hidden rounded-lg border border-border bg-background',
        focused ? 'z-20 shadow-[0_24px_50px_-20px_rgb(0_0_0/0.45)]' : 'z-10 shadow-md',
        className,
      )}
      style={style}
    >
      <div className="flex h-7 shrink-0 items-center gap-3 border-b border-border bg-sidebar px-2.5">
        <span className="flex gap-1.5" aria-hidden>
          {[0, 1, 2].map((i) => (
            <span key={i} className={cn('size-2.5 rounded-full', focused ? 'bg-muted-foreground/45' : 'bg-muted-foreground/20')} />
          ))}
        </span>
        <span className={cn('flex-1 truncate text-center text-[11.5px]', focused ? 'text-foreground' : 'text-muted-foreground')}>{title}</span>
        <span className="w-[42px]" />
      </div>
      <div className="min-h-0 flex-1">{children}</div>
    </div>
  )
}

/** Eine System-Benachrichtigung (macOS-Banner). */
export function Notification({
  title,
  body,
  time = 'jetzt',
  actions,
  waiting,
  className,
}: {
  title: string
  body?: ReactNode
  time?: string
  actions?: string[]
  /** Wartet auf dich (Freigabe/Frage) – Fase. */
  waiting?: boolean
  className?: string
}) {
  return (
    <div className={cn('flex w-[350px] gap-2.5 border border-border bg-popover/95 p-3 shadow-lg backdrop-blur', waiting ? 'chamfer border-l-4 border-l-signal' : 'rounded-xl', className)}>
      <BetonGlyph className="mt-0.5 size-8 shrink-0 text-[16px]" />
      <div className="min-w-0 flex-1 text-[12.5px]">
        <div className="flex items-baseline gap-2">
          <span className="truncate font-semibold">{title}</span>
          <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">{time}</span>
        </div>
        {body && <div className="mt-0.5 line-clamp-2 text-muted-foreground">{body}</div>}
        {actions && (
          <div className="mt-2 flex gap-1.5">
            {actions.map((a, i) => (
              <button key={a} className={cn('rounded-md px-2 py-0.5 text-[12px]', i === 0 ? 'bg-foreground font-medium text-background' : 'border border-border')}>
                {a}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

/** Fremde App im Vordergrund (ein Editor), damit klar ist: beton ist nicht fokussiert. */
export function EditorWindow({ style }: { style?: CSSProperties }) {
  const lines = [
    "import { rateLimit } from '../middleware/rate-limit'",
    '',
    'export const auth = Router()',
    "auth.post('/login', rateLimit({ window: '1m', max: 5 }), login)",
    "auth.post('/register', register)",
    '',
    'function login(req: Request, res: Response) {',
    '  const { email, password } = req.body',
    '  // …',
    '}',
  ]
  return (
    <SceneWindow title="auth.ts — shop-frontend" style={style}>
      <div className="flex h-full bg-card font-mono text-[12px] leading-[1.7]">
        <div className="w-40 shrink-0 border-r border-border bg-sidebar p-2 font-sans text-[12px] text-muted-foreground">
          <div>src</div>
          <div className="pl-3">middleware</div>
          <div className="pl-3">routes</div>
          <div className="rounded-sm bg-accent pl-6 text-foreground">auth.ts</div>
          <div className="pl-6">orders.ts</div>
        </div>
        <div className="p-3">
          {lines.map((l, i) => (
            <div key={i} className="flex gap-4 whitespace-pre">
              <span className="w-5 text-right text-muted-foreground select-none">{i + 12}</span>
              <span>{l}</span>
            </div>
          ))}
        </div>
      </div>
    </SceneWindow>
  )
}

/** Inline-Dialog innerhalb eines Fensters (Radix-Dialoge würden über den ganzen Prototyp portalen). */
export function InlineDialog({
  children,
  waiting = true,
  className,
}: {
  children: ReactNode
  /** Wartet auf deine Entscheidung – Fase und Schalungsgelb. */
  waiting?: boolean
  className?: string
}) {
  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-background/55 p-6">
      <div
        role="dialog"
        className={cn(
          'w-full max-w-md bg-popover p-5 shadow-xl',
          waiting ? 'chamfer border-l-4 border-signal' : 'rounded-lg border border-border',
          className,
        )}
      >
        {children}
      </div>
    </div>
  )
}

/** Zeile in Einstellungen: Text links, Bedienelement rechts. */
export function SettingRow({ label, hint, children, muted }: { label: ReactNode; hint?: ReactNode; children?: ReactNode; muted?: boolean }) {
  return (
    <div className={cn('flex items-center gap-4 py-2', muted && 'opacity-55')}>
      <div className="min-w-0 flex-1 text-[13px]">
        <div>{label}</div>
        {hint && <div className="text-[11.5px] text-muted-foreground">{hint}</div>}
      </div>
      {children}
    </div>
  )
}

export function SettingsSection({ title, hint, children, className }: { title: string; hint?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={cn('border-t border-border py-5', className)}>
      <h3 className="type-wide text-[14px] font-[650]">{title}</h3>
      {hint && <p className="mt-0.5 max-w-2xl text-[12.5px] text-muted-foreground">{hint}</p>}
      <div className="mt-2">{children}</div>
    </section>
  )
}

export function SettingsHeader({ page }: { page: string }) {
  return (
    <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-6">
      <span className="text-[13px] text-muted-foreground">Einstellungen</span>
      <span className="text-muted-foreground">/</span>
      <h2 className="text-[15px] font-semibold">{page}</h2>
    </div>
  )
}

/** Segment-Auswahl (z. B. Halten / Umschalten). */
export function Segmented({ options, value }: { options: string[]; value: string }) {
  return (
    <div role="radiogroup" className="inline-flex rounded-md border border-border p-0.5 text-[12px]">
      {options.map((o) => (
        <button key={o} role="radio" aria-checked={o === value} className={cn('rounded-[3px] px-2.5 py-0.5', o === value ? 'bg-foreground text-background' : 'text-muted-foreground hover:text-foreground')}>
          {o}
        </button>
      ))}
    </div>
  )
}
