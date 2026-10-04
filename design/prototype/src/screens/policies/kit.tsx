import type { ReactNode } from 'react'
import { Check, HardDrive, Lock, Minus, Server, X } from 'lucide-react'
import { AppLayout, type NavItem } from '@/app/app-layout'
import { SettingsFrame, type SettingsSectionId } from '@/app/settings-shell'
import { cn } from '@/lib/utils'

/**
 * Bausteine des Design-Pakets D-5 (Policies, Sandbox, Security, Team, Sync).
 * Nur von den Gruppen `policies`, `security`, `team` und `sync` genutzt.
 */

/* ------------------------------------------------------------------ Verdikte */

export type VerdictKind = 'deny' | 'ask' | 'allow' | 'modify' | 'notify' | 'error' | 'skip' | 'would_deny'

const verdictLabel: Record<VerdictKind, string> = {
  deny: 'abgelehnt',
  ask: 'fragt nach',
  allow: 'erlaubt',
  modify: 'ändert',
  notify: 'meldet',
  error: 'Fehler',
  skip: 'kein Treffer',
  would_deny: 'würde ablehnen',
}

/** Entscheidung als Form + Text (nie nur Farbe). */
export function Verdict({ v, label, className }: { v: VerdictKind; label?: string; className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-[12px] whitespace-nowrap', className)}>
      {v === 'deny' && <span className="size-2 rotate-45 bg-deny" />}
      {v === 'ask' && <span className="size-2.5 bg-foreground/75 [clip-path:polygon(50%_0,100%_100%,0_100%)]" />}
      {v === 'allow' && <span className="size-2 rounded-full bg-ok" />}
      {v === 'modify' && <span className="size-2 rounded-[1px] border-2 border-foreground/70" />}
      {v === 'notify' && <span className="size-2 rounded-full border-2 border-muted-foreground" />}
      {v === 'error' && <X className="size-3 text-deny" />}
      {v === 'skip' && <span className="h-px w-2 bg-muted-foreground" />}
      {v === 'would_deny' && <span className="size-2 rotate-45 border border-dashed border-deny" />}
      <span className={cn(v === 'deny' && 'text-deny', v === 'error' && 'text-deny', v === 'skip' && 'text-muted-foreground')}>
        {label ?? verdictLabel[v]}
      </span>
    </span>
  )
}

/** Ja/Nein/teilweise in Matrizen – mit Symbol und Text. */
export function Mark({ v, children }: { v: 'yes' | 'no' | 'partial'; children?: ReactNode }) {
  return (
    <span className="inline-flex items-start gap-1.5 text-[12px] leading-snug">
      {v === 'yes' && <Check className="mt-px size-3.5 shrink-0 text-ok" />}
      {v === 'no' && <X className="mt-px size-3.5 shrink-0 text-muted-foreground" />}
      {v === 'partial' && <Minus className="mt-px size-3.5 shrink-0 text-foreground" />}
      <span className={cn(v === 'no' && 'text-muted-foreground')}>{children ?? (v === 'yes' ? 'ja' : v === 'no' ? 'nein' : 'teilweise')}</span>
    </span>
  )
}

/* ------------------------------------------------------------------ Kleinteile */

export function Tag({ children, className, mono = true }: { children: ReactNode; className?: string; mono?: boolean }) {
  return (
    <span className={cn('inline-flex items-center rounded-sm border border-border px-1 text-[11px] leading-4 whitespace-nowrap text-muted-foreground', mono && 'font-mono', className)}>
      {children}
    </span>
  )
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="rounded-sm border border-border bg-card px-1 font-mono text-[10px]">{children}</kbd>
}

export function C({ children }: { children: ReactNode }) {
  return <code className="rounded-sm bg-muted px-1 font-mono text-[12px]">{children}</code>
}

/** Prämisse sichtbar machen: lokal, ohne Server, Subscription über CLI … */
export function Premise({ children, kind = 'local' }: { children: ReactNode; kind?: 'local' | 'team' | 'lock' }) {
  const Icon = kind === 'local' ? HardDrive : kind === 'team' ? Server : Lock
  return (
    <span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground">
      <Icon className="size-3.5 shrink-0" />
      <span>{children}</span>
    </span>
  )
}

