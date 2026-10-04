import type { ReactNode } from 'react'
import { AppWindow, Check, Container, Download, ExternalLink, FolderOpen, RotateCw, ShieldAlert, TriangleAlert, X } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { Composer, SessionHeader, WorkspaceRail } from '@/app/session-chrome'
import { AgentMessage, ApprovalCard, SystemNote, TerminalOutput, ToolCall, UserMessage } from '@/app/stream'
import { SettingsFrame } from '@/app/settings-shell'
import { Progress } from '@/components/ui/progress'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import type { SessionStatus } from '@/mock/data'
import { cn } from '@/lib/utils'
import { BrowserPanel, CheckoutPage, PanelFooter } from './parts'
import { BrowserInspect } from './inspect'

/*
 * Gruppe „Browser“ (Kapitel 09): eingebetteter, agent-steuerbarer Chromium pro Session.
 * Alles läuft auf deinem Rechner; Netzwerk nur über den Egress-Proxy der Session.
 * Der einzige Download (Chrome for Testing) passiert nur auf Klick.
 */

const SESSION_TITLE = 'Checkout-Formular barrierefrei machen'
const BRANCH = 'beton/checkout-a11y-6q2a'

function BrowserSession({
  status,
  rail,
  railWidth = 'w-[min(560px,46%)]',
  children,
  running,
}: {
  status: SessionStatus
  rail: ReactNode
  railWidth?: string
  children: ReactNode
  running?: boolean
}) {
  return (
    <AppLayout
      sessionList={false}
      activeSession="ses_6q2a"
      rail={
        <WorkspaceRail active="browser" width={railWidth}>
          {rail}
        </WorkspaceRail>
      }
    >
      <SessionHeader title={SESSION_TITLE} harness="claude" status={status} branch={BRANCH} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-2xl flex-col gap-3.5 px-5 py-5">{children}</div>
      </div>
      <Composer harness="claude" running={running} />
    </AppLayout>
  )
}

/** Mittige Meldung im Viewport, z. B. „noch nicht gestartet“. */
function ViewportMessage({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn('concrete-grain flex h-full flex-col items-center justify-center gap-2 p-6 text-center', className)}>{children}</div>
  )
}

const SNAPSHOT = `- banner
  - link "Acme Shop" [ref=e2]
- main
  - heading "Kasse" [level=1] [ref=e5]
  - textbox "E-Mail" [ref=e12]
  - textbox "Straße und Hausnummer" [ref=e14]
  - textbox "PLZ" [ref=e15] (invalid=true)
  - textbox "Ort" [ref=e16]
  - clickable "Jetzt kaufen" [ref=e47]
  - clickable "Abbrechen" [ref=e48]
… 118 weitere Knoten (scope_ref verwenden)`

/* ------------------------------------------------------------------------------------------ */
/* Browser-Panel                                                                               */
/* ------------------------------------------------------------------------------------------ */

