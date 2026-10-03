import type { ReactNode } from 'react'
import { Check, Crosshair, FileCode, X } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { Composer, SessionHeader, WorkspaceRail } from '@/app/session-chrome'
import { AgentMessage, Diff, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { BoxModelLegend, BrowserPanel, CheckoutPage, CropPreview } from './parts'

/*
 * Inspect-Mode / Element-Picker (BRW-013 bis BRW-016): Element zeigen statt beschreiben.
 * Der Picker ist „du bist dran“ – daher Schalungsgelb und Fase nur am Picker-Banner,
 * am Picker-Button und an der Kommentar-Karte.
 */

const SUBMIT_HTML = '<div data-testid="checkout-submit" class="btn btn-secondary mt-4">Jetzt kaufen</div>'

function ButtonCopy({ small }: { small?: boolean }) {
  return (
    <span className={cn('inline-flex rounded-[5px] bg-muted font-medium', small ? 'px-2 py-1 text-[10px]' : 'px-4 py-2 text-[12px]')}>
      Jetzt kaufen
    </span>
  )
}

function PickerBanner({ request }: { request?: string }) {
  return (
    <F id="BRW-013" className="chamfer shrink-0 border-l-4 border-signal bg-signal-soft px-3 py-2 text-[12px]">
      {request ? (
        <p>
          <span className="font-semibold">Claude Code bittet dich um eine Auswahl:</span> „{request}“
        </p>
      ) : (
        <p className="font-semibold">Wähle ein Element aus. Klicks lösen auf der Seite nichts aus.</p>
      )}
      <p className="mt-0.5 text-muted-foreground">
        <kbd className="font-mono">⇧</kbd> + Klick wählt mehrere · <kbd className="font-mono">Esc</kbd> beendet den Picker ohne Auswahl
      </p>
    </F>
  )
}

function CommentCard({ multi }: { multi?: boolean }) {
  return (
    <F
      id={multi ? ['BRW-013', 'BRW-015'] : 'BRW-013'}
      className="chamfer absolute z-40 w-[300px] border-l-4 border-signal bg-popover p-3 shadow-lg"
      badge="top-right"
    >
      <div className="text-[11px] text-muted-foreground">
        {multi ? '3 Elemente gewählt · gemeinsamer Kommentar' : '1 · div „Jetzt kaufen“'}
      </div>
      <div className="mt-1.5 min-h-16 rounded-md border border-ring bg-card px-2 py-1.5 text-[12.5px] leading-snug">
        {multi
          ? 'Die Fehlermeldung zur PLZ wird von Screenreadern nicht vorgelesen, und „Jetzt kaufen“ ist per Tastatur nicht erreichbar.'
          : 'Soll ein echter <button type="submit"> werden, rechtsbündig neben „Abbrechen“ und primär gefärbt.'}
        <span className="ml-px inline-block h-3.5 w-px animate-pulse bg-foreground align-text-bottom" />
      </div>
      <div className="mt-2 flex items-center gap-2">
        <button className="chamfer-sm bg-foreground px-2.5 py-1 text-[12px] font-semibold text-background">In den Composer legen</button>
        <button className="rounded-md border border-border px-2 py-1 text-[12px]">Direkt einreihen</button>
      </div>
      <div className="mt-1.5 text-[10.5px] text-muted-foreground">
        <kbd className="font-mono">⌘↵</kbd> Composer · <kbd className="font-mono">Esc</kbd> verwirft die Auswahl
      </div>
    </F>
  )
}

function KV({ k, children }: { k: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[92px_1fr] gap-2 py-0.5">
      <dt className="text-muted-foreground">{k}</dt>
      <dd className="min-w-0">{children}</dd>
    </div>
  )
}

/** Vorschau dessen, was an den Agent geht (BRW-014), inkl. Quelldatei (BRW-016). */
function PayloadDrawer() {
  return (
    <F id="BRW-014" className="max-h-[46%] shrink-0 overflow-y-auto border-t border-border bg-card" badge="top-left">
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-[12px]">
        <span className="font-semibold">Das bekommt der Agent</span>
        <span className="text-muted-foreground">Text und Bildausschnitt · bleibt in dieser Session</span>
        <button className="ml-auto rounded-sm px-1.5 text-[11px] text-muted-foreground hover:bg-accent">Als JSON (browser.pick v1)</button>
      </div>
      <div className="grid grid-cols-[auto_minmax(0,1fr)_minmax(0,1fr)] gap-4 p-3 text-[12px]">
        <CropPreview label="Ausschnitt 136 × 66 px, markiert">
          <ButtonCopy />
        </CropPreview>
        <dl>
          <KV k="Selektor">
            <code className="font-mono text-[11.5px] break-all">[data-testid="checkout-submit"]</code>
            <span className="ml-1.5 inline-flex items-center gap-0.5 text-[11px] text-ok">
              <Check className="size-3" /> eindeutig
            </span>
          </KV>
          <KV k="Alternativen">
            <code className="block truncate font-mono text-[11px] text-muted-foreground">form#checkout-form &gt; div.actions &gt; div:nth-of-type(1)</code>
          </KV>
          <KV k="Rolle">generic „Jetzt kaufen“ · nicht fokussierbar</KV>
          <KV k="Box">104 × 34 bei x 24, y 412 · DPR 2</KV>
          <KV k="Eltern-Layout">
            <code className="font-mono text-[11px]">div.actions</code> flex, justify-content: flex-start, gap 8px
          </KV>
          <F id="BRW-016" as="div" badge="bottom-left">
            <KV k="Quelldatei">
              <span className="inline-flex items-center gap-1 font-mono text-[11.5px] font-medium break-all">
                <FileCode className="size-3.5 shrink-0" /> src/components/CheckoutButton.tsx:42
              </span>
              <span className="block text-[11px] text-muted-foreground">CheckoutButton · React 19 · über Sourcemap zugeordnet</span>
            </KV>
          </F>
        </dl>
        <div className="min-w-0 space-y-2">
          <div>
            <div className="text-[11px] text-muted-foreground">outerHTML (91 Bytes, vollständig)</div>
            <pre className="mt-0.5 overflow-x-auto rounded-sm bg-sunken px-2 py-1 font-mono text-[11px] whitespace-pre-wrap">{SUBMIT_HTML}</pre>
          </div>
          <div>
            <div className="text-[11px] text-muted-foreground">Berechnete Styles (Auszug)</div>
            <pre className="mt-0.5 rounded-sm bg-sunken px-2 py-1 font-mono text-[11px] leading-relaxed">{`display: inline-flex
margin: 16px 0 0 0;  padding: 8px 16px
background-color: rgb(229, 231, 235)
font: 500 14px Inter;  border-radius: 6px`}</pre>
          </div>
        </div>
      </div>
    </F>
  )
}

/** Mehrfachauswahl: nummerierte Elemente mit Einzelkommentaren (BRW-015). */
function MultiDrawer() {
  const items = [
    { n: 1, el: 'div „Jetzt kaufen“', sel: '[data-testid="checkout-submit"]', src: 'src/components/CheckoutButton.tsx:42', note: 'Echter Button, per Tab erreichbar' },
    { n: 2, el: 'span.error „Bitte gib eine fünfstellige PLZ ein.“', sel: '#zip-error', src: undefined, note: 'Mit aria-describedby am PLZ-Feld verknüpfen' },
    { n: 3, el: 'input#zip', sel: '#zip', src: 'src/features/checkout/AddressFields.tsx:57', note: '' },
  ]
  return (
    <F id={['BRW-015', 'BRW-014']} className="max-h-[46%] shrink-0 overflow-y-auto border-t border-border bg-card" badge="top-left">
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-[12px]">
        <span className="font-semibold">3 Elemente in einer Auswahl</span>
        <span className="text-muted-foreground">⇧ + Klick auf ein gewähltes Element nimmt es wieder heraus</span>
      </div>
      <ol className="divide-y divide-border">
        {items.map((it) => (
          <li key={it.n} className="grid grid-cols-[20px_1fr_1fr_20px] items-start gap-3 px-3 py-2 text-[12px]">
            <span className="flex size-4 items-center justify-center rounded-full bg-foreground text-[10px] font-semibold text-background">{it.n}</span>
            <div className="min-w-0">
              <div className="truncate font-medium">{it.el}</div>
              <code className="block truncate font-mono text-[11px] text-muted-foreground">{it.sel}</code>
              <F id="BRW-016" as="div" badge="bottom-right">
                {it.src ? (
                  <span className="inline-flex items-center gap-1 font-mono text-[11px]">
                    <FileCode className="size-3" /> {it.src}
                  </span>
                ) : (
                  <span className="text-[11px] text-muted-foreground">Keine Quelldatei gefunden – der Agent bekommt Selektor und HTML</span>
                )}
              </F>
            </div>
            <div className={cn('rounded-md border border-input bg-background px-2 py-1 text-[12px]', !it.note && 'text-muted-foreground')}>
              {it.note || 'Eigener Kommentar (optional)'}
            </div>
            <button aria-label={`Element ${it.n} entfernen`} className="mt-0.5 text-muted-foreground hover:text-foreground">
              <X className="size-3.5" />
            </button>
          </li>
        ))}
      </ol>
    </F>
  )
}

/** Picker-Block im Composer (BRW-013 AC2). */
function PickerAttachment() {
  return (
    <F id={['BRW-013', 'BRW-014']} className="mx-3 mt-2 flex items-start gap-3 rounded-md border border-border bg-card p-2" badge="top-right">
      <div className="shrink-0 rounded-sm border border-border bg-popover p-1.5">
        <div className="rounded-[4px] outline-2 outline-offset-1 outline-voice-direct">
          <ButtonCopy small />
        </div>
      </div>
      <div className="min-w-0 flex-1 text-[12px]">
        <div className="flex items-center gap-1.5">
          <Crosshair className="size-3.5 text-muted-foreground" />
          <span className="font-medium">Auswahl aus localhost:5173/checkout</span>
          <span className="text-muted-foreground">· 1 Element</span>
        </div>
        <div className="mt-0.5 truncate font-mono text-[11px] text-muted-foreground">
          div „Jetzt kaufen“ · src/components/CheckoutButton.tsx:42
        </div>
        <div className="mt-0.5 truncate">Soll ein echter &lt;button type="submit"&gt; werden, rechtsbündig neben „Abbrechen“ und primär gefärbt.</div>
      </div>
      <button aria-label="Auswahl entfernen" className="text-muted-foreground hover:text-foreground">
        <X className="size-3.5" />
      </button>
    </F>
  )
}

export function BrowserInspect({ state }: { state: string }) {
  const picker = state === 'hover' || state === 'selected' || state === 'multi' || state === 'agent-request'
  const marks =
    state === 'selected' ? { submit: 1 } : state === 'multi' ? { submit: 1, error: 2, zip: 3 } : state === 'composer' ? { submit: 1 } : undefined
  const hover = state === 'hover' ? 'submit' : state === 'agent-request' ? 'zip' : undefined

  return (
    <AppLayout
      sessionList={false}
      activeSession="ses_6q2a"
      rail={
        <WorkspaceRail active="browser" width="w-[min(680px,52%)]">
          <BrowserPanel
            url="http://localhost:5173/checkout"
            picker={picker ? 'on' : 'off'}
            banner={
              state === 'hover' ? (
                <PickerBanner />
              ) : state === 'agent-request' ? (
                <PickerBanner request="Zeig mir das Feld, dessen Fehlermeldung nicht vorgelesen wird." />
              ) : undefined
            }
            drawer={
              state === 'selected' ? (
                <PayloadDrawer />
              ) : state === 'multi' ? (
                <MultiDrawer />
              ) : state === 'hover' || state === 'agent-request' ? (
                <div className="flex h-8 shrink-0 items-center gap-3 border-t border-border bg-card px-3">
                  <BoxModelLegend />
                  <span className="ml-auto text-[11px] text-muted-foreground">Overlay zeichnet Chromium direkt ins Bild</span>
                </div>
              ) : undefined
            }
          >
            <CheckoutPage hover={hover} marks={marks} />
            {state === 'selected' && (
              <div className="pointer-events-none absolute inset-0">
                <div className="pointer-events-auto absolute top-[46%] left-[34%]">
                  <CommentCard />
                </div>
              </div>
            )}
            {state === 'multi' && (
              <div className="pointer-events-none absolute inset-0">
                <div className="pointer-events-auto absolute top-[48%] left-[36%]">
                  <CommentCard multi />
                </div>
              </div>
            )}
          </BrowserPanel>
        </WorkspaceRail>
      }
    >
      <SessionHeader
        title="Checkout-Formular barrierefrei machen"
        harness="claude"
        status={state === 'agent-request' ? 'waiting' : state === 'sent' ? 'running' : 'idle'}
        branch="beton/checkout-a11y-6q2a"
      />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-2xl flex-col gap-4 px-5 py-5">
          <UserMessage>Prüf die Kasse auf Tastaturbedienung.</UserMessage>
          <AgentMessage harness="claude">
            <p>
              Zwei Probleme: „Jetzt kaufen“ ist ein <code className="rounded-sm bg-muted px-1 text-[13px]">div</code> mit Click-Handler und per
              Tab nicht erreichbar, und die PLZ-Fehlermeldung ist nicht mit dem Feld verknüpft.
            </p>
          </AgentMessage>
          {state === 'agent-request' && (
            <F id="BRW-013">
              <ToolCall
                kind="other"
                name="browser_request_pick"
                target="„Zeig mir das Feld, dessen Fehlermeldung nicht vorgelesen wird.“"
                status="waiting"
                sandbox={false}
              />
            </F>
          )}
          {state === 'sent' && (
            <>
              <UserMessage>
                <span className="mb-1.5 flex w-fit items-center gap-2 rounded-md border border-border bg-card px-2 py-1 text-[12px]">
                  <Crosshair className="size-3.5 text-muted-foreground" />
                  div „Jetzt kaufen“ · <span className="font-mono text-[11px]">src/components/CheckoutButton.tsx:42</span>
                </span>
                Soll ein echter &lt;button type="submit"&gt; werden, rechtsbündig neben „Abbrechen“ und primär gefärbt.
              </UserMessage>
              <ToolCall kind="read" name="Lesen" target="src/components/CheckoutButton.tsx:30-58" duration="0,1 s" />
              <ToolCall kind="edit" name="Bearbeiten" target="src/components/CheckoutButton.tsx" duration="0,3 s" defaultOpen>
                <Diff
                  file="src/components/CheckoutButton.tsx"
                  lines={[
                    { kind: 'del', n: 42, text: '    <div data-testid="checkout-submit" className="btn btn-secondary mt-4" onClick={submit}>' },
                    { kind: 'add', n: 42, text: '    <button type="submit" data-testid="checkout-submit" className="btn btn-primary">' },
                    { kind: 'ctx', n: 43, text: '      Jetzt kaufen' },
                    { kind: 'del', n: 44, text: '    </div>' },
                    { kind: 'add', n: 44, text: '    </button>' },
                  ]}
                />
              </ToolCall>
              <ToolCall kind="other" name="browser_screenshot" target="ref e47 · zur Kontrolle" status="running" sandbox={false} />
            </>
          )}
        </div>
      </div>
      {state === 'composer' && <PickerAttachment />}
      <Composer
        harness="claude"
        running={state === 'sent'}
        draft={state === 'composer' ? 'Bitte umsetzen und danach im Browser per Tab prüfen.' : undefined}
      />
    </AppLayout>
  )
}