/** Hinweisblock. `wait` = du bist dran (Gelb + Fase), `deny` = Fehler/abgelehnt. */
export function Callout({
  tone = 'neutral',
  title,
  children,
  actions,
  className,
}: {
  tone?: 'neutral' | 'deny' | 'ok' | 'wait'
  title: ReactNode
  children?: ReactNode
  actions?: ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        'border-l-4 p-3 text-[13px]',
        tone === 'neutral' && 'border-border bg-card',
        tone === 'deny' && 'border-deny bg-deny-soft',
        tone === 'ok' && 'border-ok bg-card',
        tone === 'wait' && 'chamfer border-signal bg-signal-soft',
        className,
      )}
    >
      <div className="flex items-start gap-2">
        {tone === 'deny' && <span className="mt-1.5 size-2 shrink-0 rotate-45 bg-deny" />}
        {tone === 'ok' && <Check className="mt-0.5 size-3.5 shrink-0 text-ok" />}
        {tone === 'wait' && <span className="chamfer-sm mt-1 size-2.5 shrink-0 bg-signal" />}
        <div className="min-w-0 flex-1">
          <div className="font-semibold">{title}</div>
          {children && <div className="mt-1 space-y-1.5 text-foreground/90">{children}</div>}
          {actions && <div className="mt-2.5 flex flex-wrap items-center gap-2">{actions}</div>}
        </div>
      </div>
    </div>
  )
}

export function PrimaryButton({ children, waiting }: { children: ReactNode; waiting?: boolean }) {
  return (
    <button
      className={cn(
        'inline-flex h-7 items-center gap-1.5 px-3 text-[13px] font-semibold',
        waiting ? 'chamfer-sm bg-foreground text-background' : 'rounded-md bg-foreground text-background',
      )}
    >
      {children}
    </button>
  )
}

export function Btn({ children, tone }: { children: ReactNode; tone?: 'deny' | 'quiet' }) {
  return (
    <button
      className={cn(
        'inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[13px] whitespace-nowrap',
        !tone && 'border border-foreground/25 hover:bg-accent',
        tone === 'deny' && 'border border-deny/50 text-deny hover:bg-deny-soft',
        tone === 'quiet' && 'text-muted-foreground hover:bg-accent hover:text-foreground',
      )}
    >
      {children}
    </button>
  )
}

/* ------------------------------------------------------------------ Layout */

export function PageHead({ title, sub, actions, children }: { title: string; sub?: ReactNode; actions?: ReactNode; children?: ReactNode }) {
  return (
    <div className="shrink-0 border-b border-border px-6 pt-4 pb-3">
      <div className="flex items-start gap-4">
        <div className="min-w-0 flex-1">
          <h2 className="type-wide text-[17px] font-[650]">{title}</h2>
          {sub && <div className="mt-0.5 text-[13px] text-muted-foreground">{sub}</div>}
        </div>
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </div>
      {children}
    </div>
  )
}

export function Section({ title, hint, actions, children, className }: { title: string; hint?: ReactNode; actions?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={cn('border-t border-border pt-3', className)}>
      <div className="mb-2 flex items-baseline gap-3">
        <h3 className="type-wide text-[13px] font-semibold">{title}</h3>
        {hint && <span className="text-[12px] text-muted-foreground">{hint}</span>}
        {actions && <div className="ml-auto flex items-center gap-2">{actions}</div>}
      </div>
      {children}
    </section>
  )
}

export function KV({ k, children, mono }: { k: ReactNode; children: ReactNode; mono?: boolean }) {
  return (
    <div className="flex gap-3 py-1 text-[13px]">
      <span className="w-44 shrink-0 text-muted-foreground">{k}</span>
      <span className={cn('min-w-0 flex-1', mono && 'font-mono text-[12px]')}>{children}</span>
    </div>
  )
}

/** Overlay innerhalb des Screens (statt Portal), damit Dialoge im Rahmen bleiben. */
export function InlineDialog({ title, children, footer, width = 'w-[520px]', waiting }: { title: string; children: ReactNode; footer?: ReactNode; width?: string; waiting?: boolean }) {
  return (
    <div className="absolute inset-0 z-40 flex items-center justify-center bg-foreground/25 p-6">
      <div role="dialog" aria-label={title} className={cn('max-h-full overflow-auto border border-border bg-popover shadow-xl', width, waiting ? 'chamfer border-t-4 border-t-signal' : 'rounded-lg')}>
        <div className="border-b border-border px-4 py-3">
          <h3 className="text-[15px] font-semibold">{title}</h3>
        </div>
        <div className="space-y-3 px-4 py-3 text-[13px]">{children}</div>
        {footer && <div className="flex items-center justify-end gap-2 border-t border-border px-4 py-2.5">{footer}</div>}
      </div>
    </div>
  )
}

