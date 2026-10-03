import type { CSSProperties, ReactNode } from 'react'
import {
  AppWindow,
  ArrowLeft,
  ArrowRight,
  Crosshair,
  Eye,
  Gauge,
  Globe,
  Lock,
  Plus,
  RotateCw,
  ShieldCheck,
  UserRound,
  X,
} from 'lucide-react'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'

/*
 * Bausteine des Browser-Panels (BRW): Tab-Leiste, Werkzeugleiste, Screencast-Fläche,
 * Statuszeile – und eine erfundene Checkout-Seite aus `shop-frontend`, auf der
 * Agent und Picker arbeiten.
 */

export type BrowserTab = { id: string; title: string; url: string }

export const checkoutTabs: BrowserTab[] = [
  { id: 'tab_1', title: 'Kasse – Acme Shop', url: 'localhost:5173/checkout' },
  { id: 'tab_2', title: 'Warenkorb – Acme Shop', url: 'localhost:5173/cart' },
  { id: 'tab_3', title: 'WCAG 2.2 – Tastatur', url: 'developer.mozilla.org/…/Keyboard' },
]

type PanelProps = {
  tabs?: BrowserTab[]
  activeTab?: string
  url: ReactNode
  /** Lokales Ziel (Dev-Server) oder TLS-Ziel – Schloss bzw. Haus-Symbol. */
  local?: boolean
  /** Hinweis in der Adressleiste, z. B. gesperrtes Ziel. */
  addressNote?: ReactNode
  /** „Agent steuert“: aktuelle Aktion des Agents. */
  driving?: string
  picker?: 'off' | 'on'
  mode?: 'embedded' | 'window'
  /** Fenster-Modus nicht verfügbar – Begründung. */
  windowUnavailable?: string
  banner?: ReactNode
  footer?: ReactNode
  /** Unterhalb des Viewports, z. B. Auswahl-Details des Pickers. */
  drawer?: ReactNode
  children: ReactNode
  className?: string
}

/** Das Browser-Panel im Workspace-Rail (oder groß). */
export function BrowserPanel({
  tabs = checkoutTabs,
  activeTab = 'tab_1',
  url,
  local = true,
  addressNote,
  driving,
  picker = 'off',
  mode = 'embedded',
  windowUnavailable,
  banner,
  footer,
  drawer,
  children,
  className,
}: PanelProps) {
  return (
    <div className={cn('flex h-full min-h-0 flex-col', className)}>
      <F id="BRW-009" className="shrink-0 border-b border-border" badge="top-right">
        <div className="flex h-8 items-end gap-px overflow-hidden bg-sidebar px-1.5 pt-1">
          {tabs.map((t) => (
            <div
              key={t.id}
              className={cn(
                'flex h-7 max-w-44 min-w-0 items-center gap-1.5 rounded-t-md px-2 text-[12px]',
                t.id === activeTab ? 'bg-background' : 'text-muted-foreground hover:bg-accent/60',
              )}
            >
              <Globe className="size-3 shrink-0" />
              <span className="truncate">{t.title}</span>
              <X className="ml-auto size-3 shrink-0 opacity-60" aria-label="Tab schließen" />
            </div>
          ))}
          <button className="mb-1 flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-accent" aria-label="Neuer Tab">
            <Plus className="size-3.5" />
          </button>
        </div>
        <div className="flex h-9 items-center gap-1 px-1.5">
          <IconButton label="Zurück">
            <ArrowLeft className="size-3.5" />
          </IconButton>
          <IconButton label="Vor">
            <ArrowRight className="size-3.5" />
          </IconButton>
          <IconButton label="Neu laden">
            <RotateCw className="size-3.5" />
          </IconButton>
          <div className="flex h-7 min-w-0 flex-1 items-center gap-1.5 rounded-md border border-input bg-card px-2 font-mono text-[12px]">
            {local ? (
              <span title="Lokaler Dev-Server dieser Session" className="shrink-0 rounded-sm bg-muted px-1 font-sans text-[10px] text-muted-foreground">
                lokal
              </span>
            ) : (
              <Lock className="size-3 shrink-0 text-muted-foreground" aria-label="TLS" />
            )}
            <span className="min-w-0 truncate">{url}</span>
            {addressNote && <span className="ml-auto shrink-0 font-sans text-[11px]">{addressNote}</span>}
          </div>
          <F id="BRW-013" as="span" badge="bottom-right">
            <button
              aria-pressed={picker === 'on'}
              title="Element auswählen und an den Agent schicken (⌘⇧C)"
              className={cn(
                'flex h-7 items-center gap-1 px-2 text-[12px]',
                picker === 'on' ? 'chamfer-sm bg-signal font-semibold text-signal-foreground' : 'rounded-md border border-border hover:bg-accent',
              )}
            >
              <Crosshair className="size-3.5" />
              {picker === 'on' ? 'Picker aktiv' : 'Element wählen'}
            </button>
          </F>
          <F id="BRW-008" as="span" badge="bottom-right">
            <button
              disabled={!!windowUnavailable}
              title={windowUnavailable ?? (mode === 'window' ? 'Zurück in das Panel' : 'In einem sichtbaren Chromium-Fenster öffnen (für Logins und DevTools)')}
              className={cn(
                'flex h-7 items-center gap-1 rounded-md border border-border px-2 text-[12px]',
                windowUnavailable ? 'cursor-not-allowed text-muted-foreground opacity-60' : 'hover:bg-accent',
                mode === 'window' && 'bg-accent font-medium',
              )}
            >
              <AppWindow className="size-3.5" />
              {mode === 'window' ? 'Im Fenster' : 'Fenster'}
            </button>
          </F>
        </div>
      </F>
      {driving && (
        <F id={['BRW-009', 'BRW-010']} className="flex h-7 shrink-0 items-center gap-2 border-b border-border bg-card px-3 text-[12px]">
          <span className="size-2 animate-pulse rounded-full bg-voice-claude" aria-hidden />
          <span className="font-medium">Agent steuert</span>
          <span className="truncate font-mono text-[11px] text-muted-foreground">{driving}</span>
          <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">Deine Eingaben werden danach zugestellt</span>
        </F>
      )}
      {banner}
      <F id={['BRW-005', 'BRW-006']} className="relative min-h-0 flex-1 overflow-hidden bg-sunken p-2" badge="bottom-left">
        {children}
      </F>
      {drawer}
      {footer ?? <PanelFooter />}
    </div>
  )
}

