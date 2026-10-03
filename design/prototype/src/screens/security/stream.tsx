import type { ReactNode } from 'react'
import { ShieldAlert } from 'lucide-react'
import { SessionHeader } from '@/app/session-chrome'
import { AgentMessage, SystemNote, TerminalOutput, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { Btn, C, Callout, CodeView, InlineDialog, PrimaryButton, Shell } from '../policies/kit'

/** Verstoß direkt unter der verursachenden Tool-Karte (SBX-014). */
function Violation({ kind, children, actions }: { kind: string; children: ReactNode; actions?: ReactNode }) {
  return (
    <div className="ml-[3.75rem] -mt-2 flex items-start gap-2 rounded-md border border-deny/40 bg-deny-soft px-2.5 py-1.5 text-[12px]">
      <ShieldAlert className="mt-0.5 size-3.5 shrink-0 text-deny" />
      <div className="min-w-0 flex-1">
        <span className="font-semibold">Sandbox hat blockiert · {kind}</span>
        <div className="text-foreground/90">{children}</div>
      </div>
      {actions && <div className="flex shrink-0 gap-1.5">{actions}</div>}
    </div>
  )
}

export function SandboxViolation({ state }: { state: string }) {
  const suggest = state === 'suggest'
  return (
    <Shell
      nav="sessions"
      activeSession="ses_6q2a"
      overlay={
        suggest ? (
          <InlineDialog
            title="Regel für dieses Projekt hinzufügen?"
            width="w-[560px]"
            footer={
              <>
                <Btn tone="quiet">Abbrechen</Btn>
                <PrimaryButton>In .beton/config.yaml schreiben</PrimaryButton>
              </>
            }
          >
            <p>
              Damit <C>pnpm install</C> Pakete aus der Yarn-Registry laden darf, ergänzt beton diese Zeile. Sie gilt für alle Sessions in
              shop-frontend und landet im Repo – prüfe sie vor dem Commit.
            </p>
            <CodeView
              file=".beton/config.yaml"
              start={9}
              code={`  egress_rules:
    - "GET,HEAD registry.npmjs.org/**"
    - "GET,HEAD registry.yarnpkg.com/**"
    - "* api.github.com/repos/ifahrentholz/**"`}
              marks={[{ line: 11, tone: 'changed' }]}
            />
            <p className="text-[12px] text-muted-foreground">Nur lesend (GET, HEAD). beton schreibt nie automatisch – erst nach deiner Bestätigung.</p>
          </InlineDialog>
        ) : undefined
      }
    >
      <SessionHeader title="Checkout-Formular barrierefrei machen" harness="claude" status="idle" branch="beton/checkout-a11y-6q2a" />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <F id={['SBX-014', 'PRX-009']} className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
          <UserMessage>Installiere @reach/dialog und ersetze damit unser selbstgebautes Modal.</UserMessage>
          {(state === 'net' || suggest) && (
            <>
              <ToolCall kind="shell" name="Shell" target="pnpm add @reach/dialog" status="failed" duration="2,3 s" />
              <Violation
                kind="Netz"
                actions={
                  <>
                    <Btn>Regel vorschlagen …</Btn>
                  </>
                }
              >
                <C>GET registry.yarnpkg.com/@reach%2fdialog</C> – keine passende Egress-Regel (Default: ablehnen). Stufe 2, Tool-Call tc_8h1q.
              </Violation>
              <AgentMessage harness="claude">
                <p>Die Installation scheitert, weil die Yarn-Registry nicht freigegeben ist. Soll ich es über registry.npmjs.org versuchen?</p>
              </AgentMessage>
            </>
          )}
          {state === 'fs' && (
            <>
              <ToolCall kind="shell" name="Shell" target="cat ~/.aws/credentials | head -3" status="failed" duration="0,1 s" defaultOpen>
                <TerminalOutput>{`cat: /Users/ingo/.aws/credentials: Operation not permitted`}</TerminalOutput>
              </ToolCall>
              <Violation kind="Dateisystem">
                <C>~/.aws/credentials</C> ist in jeder Stufe maskiert. Diese Maske lässt sich nicht als Regel freigeben – nutze für AWS ein Credential-Binding
                mit Platzhalter.
              </Violation>
            </>
          )}
          {state === 'private' && (
            <F id="PRX-007" className="flex flex-col gap-4">
              <ToolCall kind="shell" name="Shell" target="curl -s http://metadata.internal.test/latest/meta-data/" status="failed" duration="0,2 s" />
              <Violation kind="Netz, privates Ziel">
                <C>metadata.internal.test</C> löst auf <C>169.254.169.254</C> auf (Cloud-Metadaten). Private und lokale Ziele sind gesperrt, auch wenn eine
                Regel den Hostnamen erlaubt.
              </Violation>
            </F>
          )}
          {state === 'limit' && (
            <F id="SBX-013" className="flex flex-col gap-4">
              <ToolCall kind="shell" name="Shell" target="node scripts/stress-render.js --workers 4000" status="failed" duration="12,0 s" />
              <Violation kind="Prozessgrenze">
                Der Prozessbaum hat die Grenze von 1 024 Prozessen erreicht; weitere Starts wurden abgelehnt. Der Rechner blieb bedienbar, das Tool wurde
                beendet.
              </Violation>
              <SystemNote>3 weitere Verstöße zusammengefasst (höchstens 10 pro Sekunde)</SystemNote>
            </F>
          )}
        </F>
      </div>
    </Shell>
  )
}

export function SandboxUnavailable({ state }: { state: string }) {
  const confirm = state === 'confirm-none'
  return (
    <Shell
      nav="sessions"
      activeSession="ses_6h2f"
      overlay={
        confirm ? (
          <InlineDialog
            title="Diese Session ohne Sandbox starten?"
            footer={
              <>
                <PrimaryButton>Mit Sandbox abbrechen</PrimaryButton>
                <Btn tone="deny">Ohne Sandbox starten</Btn>
              </>
            }
          >
            <p>
              Der Agent könnte dann alles lesen und ändern, was dein Benutzerkonto darf – auch <C>~/.ssh</C>, Cloud-Zugänge und andere Projekte – und
              beliebige Server erreichen. Policies gelten weiter, aber ohne zweite Verteidigungslinie.
            </p>
            <p className="text-muted-foreground">Gilt nur für diese Session. Wird im Audit-Log vermerkt. YOLO-Mode bleibt gesperrt.</p>
          </InlineDialog>
        ) : undefined
      }
    >
      <SessionHeader title="Lokales Modell testen" harness="ollama" status={state === 'proxy-down' ? 'stopped' : 'failed'} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <F id={['SBX-006', 'SBX-001']} className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
          <UserMessage>Lass die Benchmark-Suite gegen das neue Modell laufen.</UserMessage>
          {(state === 'unavailable' || confirm) && (
            <Callout
              tone="deny"
              title="Session nicht gestartet: Sandbox verlangt, aber auf diesem Rechner nicht verfügbar"
              actions={
                <>
                  <Btn>Fähigkeiten prüfen</Btn>
                  <Btn>Mit Podman starten</Btn>
                  <Btn tone="quiet">Ohne Sandbox starten …</Btn>
                </>
              }
            >
              <p>
                Debian 11 mit Kernel 5.10: Landlock fehlt, bubblewrap ist nicht installiert. Ohne Netz- und Dateiisolation startet beton keinen Agent – es
                gibt keinen stillen Rückfall.
              </p>
              <p className="text-muted-foreground">
                Fehlt: <C>net</C>, <C>fs_write</C>, <C>masks</C> · Tipp: Kernel ≥ 5.19 mit Landlock, <C>apt install bubblewrap</C> oder Backend Docker/Podman.
              </p>
            </Callout>
          )}
          {state === 'forbidden' && (
            <Callout tone="deny" title="Ohne Sandbox ist hier nicht erlaubt">
              <p>
                Die Org-Policy <span className="font-mono">acme-baseline/require-sandbox</span> verbietet <C>backend: none</C> – auch mit Bestätigung. Richte
                eine Sandbox ein oder nutze einen Remote-Runner.
              </p>
            </Callout>
          )}
          {state === 'yolo' && (
            <Callout tone="deny" title="YOLO-Mode braucht eine aktive Sandbox">
              <p>
                Im YOLO-Mode fragt die CLI nicht mehr nach. Das ist nur sicher, wenn Sandbox und Egress-Proxy laufen. Diese Session ist auf{' '}
                <C>backend: none</C> gestellt.
              </p>
            </Callout>
          )}
          {state === 'proxy-down' && (
            <>
              <ToolCall kind="shell" name="Shell" target="pnpm bench --model qwen3-coder:30b" status="failed" duration="0,4 s" />
              <SystemNote tone="deny">Egress-Proxy beendet · Session angehalten</SystemNote>
              <Callout
                tone="deny"
                title="Session angehalten: der Egress-Proxy läuft nicht mehr"
                actions={
                  <>
                    <PrimaryButton>Proxy neu starten und fortsetzen</PrimaryButton>
                    <Btn tone="quiet">Log ansehen</Btn>
                  </>
                }
              >
                <p>Ohne Proxy hat die Sandbox kein Netz – Tools können nicht direkt ins Internet ausweichen. Neue Tool-Aufrufe werden abgelehnt, bis der Proxy wieder läuft.</p>
              </Callout>
            </>
          )}
        </F>
      </div>
    </Shell>
  )
}