/** Einstellungen mit Unternavigation links; Navigation und Sektionsliste aus `@/app/settings-shell`. */
export function SettingsLayout({
  active,
  children,
  connection = 'local',
  overlay,
}: {
  active: SettingsSectionId
  children: ReactNode
  connection?: 'local' | 'server' | 'offline'
  overlay?: ReactNode
}) {
  return (
    <SettingsFrame active={active} connection={connection} overlay={overlay}>
      {children}
    </SettingsFrame>
  )
}

/** Allgemeiner Wrapper mit Overlay-Unterstützung. */
export function Shell({ nav, children, overlay, connection = 'local', sessionList, activeSession }: { nav: NavItem; children: ReactNode; overlay?: ReactNode; connection?: 'local' | 'server' | 'offline'; sessionList?: boolean; activeSession?: string }) {
  return (
    <div className="relative h-full">
      <AppLayout nav={nav} connection={connection} sessionList={sessionList} activeSession={activeSession}>
        {children}
      </AppLayout>
      {overlay}
    </div>
  )
}

export function Scroll({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn('min-h-0 flex-1 overflow-y-auto', className)}>{children}</div>
}

/* ------------------------------------------------------------------ Code */

const ACTIONS = new Set(['deny', 'ask', 'allow', 'modify', 'notify'])