function IconButton({ label, children }: { label: string; children: ReactNode }) {
  return (
    <button aria-label={label} title={label} className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground">
      {children}
    </button>
  )
}

/** Statuszeile: Profil, Egress-Proxy, Qualität, Zuschauer. */
export function PanelFooter({
  quality = 'hoch · 30 fps · 2×',
  qualityLow,
  viewers = ['Desktop (du steuerst)', 'Web auf MacBook'],
}: {
  /** `null` = noch kein Bild (Browser nicht gestartet). */
  quality?: string | null
  qualityLow?: boolean
  viewers?: string[]
}) {
  return (
    <div className="flex h-7 shrink-0 items-center gap-3 overflow-hidden border-t border-border px-3 text-[11px] whitespace-nowrap text-muted-foreground">
      <F id="BRW-004" as="span" badge="top-left">
        <span className="inline-flex items-center gap-1" title="~/.beton/browser/profiles/ses_6q2a – getrennt von deinem Alltags-Browser und anderen Sessions">
          <UserRound className="size-3" /> Eigenes Profil
        </span>
      </F>
      <F id="BRW-018" as="span" badge="top-left">
        <span className="inline-flex items-center gap-1" title="Aller Verkehr läuft über den Egress-Proxy der Session; die beton-CA gilt nur in dieser Instanz">
          <ShieldCheck className="size-3" /> Über Egress-Proxy
        </span>
      </F>
      {quality !== null && (
        <F id="BRW-007" as="span" badge="top-left">
          <span className={cn('inline-flex items-center gap-1', qualityLow && 'text-foreground')} title="Qualität passt sich an Verbindung, Fenstergröße und Pixeldichte an">
            <Gauge className="size-3" /> Qualität {quality}
          </span>
        </F>
      )}
      <F id="BRW-020" as="span" className="ml-auto" badge="top-right">
        <span className="inline-flex items-center gap-1" title={viewers.join(', ')}>
          <Eye className="size-3" /> Live auf {viewers.length} {viewers.length === 1 ? 'Gerät' : 'Geräten'}
        </span>
      </F>
    </div>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Erfundene Seite im Screencast                                                               */
/* ------------------------------------------------------------------------------------------ */

export type PickTarget = 'email' | 'street' | 'zip' | 'submit' | 'error' | 'total'

type PageProps = {
  /** Element unter dem Cursor im Picker (Box-Model-Overlay). */
  hover?: PickTarget
  /** Gewählte Elemente mit Nummer. */
  marks?: Partial<Record<PickTarget, number>>
  /** Highlight eines anderen Users (Co-Viewer). */
  remoteHover?: { target: PickTarget; who: string }
  /** Feld hat Fokus (Agent tippt). */
  typing?: boolean
  cursor?: { x: string; y: string; label?: string }
  className?: string
  style?: CSSProperties
}

/** Kasse von `shop-frontend` auf localhost:5173 – so, wie sie der Screencast zeigt. */
export function CheckoutPage({ hover, marks = {}, remoteHover, typing, cursor, className, style }: PageProps) {
  const p = (t: PickTarget) => ({
    hover: hover === t,
    mark: marks[t],
    remote: remoteHover?.target === t ? remoteHover.who : undefined,
  })
  return (
    <div className={cn('relative mx-auto h-full w-full max-w-[640px] overflow-hidden rounded-sm bg-popover text-[12px] text-popover-foreground shadow-sm', className)} style={style}>
      <div className="flex items-center justify-between border-b border-border px-5 py-2">
        <span className="font-semibold tracking-tight">Acme Shop</span>
        <span className="text-[11px] text-muted-foreground">Warenkorb (2)</span>
      </div>
      <div className="grid grid-cols-[minmax(0,1fr)_170px] gap-6 px-5 py-3">
        <div>
          <div className="text-[17px] font-semibold">Kasse</div>
          <div className="mt-2 space-y-2">
            <Pick {...p('email')} margin={[0, 0, 0, 0]} padding={[6, 8]} label="input#email" size="296 × 30">
              <Field label="E-Mail" value={typing ? 'test@example.com' : ''} focused={typing} />
            </Pick>
            <Pick {...p('street')} margin={[0, 0, 0, 0]} padding={[6, 8]} label="input#street" size="296 × 30">
              <Field label="Straße und Hausnummer" value="Hafenstraße 12" />
            </Pick>
            <div className="grid grid-cols-[90px_1fr] gap-2">
              <Pick {...p('zip')} margin={[0, 0, 0, 0]} padding={[6, 8]} label="input#zip" size="90 × 30">
                <Field label="PLZ" value="2045" invalid />
              </Pick>
              <Field label="Ort" value="Hamburg" />
            </div>
            <Pick {...p('error')} margin={[0, 0, 0, 0]} padding={[0, 0]} label="span.error" size="296 × 16">
              <div className="text-[11px] text-deny">Bitte gib eine fünfstellige PLZ ein.</div>
            </Pick>
          </div>
          <div className="mt-1 flex gap-2">
            <Pick
              {...p('submit')}
              margin={[16, 0, 0, 0]}
              padding={[8, 16]}
              label="div.btn.btn-secondary"
              size="104 × 34"
              a11y="Rolle: generic · Tastatur: nicht fokussierbar"
              tipAbove
            >
              <div className="mt-4 inline-flex rounded-[5px] bg-muted px-4 py-2 font-medium">Jetzt kaufen</div>
            </Pick>
            <div className="mt-4 inline-flex rounded-[5px] px-3 py-2 text-muted-foreground">Abbrechen</div>
          </div>
        </div>
        <div className="border-l border-border pl-4">
          <div className="font-semibold">Bestellung</div>
          <div className="mt-2 space-y-1 text-[11px]">
            <Row k="Wanderjacke, Gr. M" v="129,00 €" />
            <Row k="Trinkflasche" v="19,90 €" />
            <Row k="Versand" v="4,90 €" />
          </div>
          <Pick {...p('total')} margin={[8, 0, 0, 0]} padding={[4, 0]} label="dl.total" size="164 × 24">
            <div className="mt-2 flex justify-between border-t border-border py-1 font-semibold">
              <span>Summe</span>
              <span>153,80 €</span>
            </div>
          </Pick>
        </div>
      </div>
      {cursor && (
        <span className="pointer-events-none absolute z-20" style={{ left: cursor.x, top: cursor.y }}>
          <svg width="14" height="18" viewBox="0 0 14 18" aria-hidden>
            <path d="M1 1 L1 15 L4.5 11.5 L7 17 L9 16 L6.5 10.5 L11.5 10.5 Z" fill="var(--foreground)" stroke="var(--popover)" strokeWidth="1" />
          </svg>
          {cursor.label && (
            <span className="ml-3 rounded-sm bg-voice-claude px-1 py-px text-[10px] whitespace-nowrap text-background">{cursor.label}</span>
          )}
        </span>
      )}
    </div>
  )
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between gap-2">
      <span className="text-muted-foreground">{k}</span>
      <span className="tabular-nums">{v}</span>
    </div>
  )
}

function Field({ label, value, focused, invalid }: { label: string; value: string; focused?: boolean; invalid?: boolean }) {
  return (
    <label className="block">
      <span className="text-[11px] text-muted-foreground">{label}</span>
      <span
        className={cn(
          'mt-0.5 flex h-[28px] items-center rounded-[5px] border bg-card px-2',
          focused ? 'border-foreground outline-2 outline-ring/60' : invalid ? 'border-deny' : 'border-input',
        )}
      >
        {value}
        {focused && <span className="ml-px inline-block h-3.5 w-px animate-pulse bg-foreground" />}
      </span>
    </label>
  )
}

/**
 * Ein auswählbares Element der Seite. Zeigt im Picker das Box-Model (Außenabstand, Innenabstand,
 * Inhalt), gewählte Elemente als nummerierte Marker, fremde Auswahl mit Namen.
 */
function Pick({
  hover,
  mark,
  remote,
  margin,
  padding,
  label,
  size,
  a11y,
  tipAbove,
  children,
}: {
  hover?: boolean
  mark?: number
  remote?: string
  margin: [number, number, number, number]
  padding: [number, number]
  label: string
  size: string
  a11y?: string
  tipAbove?: boolean
  children: ReactNode
}) {
  const [mt, mr, mb, ml] = margin
  const [py, px] = padding
  return (
    <div className="relative">
      {children}
      {hover && (
        <>
          {/* Außenabstand */}
          {mt + mb + ml + mr > 0 && (
            <span
              aria-hidden
              className="pointer-events-none absolute bg-voice-codex/25"
              style={{ top: 0, left: -ml, right: -mr, height: mt }}
            />
          )}
          <span
            aria-hidden
            className="pointer-events-none absolute border-voice-acp/45 bg-voice-direct/30"
            style={{
              top: mt,
              left: 0,
              bottom: 0,
              right: 0,
              borderTopWidth: py,
              borderBottomWidth: py,
              borderLeftWidth: px,
              borderRightWidth: px,
            }}
          />
          <span
            className={cn(
              'pointer-events-none absolute left-0 z-30 rounded-sm border border-border bg-card px-1.5 py-1 font-mono text-[10.5px] leading-snug whitespace-nowrap shadow-md',
              tipAbove ? 'bottom-full mb-1' : 'top-full mt-1',
            )}
          >
            <span className="font-semibold">{label}</span> <span className="text-muted-foreground">{size}</span>
            {a11y && <span className="block font-sans text-[10.5px] text-muted-foreground">{a11y}</span>}
          </span>
        </>
      )}
      {mark !== undefined && (
        <>
          <span aria-hidden className="pointer-events-none absolute -inset-0.5 rounded-[5px] outline-2 outline-foreground" style={{ top: mt - 2 }} />
          <span className="absolute -top-2 -left-2 z-30 flex size-4 items-center justify-center rounded-full bg-foreground text-[10px] font-semibold text-background" style={{ top: mt - 8 }}>
            {mark}
          </span>
        </>
      )}
      {remote && (
        <>
          <span aria-hidden className="pointer-events-none absolute -inset-0.5 rounded-[5px] outline-2 outline-dashed outline-voice-codex" style={{ top: mt - 2 }} />
          <span className="absolute -top-4 right-0 z-30 rounded-sm bg-voice-codex px-1 text-[10px] text-background" style={{ top: mt - 16 }}>
            {remote} wählt aus
          </span>
        </>
      )}
    </div>
  )
}

/** Legende zum Box-Model-Overlay – Farbe nie allein. */
export function BoxModelLegend() {
  return (
    <div className="flex items-center gap-3 text-[11px] text-muted-foreground">
      <span className="inline-flex items-center gap-1">
        <span className="size-2.5 bg-voice-direct/40" /> Inhalt
      </span>
      <span className="inline-flex items-center gap-1">
        <span className="size-2.5 bg-voice-acp/50" /> Innenabstand
      </span>
      <span className="inline-flex items-center gap-1">
        <span className="size-2.5 bg-voice-codex/35" /> Außenabstand
      </span>
    </div>
  )
}

/** Markierter Screenshot-Ausschnitt eines gewählten Elements. */
export function CropPreview({ children, label }: { children: ReactNode; label?: string }) {
  return (
    <figure className="w-fit">
      <div className="rounded-sm border border-border bg-popover p-3">
        <div className="rounded-[5px] outline-2 outline-offset-2 outline-voice-direct">{children}</div>
      </div>
      {label && <figcaption className="mt-1 text-[10.5px] text-muted-foreground">{label}</figcaption>}
    </figure>
  )
}