function BrowserPanelScreen({ state }: { state: string }) {
  if (state === 'devserver') {
    return (
      <BrowserSession
        status="idle"
        rail={
          <BrowserPanel url={<span className="text-muted-foreground">Adresse eingeben oder Link aus dem Chat öffnen</span>} tabs={[{ id: 't', title: 'Neuer Tab', url: '' }]} activeTab="t" footer={<PanelFooter quality={null} viewers={['Desktop']} />}>
            <F id="BRW-003" className="h-full">
              <ViewportMessage>
                <p className="type-wide text-[15px] font-[650]">Der Browser ist noch nicht gestartet</p>
                <p className="max-w-sm text-[13px] text-muted-foreground">
                  Er startet erst, wenn du ihn öffnest oder der Agent ihn braucht, und beendet sich nach 15 Minuten ohne Zuschauer und
                  ohne Agent-Aufruf. Verwendet wird dein installiertes Google Chrome 141.
                </p>
                <F id="BRW-017" className="mt-2 flex items-center gap-2">
                  <button className="rounded-md bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">localhost:5173 öffnen</button>
                  <button className="rounded-md border border-border px-3 py-1.5 text-[13px]">Leeren Tab öffnen</button>
                </F>
              </ViewportMessage>
            </F>
          </BrowserPanel>
        }
      >
        <UserMessage>Starte den Dev-Server und zeig mir die Kasse.</UserMessage>
        <ToolCall kind="shell" name="Shell" target="pnpm dev" status="running" defaultOpen>
          <TerminalOutput>{`  VITE v7.1.4  ready in 412 ms

  ➜  Local:   http://localhost:5173/
  ➜  Network: use --host to expose`}</TerminalOutput>
        </ToolCall>
        <F id="BRW-017" className="ml-9 flex flex-wrap items-center gap-2 rounded-md border border-border bg-card px-3 py-2 text-[12.5px]">
          <span className="size-2 rounded-full bg-ok" aria-hidden />
          <span className="font-medium">Dev-Server erkannt: localhost:5173</span>
          <span className="text-muted-foreground">pnpm dev · PID 48121</span>
          <button className="ml-auto inline-flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-[12px] hover:bg-accent">
            <ExternalLink className="size-3" /> Im Session-Browser öffnen
          </button>
          <p className="w-full text-[11.5px] text-muted-foreground">
            Nur dieser Port ist für den Session-Browser freigegeben, solange der Prozess läuft.
          </p>
        </F>
        <AgentMessage harness="claude">
          <p>
            Der Dev-Server läuft. Die Kasse findest du unter{' '}
            <F id="BRW-009" as="span" badge="bottom-right">
              <span className="underline decoration-dotted underline-offset-2">http://localhost:5173/checkout</span>{' '}
              <button className="ml-1 inline-flex items-center gap-1 rounded-sm border border-border px-1.5 align-middle text-[11.5px] hover:bg-accent">
                <AppWindow className="size-3" /> Im Session-Browser öffnen
              </button>
            </F>
          </p>
        </AgentMessage>
      </BrowserSession>
    )
  }

  if (state === 'no-browser') {
    return (
      <BrowserSession
        status="waiting"
        rail={
          <BrowserPanel url={<span className="text-muted-foreground">localhost:5173/checkout</span>} footer={<PanelFooter quality={null} viewers={['Desktop']} />}>
            <ViewportMessage>
              <F id={['BRW-001', 'BRW-002']} className="chamfer w-full max-w-md border-l-4 border-signal bg-signal-soft p-4 text-left">
                <p className="text-[14px] font-semibold">Kein passender Browser gefunden</p>
                <p className="mt-1 text-[12.5px]">
                  beton hat nach Chrome, Chromium, Edge und Brave gesucht. Gefunden wurde nur Microsoft Edge 118 – zu alt, nötig ist
                  mindestens Chromium 120.
                </p>
                <div className="mt-3 space-y-2 text-[12.5px]">
                  <div className="rounded-md border border-border bg-card p-2.5">
                    <p className="font-medium">Chrome for Testing 141.0.7390.54 laden</p>
                    <p className="text-muted-foreground">
                      168 MB von storage.googleapis.com (Google) · nach ~/.beton/browser/cft/ · geprüft per SHA-256
                    </p>
                    <button className="chamfer-sm mt-2 inline-flex items-center gap-1 bg-foreground px-3 py-1.5 text-[12.5px] font-semibold text-background">
                      <Download className="size-3.5" /> Chrome for Testing laden (168 MB)
                    </button>
                  </div>
                  <div className="flex items-center gap-2">
                    <button className="inline-flex items-center gap-1 rounded-md border border-foreground/30 bg-card px-3 py-1.5">
                      <FolderOpen className="size-3.5" /> Installierten Browser angeben…
                    </button>
                    <span className="text-[11.5px] text-muted-foreground">ohne Download</span>
                  </div>
                </div>
                <p className="mt-3 text-[11.5px] text-muted-foreground">
                  Ohne deine Zustimmung wird nichts geladen. Der Agent bekommt bis dahin <code className="font-mono">browser_unavailable</code>.
                </p>
              </F>
            </ViewportMessage>
          </BrowserPanel>
        }
      >
        <UserMessage>Schau dir die Kasse im Browser an.</UserMessage>
        <F id="BRW-002">
          <ToolCall kind="other" name="browser_navigate" target="localhost:5173/checkout → browser_unavailable" status="waiting" sandbox={false} />
        </F>
        <SystemNote>Wartet auf deine Entscheidung im Browser-Panel</SystemNote>
      </BrowserSession>
    )
  }

  if (state === 'crashed') {
    return (
      <BrowserSession
        status="running"
        running
        rail={
          <BrowserPanel url="http://localhost:5173/checkout" footer={<PanelFooter quality={null} />}>
            <F id="BRW-003" className="h-full">
              <ViewportMessage>
                <TriangleAlert className="size-5 text-deny" aria-hidden />
                <p className="text-[14px] font-semibold">Chromium wurde unerwartet beendet</p>
                <p className="max-w-sm text-[12.5px] text-muted-foreground">
                  Beim nächsten Schritt startet beton ihn mit demselben Profil neu und öffnet localhost:5173/checkout wieder. Anmeldungen
                  und Cookies bleiben erhalten.
                </p>
                <button className="mt-1 inline-flex items-center gap-1 rounded-md border border-border bg-card px-3 py-1.5 text-[13px]">
                  <RotateCw className="size-3.5" /> Jetzt neu starten
                </button>
              </ViewportMessage>
            </F>
          </BrowserPanel>
        }
      >
        <UserMessage>Klick auf „Jetzt kaufen“ und schau, was passiert.</UserMessage>
        <ToolCall kind="other" name="browser_snapshot" target="142 Knoten" duration="0,2 s" sandbox={false} />
        <F id="BRW-003">
          <SystemNote tone="deny">Browser abgestürzt (Signal 9) · Profil bleibt erhalten</SystemNote>
        </F>
        <ToolCall kind="other" name="browser_click" target="ref e47 · Browser startet neu mit gleichem Profil" status="running" sandbox={false} />
      </BrowserSession>
    )
  }

  const slow = state === 'slow'
  const viewer = state === 'viewer'
  return (
    <BrowserSession
      status="running"
      running
      rail={
        <BrowserPanel
          url="http://localhost:5173/checkout"
          driving={viewer ? undefined : 'browser_press Tab (3 von 4)'}
          banner={
            viewer ? (
              <F id="BRW-020" className="flex shrink-0 items-center gap-2 border-b border-border bg-card px-3 py-1.5 text-[12px]">
                <span className="font-medium">Du schaust zu.</span>
                <span className="text-muted-foreground">Ingo steuert vom Desktop; deine Eingaben im Browser sind gesperrt.</span>
              </F>
            ) : undefined
          }
          footer={
            <PanelFooter
              quality={slow ? 'niedrig · 5 fps · Verbindung langsam' : undefined}
              qualityLow={slow}
              viewers={viewer ? ['Ingo · Desktop (steuert)', 'Ingo · Web', 'Mara · Web (zuschauen)'] : undefined}
            />
          }
        >
          <CheckoutPage
            typing={!viewer}
            remoteHover={viewer ? { target: 'submit', who: 'Ingo' } : undefined}
            cursor={viewer ? undefined : { x: '66%', y: '30%', label: 'Claude Code' }}
            className={slow ? 'blur-[0.7px]' : undefined}
          />
          {slow && (
            <F id="BRW-007" className="absolute top-3 right-3 rounded-md border border-border bg-card px-2 py-1 text-[11.5px] shadow-sm">
              Verbindung langsam – Bild in niedriger Qualität, Eingaben gehen weiter
            </F>
          )}
        </BrowserPanel>
      }
    >
      <UserMessage>Prüf im Session-Browser, ob man die Kasse komplett per Tastatur bedienen kann. Der Dev-Server läuft schon.</UserMessage>
      <F id="BRW-010" className="flex flex-col gap-1">
        <ToolCall kind="other" name="browser_navigate" target="localhost:5173/checkout → 200 · „Kasse – Acme Shop“" duration="0,4 s" sandbox={false} />
        <F id="BRW-011">
          <ToolCall kind="other" name="browser_snapshot" target="142 Knoten, refs stabil bis zur nächsten Navigation" duration="0,2 s" sandbox={false} defaultOpen>
            <TerminalOutput>{SNAPSHOT}</TerminalOutput>
          </ToolCall>
        </F>
        <ToolCall kind="other" name="browser_type" target='ref e12 · "test@example.com"' duration="0,3 s" sandbox={false} />
        <ToolCall kind="other" name="browser_press" target="Tab × 4" status="running" sandbox={false} />
      </F>
      <AgentMessage harness="claude" streaming>
        <p>
          „Jetzt kaufen“ und „Abbrechen“ erscheinen im Snapshot nur als <em>clickable</em> – es sind{' '}
          <code className="rounded-sm bg-muted px-1 text-[13px]">div</code>-Elemente mit Click-Handler. Per Tab springt der Fokus vom
          Ort-Feld direkt zum Footer
        </p>
      </AgentMessage>
    </BrowserSession>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Sichtbares Fenster & Remote-Runner                                                          */
/* ------------------------------------------------------------------------------------------ */

function BrowserWindowScreen({ state }: { state: string }) {
  const remote = state === 'remote'
  return (
    <BrowserSession
      status="idle"
      rail={
        <BrowserPanel
          url="http://localhost:5173/checkout"
          mode={state === 'window' ? 'window' : 'embedded'}
          windowUnavailable={remote ? 'Nicht verfügbar: Die Session läuft im Docker-Runner „build-02“ ohne Display.' : undefined}
          banner={
            state === 'window' ? (
              <F id="BRW-008" className="flex shrink-0 items-center gap-2 border-b border-border bg-card px-3 py-2 text-[12px]">
                <AppWindow className="size-4 shrink-0 text-muted-foreground" />
                <span>
                  <span className="font-medium">Läuft als eigenes Fenster auf diesem Rechner</span>
                  <span className="text-muted-foreground"> – zum Anmelden oder für die DevTools. Der Agent arbeitet weiter, dieses Bild auch.</span>
                </span>
                <button className="ml-auto shrink-0 rounded-md border border-border px-2 py-1 hover:bg-accent">Zurück ins Panel</button>
              </F>
            ) : remote ? (
              <F id="BRW-021" className="flex shrink-0 items-center gap-2 border-b border-border bg-card px-3 py-2 text-[12px]">
                <Container className="size-4 shrink-0 text-muted-foreground" />
                <span>
                  <span className="font-medium">Browser im Container „build-02“</span>
                  <span className="text-muted-foreground"> · Bild und Eingaben laufen über den Runner-Tunnel · nur eingebettet, kein Fenster</span>
                </span>
              </F>
            ) : undefined
          }
        >
          <CheckoutPage />
          {state === 'confirm' && (
            <div className="absolute inset-0 flex items-center justify-center bg-background/60 p-6">
              <F id={['BRW-008', 'BRW-004']} className="chamfer w-full max-w-sm border-l-4 border-signal bg-popover p-4 shadow-lg" badge="top-right">
                <p className="text-[14px] font-semibold">Im sichtbaren Fenster öffnen?</p>
                <p className="mt-1 text-[12.5px] text-muted-foreground">
                  Chromium startet dafür mit demselben Profil neu. Danach kannst du dich anmelden oder die DevTools öffnen.
                </p>
                <div className="mt-3 grid grid-cols-2 gap-3 text-[12px]">
                  <div>
                    <p className="flex items-center gap-1 font-medium">
                      <Check className="size-3.5 text-ok" /> Bleibt erhalten
                    </p>
                    <ul className="mt-1 space-y-0.5 text-muted-foreground">
                      <li>Anmeldungen und Cookies</li>
                      <li>LocalStorage, IndexedDB</li>
                      <li>3 Tabs mit Scroll-Position</li>
                    </ul>
                  </div>
                  <div>
                    <p className="flex items-center gap-1 font-medium">
                      <X className="size-3.5 text-deny" /> Geht verloren
                    </p>
                    <ul className="mt-1 space-y-0.5 text-muted-foreground">
                      <li>Eingaben im Formular „Kasse“</li>
                      <li>sessionStorage</li>
                      <li>laufender JavaScript-Zustand</li>
                    </ul>
                  </div>
                </div>
                <div className="mt-4 flex gap-2">
                  <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[12.5px] font-semibold text-background">Im Fenster neu starten</button>
                  <button className="rounded-md border border-border px-3 py-1.5 text-[12.5px]">Abbrechen</button>
                </div>
              </F>
            </div>
          )}
        </BrowserPanel>
      }
    >
      <UserMessage>Ich muss mich im Testkonto anmelden, bevor du weitermachst.</UserMessage>
      {state === 'window' && (
        <F id="BRW-008">
          <SystemNote>Browser im Fenster neu gestartet · 3 Tabs wiederhergestellt · weiterhin angemeldet</SystemNote>
        </F>
      )}
      {remote && (
        <F id="BRW-021">
          <SystemNote>Session läuft im Docker-Runner „build-02“ · Browser startet dort</SystemNote>
        </F>
      )}
      <AgentMessage harness="claude">
        <p>
          {remote
            ? 'Der Browser läuft im Container neben dem Dev-Server. Für eine Anmeldung kannst du das Panel normal bedienen; ein eigenes Fenster gibt es hier nicht.'
            : 'Gern. Öffne den Browser im eigenen Fenster, melde dich an und schalte zurück – die Anmeldung bleibt im Profil dieser Session.'}
        </p>
      </AgentMessage>
    </BrowserSession>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Policies im Browser                                                                         */
/* ------------------------------------------------------------------------------------------ */

function BrowserPoliciesScreen({ state }: { state: string }) {
  const blocked = state === 'blocked'
  const waiting = !blocked
  const pausedLabel = state === 'submit' ? 'Formular senden' : state === 'download' ? 'Download speichern' : 'JavaScript ausführen'
  return (
    <BrowserSession
      status={waiting ? 'waiting' : 'running'}
      running={blocked}
      rail={
        <BrowserPanel
          url={blocked ? 'https://stackoverflow.com/questions/1599660' : 'http://localhost:5173/checkout'}
          local={!blocked}
          addressNote={
            blocked ? (
              <F id={['BRW-009', 'BRW-019']} as="span" badge="bottom-right">
                <span className="inline-flex items-center gap-1 text-deny">
                  <ShieldAlert className="size-3" /> Blockiert
                </span>
              </F>
            ) : undefined
          }
          banner={
            waiting ? (
              <F id="BRW-019" className="chamfer shrink-0 border-l-4 border-signal bg-signal-soft px-3 py-1.5 text-[12px]">
                <span className="font-semibold">Angehalten:</span> {pausedLabel} wartet auf deine Freigabe – bis dahin wird nichts gesendet.
              </F>
            ) : undefined
          }
        >
          {blocked ? (
            <F id={['BRW-018', 'BRW-019']} className="mx-auto flex h-full max-w-[640px] flex-col justify-center rounded-sm bg-popover p-8">
              <ShieldAlert className="size-6 text-deny" aria-hidden />
              <p className="type-wide mt-3 text-[18px] font-[700]">Blockiert durch Egress-Policy</p>
              <p className="mt-2 text-[13px]">
                Regel <code className="rounded-sm bg-muted px-1 font-mono text-[12px]">browser-nav-allowlist</code> (Projekt shop-frontend): Der
                Agent darf nur docs.rs und developer.mozilla.org öffnen.
              </p>
              <p className="mt-1 text-[12.5px] text-muted-foreground">
                Die Seite wurde nicht geladen. Der Agent hat die Antwort <code className="font-mono">navigation_blocked</code> bekommen.
              </p>
              <div className="mt-4 flex gap-2">
                <button className="rounded-md border border-border px-3 py-1.5 text-[12.5px]">Regel ansehen</button>
                <button className="rounded-md border border-border px-3 py-1.5 text-[12.5px]">Zurück</button>
              </div>
            </F>
          ) : (
            <CheckoutPage marks={state === 'submit' ? { submit: 1 } : undefined} />
          )}
        </BrowserPanel>
      }
    >
      <UserMessage>Teste die Kasse einmal komplett durch, auch das Bezahlen in der Sandbox.</UserMessage>
      {blocked && (
        <>
          <F id="BRW-019">
            <ToolCall
              kind="other"
              name="browser_navigate"
              target="stackoverflow.com/questions/1599660 → navigation_blocked"
              status="denied"
              policy="browser-nav-allowlist"
              sandbox={false}
            />
          </F>
          <AgentMessage harness="claude" streaming>
            <p>Die Seite ist durch die Navigations-Policy gesperrt. Ich nehme stattdessen die MDN-Seite zu aria-describedby</p>
          </AgentMessage>
        </>
      )}
      {state === 'submit' && (
        <>
          <ToolCall kind="other" name="browser_click" target="ref e47 · „Jetzt kaufen“ → Formular-Submit erkannt" status="waiting" policy="browser_action" sandbox={false} />
          <F id="BRW-019">
            <ApprovalCard
              tool="Browser · Formular senden"
              command={'POST https://pay.sandbox.acme-payments.example/checkout\nFormular #checkout-form · ausgelöst vom Agent'}
              rule="browser-no-external-submit (Projekt shop-frontend)"
              reason="Der Agent will ein Formular an eine Seite außerhalb deines Rechners senden. Der Request ist angehalten; es wurde noch nichts übertragen."
            />
          </F>
        </>
      )}
      {state === 'download' && (
        <>
          <ToolCall kind="other" name="browser_click" target="„Rechnung herunterladen“ → Download erkannt" status="waiting" policy="browser_action" sandbox={false} />
          <F id="BRW-019">
            <ApprovalCard
              tool="Browser · Download"
              command="rechnung-2026-10.pdf · 182 KB · von localhost:5173/orders/1042/invoice"
              rule="downloads-fragen (Standard)"
              reason="Nach der Freigabe landet die Datei in .beton/downloads/ im Workspace. Lehnst du ab, wird sie gelöscht."
            />
          </F>
        </>
      )}
      {state === 'eval' && (
        <>
          <ToolCall kind="other" name="browser_eval" target="6 Zeilen JavaScript im Seitenkontext" status="waiting" sandbox={false} />
          <F id={['BRW-012', 'BRW-019']}>
            <ApprovalCard
              tool="Browser · JavaScript ausführen"
              command={`[...document.querySelectorAll('[data-testid]')]
  .filter((el) => el.tabIndex < 0)
  .map((el) => ({
    id: el.dataset.testid,
    tag: el.tagName.toLowerCase(),
  }))`}
              rule="browser-eval-fragen (Standard)"
              reason="browser_eval ist für diesen Agent eingeschaltet. Jeder Aufruf braucht deine Freigabe, weil Skripte Cookies und Tokens der Seite lesen können."
            />
          </F>
        </>
      )}
    </BrowserSession>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Einstellungen: Browser                                                                      */
/* ------------------------------------------------------------------------------------------ */

function Section({ title, hint, children, id }: { title: string; hint?: string; children: ReactNode; id: string | string[] }) {
  return (
    <F id={id} as="section" className="border-t border-border py-5">
      <h3 className="type-wide text-[14px] font-[650]">{title}</h3>
      {hint && <p className="mt-0.5 max-w-2xl text-[12.5px] text-muted-foreground">{hint}</p>}
      <div className="mt-3">{children}</div>
    </F>
  )
}

type Found = { name: string; version: string; path: string; status: 'used' | 'ok' | 'old' }

function BrowserSetupScreen({ state }: { state: string }) {
  const found: Found[] =
    state === 'found'
      ? [
          { name: 'Google Chrome', version: '141.0.7390.66', path: '/Applications/Google Chrome.app', status: 'used' },
          { name: 'Brave', version: '1.83 (Chromium 141)', path: '/Applications/Brave Browser.app', status: 'ok' },
          { name: 'Microsoft Edge', version: '118.0.2088', path: '/Applications/Microsoft Edge.app', status: 'old' },
        ]
      : [{ name: 'Microsoft Edge', version: '118.0.2088', path: '/Applications/Microsoft Edge.app', status: 'old' }]
  return (
    <SettingsFrame active="browser">
      <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-6">
        <span className="text-[13px] text-muted-foreground">Einstellungen</span>
        <span className="text-muted-foreground">/</span>
        <h2 className="text-[15px] font-semibold">Browser</h2>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-8">
        <div className="max-w-3xl">
          <p className="py-4 text-[13px] text-muted-foreground">
            Jede Session kann einen eigenen Chromium starten, den du im Panel siehst und der Agent bedienen kann. Er läuft auf diesem Rechner;
            sein Verkehr geht über den Egress-Proxy der Session.
          </p>

          <Section id="BRW-001" title="Welcher Browser verwendet wird" hint="beton nimmt den ersten kompatiblen Chromium-Browser (mindestens Version 120). Ein fest eingestellter Pfad hat Vorrang.">
            <div className="mb-3 flex items-center gap-3 text-[12.5px]">
              <code className="font-mono text-[12px]">browser.executable</code>
              <span className="text-muted-foreground">nicht gesetzt – beton sucht selbst</span>
              <button className="ml-auto rounded-md border border-border px-2.5 py-1 text-[12px] hover:bg-accent">Pfad festlegen…</button>
            </div>
            <table className="w-full text-[12.5px]">
              <thead>
                <tr className="border-b border-border text-left text-[11.5px] text-muted-foreground">
                  <th className="py-1 font-normal">Browser</th>
                  <th className="py-1 font-normal">Version</th>
                  <th className="py-1 font-normal">Pfad</th>
                  <th className="py-1 font-normal">Status</th>
                </tr>
              </thead>
              <tbody>
                {found.map((b) => (
                  <tr key={b.name} className="border-b border-border/60">
                    <td className="py-1.5 font-medium">{b.name}</td>
                    <td className="py-1.5 font-mono text-[11.5px]">{b.version}</td>
                    <td className="py-1.5 font-mono text-[11.5px] text-muted-foreground">{b.path}</td>
                    <td className="py-1.5">
                      {b.status === 'used' && (
                        <span className="inline-flex items-center gap-1 text-ok">
                          <Check className="size-3.5" /> wird verwendet
                        </span>
                      )}
                      {b.status === 'ok' && <span className="text-muted-foreground">kompatibel</span>}
                      {b.status === 'old' && (
                        <span className="inline-flex items-center gap-1 text-deny">
                          <X className="size-3.5" /> zu alt
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="mt-2 text-[11.5px] text-muted-foreground">
              Dasselbe Ergebnis zeigt <code className="font-mono">beton doctor</code> unter <code className="font-mono">browser.chromium</code>.
            </p>
          </Section>

          {state !== 'found' && (
            <Section id="BRW-002" title="Chrome for Testing" hint="Nur nötig, wenn kein passender Browser installiert ist. Der Download passiert ausschließlich auf deinen Klick.">
              {state === 'missing' && (
                <div className="chamfer border-l-4 border-signal bg-signal-soft p-3 text-[12.5px]">
                  <p className="font-semibold">Kein kompatibler Browser auf diesem Rechner</p>
                  <p className="mt-0.5">Gib einen installierten Chrome oder Chromium an – oder lade Chrome for Testing in der gepinnten Version.</p>
                  <div className="mt-2.5 flex flex-wrap items-center gap-2">
                    <button className="chamfer-sm inline-flex items-center gap-1 bg-foreground px-3 py-1.5 font-semibold text-background">
                      <Download className="size-3.5" /> Chrome for Testing laden (168 MB)
                    </button>
                    <button className="inline-flex items-center gap-1 rounded-md border border-foreground/30 bg-card px-3 py-1.5">
                      <FolderOpen className="size-3.5" /> Installierten Browser angeben…
                    </button>
                  </div>
                  <p className="mt-2 text-[11.5px] text-muted-foreground">
                    Version 141.0.7390.54 für macOS arm64 · Quelle storage.googleapis.com (Google) · Ablage ~/.beton/browser/cft/141.0.7390.54/ ·
                    Prüfung per SHA-256 aus dem offiziellen Manifest
                  </p>
                </div>
              )}
              {state === 'downloading' && (
                <div className="max-w-lg text-[12.5px]">
                  <div className="flex items-baseline justify-between">
                    <span className="font-medium">Chrome for Testing 141.0.7390.54 wird geladen</span>
                    <span className="text-muted-foreground tabular-nums">108 von 168 MB</span>
                  </div>
                  <Progress value={64} className="mt-2 h-1.5" />
                  <div className="mt-2 flex items-center gap-3 text-[11.5px] text-muted-foreground">
                    <span>von storage.googleapis.com · danach Prüfung per SHA-256</span>
                    <button className="ml-auto rounded-md border border-border px-2 py-0.5 text-foreground hover:bg-accent">Abbrechen</button>
                  </div>
                </div>
              )}
              {state === 'checksum' && (
                <div className="rounded-md border border-deny/40 bg-deny-soft p-3 text-[12.5px]">
                  <p className="font-semibold">Download verworfen: Prüfsumme stimmt nicht</p>
                  <p className="mt-0.5">
                    Die geladene Datei passt nicht zum offiziellen Manifest. beton hat sie gelöscht und nicht ausgeführt. Häufige Ursache: ein
                    abgebrochener Download oder ein Proxy, der Inhalte verändert.
                  </p>
                  <TerminalOutput>{`erwartet  sha256:4be1…9c02
erhalten  sha256:07aa…e3f1`}</TerminalOutput>
                  <div className="mt-2 flex gap-2">
                    <button className="rounded-md bg-foreground px-3 py-1.5 font-semibold text-background">Erneut laden</button>
                    <button className="rounded-md border border-border bg-card px-3 py-1.5">Installierten Browser angeben…</button>
                  </div>
                </div>
              )}
            </Section>
          )}

          <Section id={['BRW-003', 'BRW-002']} title="Start und Leerlauf">
            <div className="space-y-3 text-[12.5px]">
              <Row label="Chromium startet erst, wenn du das Panel öffnest oder der Agent den Browser braucht." />
              <Row label="Beenden nach Leerlauf" hint="ohne Zuschauer und ohne Agent-Aufruf">
                <span className="rounded-md border border-input bg-card px-2 py-1 font-mono text-[12px]">15 Min.</span>
              </Row>
              <Row label="Chrome for Testing im Skript- und Async-Modus automatisch laden" hint="browser.auto_download · verbindet sich ohne Rückfrage mit storage.googleapis.com">
                <Switch aria-label="Automatisch laden" />
              </Row>
            </div>
          </Section>

          <Section id="BRW-004" title="Profile" hint="Jede Session bekommt ein eigenes Browser-Profil. Dein Alltags-Browser wird nie verwendet; Cookies einer Session sieht keine andere.">
            <div className="flex items-center gap-3 text-[12.5px]">
              <code className="font-mono text-[12px]">~/.beton/browser/profiles/</code>
              <span className="text-muted-foreground">23 Profile · 412 MB · werden mit der Session gelöscht</span>
            </div>
          </Section>

          <Section id={['BRW-010', 'BRW-012']} title="Agent-Tools" hint="Diese Werkzeuge stehen jedem Harness zur Verfügung – Claude Code, Codex, ACP-Agents und Direkt-API.">
            <div className="flex flex-wrap gap-1">
              {['navigate', 'snapshot', 'click', 'type', 'select', 'scroll', 'press', 'screenshot', 'wait_for', 'tabs', 'console', 'request_pick'].map((t) => (
                <code key={t} className="rounded-sm border border-border px-1.5 py-0.5 font-mono text-[11.5px]">
                  browser_{t}
                </code>
              ))}
            </div>
            <div className="mt-4">
              <Row label="JavaScript im Seitenkontext erlauben (browser_eval)" hint="Aus. Wenn an, fragt trotzdem jeder Aufruf nach – Skripte können Cookies und Tokens lesen.">
                <Switch aria-label="browser_eval erlauben" />
              </Row>
            </div>
          </Section>
        </div>
      </div>
    </SettingsFrame>
  )
}

function Row({ label, hint, children }: { label: string; hint?: string; children?: ReactNode }) {
  return (
    <div className="flex items-center gap-4">
      <div className="min-w-0 flex-1">
        <div>{label}</div>
        {hint && <div className="text-[11.5px] text-muted-foreground">{hint}</div>}
      </div>
      {children}
    </div>
  )
}

export const group: ScreenGroup = {
  id: 'browser',
  title: 'Browser',
  order: 50,
  screens: [
    {
      id: 'browser-panel',
      title: 'Browser im Workspace',
      description:
        'Der Session-Browser im Workspace-Rail: Chromium läuft headless auf deinem Rechner, das Bild kommt als Screencast. Der Agent steuert ihn über browser_*-Tools; du siehst jede Aktion im Verlauf und kannst jederzeit selbst klicken.',
      features: [
        'BRW-001', 'BRW-002', 'BRW-003', 'BRW-004', 'BRW-005', 'BRW-006', 'BRW-007', 'BRW-009', 'BRW-010', 'BRW-011',
        'BRW-017', 'BRW-018', 'BRW-020',
      ],
      states: [
        { id: 'agent', title: 'Agent steuert' },
        { id: 'devserver', title: 'Dev-Server erkannt, nicht gestartet' },
        { id: 'no-browser', title: 'Kein Browser gefunden' },
        { id: 'crashed', title: 'Browser abgestürzt' },
        { id: 'slow', title: 'Langsame Verbindung' },
        { id: 'viewer', title: 'Als Zuschauer (ab M4)' },
      ],
      component: BrowserPanelScreen,
    },
    {
      id: 'browser-inspect',
      title: 'Inspect-Mode',
      description:
        'Element zeigen statt beschreiben: Im Picker hebt Chromium das Element unter dem Cursor mit Box-Model hervor. Nach dem Klick schreibst du einen Kommentar; Selektor, HTML, Styles, Bildausschnitt und – wenn möglich – die Quelldatei gehen an den Agent.',
      features: ['BRW-013', 'BRW-014', 'BRW-015', 'BRW-016', 'BRW-005', 'BRW-006', 'BRW-009'],
      states: [
        { id: 'hover', title: 'Picker aktiv' },
        { id: 'selected', title: 'Element gewählt' },
        { id: 'multi', title: 'Mehrfachauswahl' },
        { id: 'composer', title: 'Im Composer' },
        { id: 'sent', title: 'An den Agent gesendet' },
        { id: 'agent-request', title: 'Agent bittet um Auswahl' },
      ],
      component: BrowserInspect,
    },
    {
      id: 'browser-window',
      title: 'Sichtbares Fenster',
      description:
        'Für Logins, OAuth und echte DevTools wechselt der Browser in ein eigenes Fenster – mit demselben Profil. In Remote- und Container-Runnern gibt es nur das eingebettete Bild.',
      features: ['BRW-008', 'BRW-004', 'BRW-021'],
      states: [
        { id: 'confirm', title: 'Umschalten bestätigen' },
        { id: 'window', title: 'Im Fenster' },
        { id: 'remote', title: 'Container-Runner (M5)' },
      ],
      component: BrowserWindowScreen,
    },
    {
      id: 'browser-policies',
      title: 'Browser-Policies',
      description:
        'Navigation, Formular-Submits, Downloads und browser_eval sind Policy-Punkte. Gesperrte Ziele zeigen eine beton-Fehlerseite, riskante Aktionen halten an und warten auf deine Freigabe.',
      features: ['BRW-012', 'BRW-018', 'BRW-019', 'BRW-009'],
      states: [
        { id: 'blocked', title: 'Navigation blockiert' },
        { id: 'submit', title: 'Formular-Submit fragt' },
        { id: 'download', title: 'Download fragt' },
        { id: 'eval', title: 'browser_eval fragt' },
      ],
      component: BrowserPoliciesScreen,
    },
    {
      id: 'browser-setup',
      title: 'Browser einrichten',
      description:
        'Einstellungsseite: welcher installierte Browser verwendet wird, Chrome for Testing nur auf Klick, Leerlauf, Profile und Agent-Tools.',
      features: ['BRW-001', 'BRW-002', 'BRW-003', 'BRW-004', 'BRW-010', 'BRW-012'],
      states: [
        { id: 'found', title: 'Browser gefunden' },
        { id: 'missing', title: 'Kein Browser' },
        { id: 'downloading', title: 'Download läuft' },
        { id: 'checksum', title: 'Prüfsumme falsch' },
      ],
      component: BrowserSetupScreen,
    },
  ],
}