function highlightYamlLine(line: string): ReactNode {
  // Kommentar
  const hash = line.search(/(^|\s)#/)
  let code = line
  let comment = ''
  if (hash >= 0) {
    const i = line[hash] === '#' ? hash : hash + 1
    code = line.slice(0, i)
    comment = line.slice(i)
  }
  const m = code.match(/^(\s*-?\s*)([A-Za-z_][\w.-]*)(:)(.*)$/)
  let body: ReactNode = <Value text={code} />
  if (m) {
    body = (
      <>
        {m[1]}
        <span className="font-semibold">{m[2]}</span>
        {m[3]}
        <Value text={m[4]} keyName={m[2]} />
      </>
    )
  }
  return (
    <>
      {body}
      {comment && <span className="text-muted-foreground italic">{comment}</span>}
    </>
  )
}

function Value({ text, keyName }: { text: string; keyName?: string }) {
  const trimmed = text.trim()
  if (keyName === 'action' && ACTIONS.has(trimmed)) {
    return (
      <>
        {text.slice(0, text.indexOf(trimmed))}
        <span className={cn('font-semibold', trimmed === 'deny' && 'text-deny', trimmed === 'allow' && 'text-ok', (trimmed === 'ask' || trimmed === 'modify') && 'underline decoration-dotted underline-offset-2')}>{trimmed}</span>
      </>
    )
  }
  if (keyName === 'when' || keyName === 'match') {
    return <span className="text-foreground/85">{highlightCel(text)}</span>
  }
  // Strings in Anführungszeichen dezent absetzen
  const parts = text.split(/("[^"]*")/g)
  return (
    <>
      {parts.map((p, i) =>
        p.startsWith('"') ? (
          <span key={i} className="text-muted-foreground">
            {p}
          </span>
        ) : (
          <span key={i}>{p}</span>
        ),
      )}
    </>
  )
}

/** Sehr einfache CEL-Hervorhebung: Funktionen kursiv, Operatoren halbfett, Strings gedämpft. */
function highlightCel(text: string): ReactNode {
  const tokens = text.split(/("[^"]*"|&&|\|\||==|!=|>=|<=|\b[a-z_]+(?=\())/g)
  return tokens.map((t, i) => {
    if (!t) return null
    if (t.startsWith('"')) return <span key={i} className="text-muted-foreground">{t}</span>
    if (['&&', '||', '==', '!=', '>=', '<='].includes(t)) return <span key={i} className="font-semibold">{t}</span>
    if (/^[a-z_]+$/.test(t) && i % 2 === 1) return <span key={i} className="italic underline decoration-border underline-offset-2">{t}</span>
    return <span key={i}>{t}</span>
  })
}

export type CodeMark = { line: number; tone: 'deny' | 'changed' | 'focus'; note?: ReactNode; col?: number; len?: number }

/** Code mit Zeilennummern, YAML-Hervorhebung und Zeilenmarkierungen (Fehler, Änderung). */
export function CodeView({
  code,
  marks = [],
  file,
  lang = 'yaml',
  className,
  start = 1,
}: {
  code: string
  marks?: CodeMark[]
  file?: ReactNode
  lang?: 'yaml' | 'plain' | 'json'
  className?: string
  start?: number
}) {
  const lines = code.replace(/\n$/, '').split('\n')
  return (
    <div className={cn('overflow-hidden rounded-md border border-border bg-card font-mono text-[12px] leading-[1.6]', className)}>
      {file && <div className="flex items-center gap-2 border-b border-border bg-sunken px-3 py-1 text-[11px] text-muted-foreground">{file}</div>}
      <div className="overflow-x-auto py-1">
        {lines.map((l, idx) => {
          const n = idx + start
          const mark = marks.find((m) => m.line === n)
          return (
            <div key={idx}>
              <div className={cn('flex', mark?.tone === 'deny' && 'bg-deny-soft', mark?.tone === 'changed' && 'bg-ok-soft', mark?.tone === 'focus' && 'bg-accent')}>
                <span className="w-10 shrink-0 pr-3 text-right text-muted-foreground select-none">{n}</span>
                <span className="pr-4 whitespace-pre">
                  {mark?.tone === 'deny' && mark.col !== undefined ? (
                    <>
                      {lang === 'yaml' ? highlightYamlLine(l.slice(0, mark.col)) : l.slice(0, mark.col)}
                      <span className="underline decoration-deny decoration-wavy underline-offset-4">{l.slice(mark.col, mark.col + (mark.len ?? 1))}</span>
                      {l.slice(mark.col + (mark.len ?? 1))}
                    </>
                  ) : lang === 'yaml' ? (
                    highlightYamlLine(l)
                  ) : (
                    l
                  )}
                </span>
              </div>
              {mark?.note && (
                <div className={cn('ml-10 border-l-2 px-2 py-1 font-sans text-[12px]', mark.tone === 'deny' ? 'border-deny bg-deny-soft' : 'border-border bg-sunken')}>
                  {mark.note}
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}

/** Browser-Fenster im Desktop-Rahmen (für Login-, Pairing- und Einmal-Link-Seiten). */
export function BrowserChrome({ url, children, secure = true, overlay }: { url: string; children: ReactNode; secure?: boolean; overlay?: ReactNode }) {
  return (
    <div className="relative flex h-full flex-col bg-background">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-sidebar px-3">
        <span className="flex gap-1 text-muted-foreground" aria-hidden>
          ‹ ›
        </span>
        <div className="flex h-7 flex-1 items-center gap-2 rounded-md border border-input bg-card px-2 font-mono text-[12px]">
          {secure ? <Lock className="size-3 text-muted-foreground" /> : <span className="text-[11px] text-muted-foreground">http</span>}
          <span className="truncate">{url}</span>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">{children}</div>
      {overlay}
    </div>
  )
}

/** Einfaches QR-artiges Muster (nur Darstellung). */
export function FakeQr({ size = 168, seed = 7 }: { size?: number; seed?: number }) {
  const n = 25
  const cells: boolean[] = []
  let x = seed * 9301 + 49297
  for (let i = 0; i < n * n; i++) {
    x = (x * 9301 + 49297) % 233280
    cells.push(x / 233280 > 0.52)
  }
  const finder = (r: number, c: number) => {
    const inBox = (r0: number, c0: number) => r >= r0 && r < r0 + 7 && c >= c0 && c < c0 + 7
    for (const [r0, c0] of [
      [0, 0],
      [0, n - 7],
      [n - 7, 0],
    ]) {
      if (inBox(r0, c0)) {
        const rr = r - r0
        const cc = c - c0
        return rr === 0 || rr === 6 || cc === 0 || cc === 6 || (rr >= 2 && rr <= 4 && cc >= 2 && cc <= 4)
      }
    }
    return null
  }
  const s = size / n
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img" aria-label="QR-Code zum Koppeln" className="bg-card p-0">
      {cells.map((on, i) => {
        const r = Math.floor(i / n)
        const c = i % n
        const f = finder(r, c)
        const fill = f === null ? on : f
        return fill ? <rect key={i} x={c * s} y={r * s} width={s} height={s} fill="var(--foreground)" /> : null
      })}
    </svg>
  )
}
