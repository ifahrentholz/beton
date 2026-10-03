import { GitFork, MonitorSmartphone, Play, RotateCw, Sparkles, SquareTerminal } from 'lucide-react'
import { Composer } from '@/app/session-chrome'
import { AgentMessage, SystemNote, ToolCall, TurnFooter, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { ShortStream } from './bits'
import { Btn, Callout, SessionShell } from './parts'

function GeminiStream() {
  return (
    <>
      <UserMessage>Erklär mir den Terraform-Plan für das neue VPC-Peering. Was wird ersetzt statt geändert?</UserMessage>
      <ToolCall kind="shell" name="Shell" target="terraform plan -out=plan.bin" duration="18,2 s" />
      <AgentMessage harness="gemini">
        <p>
          Zwei Ressourcen werden ersetzt: die Route-Table <code className="rounded-sm bg-muted px-1 text-[13px]">rt-private-b</code> und die Security-Group für den Bastion-Host,
          weil sich ihre VPC-ID ändert.
        </p>
      </AgentMessage>
    </>
  )
}

export function ResumeScreen({ state }: { state: string }) {
  const gemini = state === 'handover' || state === 'compact-unsupported'
  const harness = gemini ? 'gemini' : 'claude'
  const status = state === 'stopped' ? 'stopped' : state === 'resumed' || state === 'handover' ? 'running' : 'idle'
  const title = gemini ? 'Terraform-Plan erklären' : state === 'title' ? 'Rate-Limiter für die Login-API' : undefined

  return (
    <SessionShell
      title={title}
      harness={harness}
      status={status}
      branch={gemini ? '' : undefined}
      activeSession={gemini ? 'ses_6m4d' : 'ses_7f3k'}
      headerExtra={
        state === 'reconnected' ? (
          <F id="SES-002" as="span" className="hidden items-center gap-1.5 text-[11px] text-muted-foreground lg:inline-flex" badge="bottom-left">
            <span title="Auch geöffnet in">Auch offen:</span>
            <SquareTerminal className="size-3.5" aria-label="Terminal (beton attach)" />
            <MonitorSmartphone className="size-3.5" aria-label="Browser auf dem Handy" />
          </F>
        ) : undefined
      }
      composer={
        state === 'stopped' ? (
          <Composer harness="claude" placeholder="Senden setzt die Session fort – @ für Dateien, / für Befehle" />
        ) : state === 'context-high' ? (
          <div>
            <F id="SES-011" className="px-4 pt-3">
              <Callout tone="neutral" className="flex items-center gap-3">
                <span className="flex-1">
                  Der Kontext ist zu 86 % gefüllt (172.000 von 200.000 Tokens). Komprimieren lässt Claude Code den bisherigen Verlauf zusammenfassen; der Verlauf in beton bleibt
                  vollständig.
                </span>
                <Btn size="sm">Jetzt komprimieren</Btn>
              </Callout>
            </F>
            <Composer harness="claude" />
          </div>
        ) : undefined
      }
    >
      {state === 'reconnected' && (
        <F id="SES-002">
          <Callout tone="ok" className="flex items-center gap-2">
            <RotateCw className="size-3.5 text-ok" />
            Verbindung kurz unterbrochen. 214 Ereignisse ab #1.802 nachgeladen, nichts fehlt und nichts doppelt.
          </Callout>
        </F>
      )}

      {gemini ? <GeminiStream /> : <ShortStream />}

      {state === 'title' && (
        <>
          <TurnFooter duration="1 Min. 12 s" tokens="38.410" cost="Subscription" />
          <F id="SES-010">
            <SystemNote>
              <span className="inline-flex items-center gap-1">
                <Sparkles className="size-3" /> Titel erzeugt: „Rate-Limiter für die Login-API“ · claude-haiku-4-5 über deine Claude-Max-Anmeldung ·{' '}
                <span className="text-foreground underline underline-offset-2">Umbenennen</span>
              </span>
            </SystemNote>
          </F>
        </>
      )}

      {state === 'stopped' && (
        <F id="SES-003">
          <div className="ml-9 border-l-2 border-border py-1 pl-3 text-[13px]">
            <p className="font-medium">Session gestoppt</p>
            <p className="mt-0.5 text-muted-foreground">
              Claude Code wurde nach 30 Minuten ohne Aktivität beendet. Der Verlauf ist gespeichert. Wenn du etwas sendest, startet beton Claude Code neu und setzt die
              Unterhaltung fort.
            </p>
            <Btn size="sm" className="mt-2">
              <Play className="size-3.5" /> Jetzt fortsetzen
            </Btn>
          </div>
        </F>
      )}

      {state === 'resumed' && (
        <>
          <F id="SES-003">
            <SystemNote>Fortgesetzt · Claude Code kennt den bisherigen Verlauf (gespeicherte Session-ID)</SystemNote>
          </F>
          <UserMessage>Danach bitte auch die Register-Route absichern.</UserMessage>
          <AgentMessage harness="claude" streaming>
            <p>Mache ich. Ich nutze dieselbe Middleware mit 3 Versuchen pro</p>
          </AgentMessage>
        </>
      )}

      {state === 'handover' && (
        <>
          <F id="SES-003">
            <SystemNote>Fortgesetzt per Übergabe · Gemini CLI kann nicht nativ fortsetzen, beton hat den Verlauf zusammengefasst (5.880 Tokens)</SystemNote>
          </F>
          <UserMessage>Und was passiert mit laufenden Verbindungen, wenn die Route-Table ersetzt wird?</UserMessage>
          <AgentMessage harness="gemini" streaming>
            <p>Bestehende Verbindungen über</p>
          </AgentMessage>
        </>
      )}

      {state === 'compacted' && (
        <F id="SES-011" className="flex flex-col gap-3">
          <SystemNote>Komprimierung gestartet · /compact an Claude Code gesendet</SystemNote>
          <SystemNote tone="ok">Kontext komprimiert: 172.000 → 38.500 Tokens · Verlauf in beton bleibt vollständig</SystemNote>
        </F>
      )}

      {state === 'compact-unsupported' && (
        <F id="SES-011" className="flex flex-col gap-3">
          <UserMessage>/compact</UserMessage>
          <div className="ml-9 border-l-2 border-deny bg-deny-soft py-1.5 pr-2 pl-3 text-[13px]">
            <p className="font-medium">Gemini CLI kann den Kontext nicht komprimieren.</p>
            <p className="mt-0.5">Forke die Session mit Übergabe, um mit einer Zusammenfassung und kleinem Kontext weiterzumachen.</p>
            <Btn size="sm" className="mt-2">
              <GitFork className="size-3.5" /> Mit Übergabe forken
            </Btn>
          </div>
        </F>
      )}
    </SessionShell>
  )
}
