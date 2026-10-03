import { FileText, SquareTerminal } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { Composer, SessionHeader } from '@/app/session-chrome'
import { AgentMessage, SystemNote, TerminalOutput, ToolCall, TurnFooter, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import type { HarnessId, SessionStatus } from '@/mock/data'
import { Btn, Code } from './kit'

function Earlier() {
  return (
    <>
      <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.</UserMessage>
      <ToolCall kind="edit" name="Bearbeiten" target="src/routes/auth.ts" duration="0,3 s" />
      <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth" duration="4,8 s" />
      <AgentMessage harness="claude">
        <p>Die Login-Route ist jetzt auf 5 Versuche pro Minute und IP begrenzt. Alle 10 Tests laufen.</p>
      </AgentMessage>
      <TurnFooter duration="1 Min. 12 s" tokens="38.410" cost="Subscription" />
    </>
  )
}

export function NoticesScreen({ state }: { state: string }) {
  const harness: HarnessId = state === 'fork-preamble' ? 'codex' : state === 'cold-resume' ? 'gemini' : 'claude'
  const status: SessionStatus = state === 'auth-required' ? 'waiting' : 'idle'
  const title =
    state === 'fork-preamble'
      ? 'Rate-Limiter – Review mit Codex'
      : state === 'fork-rebuild'
        ? 'Rate-Limiter – Variante mit Redis'
        : state === 'cold-resume'
          ? 'Terraform-Plan erklären'
          : 'Rate-Limiter für die Login-API'

  return (
    <AppLayout activeSession={state === 'cold-resume' ? 'ses_6m4d' : 'ses_7f3k'}>
      <SessionHeader title={title} harness={harness} status={status} branch="beton/rate-limiter-7f3k" />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
          {state === 'auth-required' && (
            <>
              <Earlier />
              <UserMessage queued>Bitte jetzt auch die Register-Route absichern.</UserMessage>
              <F id={['HAR-015', 'HAR-001']}>
                <div className="chamfer ml-9 border-l-4 border-signal bg-signal-soft p-3 text-[13px]">
                  <p className="font-semibold">Claude Code braucht eine neue Anmeldung</p>
                  <p className="mt-1">
                    Die claude-CLI meldet, dass dein Login abgelaufen ist. Melde dich in der CLI neu an; beton schickt deine Nachricht danach
                    automatisch ab. beton selbst sieht dein Konto nicht.
                  </p>
                  <div className="mt-2.5 flex flex-wrap items-center gap-2">
                    <Btn variant="primary" className="chamfer-sm rounded-none">
                      <SquareTerminal className="size-3.5" /> Terminal mit claude auth login öffnen
                    </Btn>
                    <Btn variant="outline">Erneut prüfen</Btn>
                    <span className="text-[11px] text-muted-foreground">oder im eigenen Terminal ausführen</span>
                  </div>
                </div>
              </F>
            </>
          )}

          {state === 'runner-restart' && (
            <>
              <Earlier />
              <UserMessage>Ja, push und PR.</UserMessage>
              <ToolCall kind="git" name="Shell" target="git push -u origin beton/rate-limiter-7f3k" status="failed" policy="git_guard" />
              <F id={['HAR-020', 'HAR-001']}>
                <SystemNote tone="deny">Runner neu gestartet (Rechner-Neustart um 08:12) · laufender Turn abgebrochen, offene Freigabe verworfen</SystemNote>
              </F>
              <F id="HAR-020">
                <SystemNote tone="ok">Session mit dem vollständigen Verlauf von Claude Code fortgesetzt (warm, claude --resume)</SystemNote>
              </F>
              <div className="ml-9 text-[13px] text-muted-foreground">
                Der Push wurde nicht ausgeführt. Schreib „push“, damit der Agent es erneut versucht; die Freigabe wird dann neu angefragt.
              </div>
            </>
          )}

          {state === 'cold-resume' && (
            <>
              <UserMessage>Was ändert der Plan an der Datenbank?</UserMessage>
              <AgentMessage harness="gemini">
                <p>Der Plan ersetzt die Instanzklasse von db.t3.medium auf db.t4g.large. Das erzwingt einen Neustart.</p>
              </AgentMessage>
              <F id={['HAR-020', 'HAR-018']}>
                <SystemNote>Runner neu gestartet · Gemini CLI kann Sessions nicht nativ fortsetzen</SystemNote>
                <div className="mt-3 ml-9 flex items-start gap-2 rounded-md border border-border bg-card p-3 text-[13px]">
                  <FileText className="mt-0.5 size-4 text-muted-foreground" />
                  <div>
                    <p>
                      Neu gestartet mit Übergabe-Dokument <Code>.beton/handover/ses_6m4d.md</Code>: 9 Turns, davon 3 gekürzt, 18 % des
                      Kontextfensters.
                    </p>
                    <p className="mt-1 text-muted-foreground">Der Agent kennt Ziel, letzte Antworten und geänderte Dateien, aber nicht jedes Tool-Ergebnis im Wortlaut.</p>
                  </div>
                </div>
              </F>
            </>
          )}

          {state === 'fork-preamble' && (
            <>
              <F id="HAR-018">
                <SystemNote>Fork von „Rate-Limiter für die Login-API“ (Claude Code) ab Ereignis 412 · Übergabe an Codex per Präambel</SystemNote>
              </F>
              <F id="HAR-018">
                <UserMessage author="beton · Übergabe">
                  <div className="space-y-2 rounded-md border border-border bg-card p-3 text-[13px]">
                    <p>
                      <strong>Ziel:</strong> Login-Route mit Rate-Limit (5/min/IP), Tests grün.
                    </p>
                    <p>
                      <strong>Geändert:</strong> <Code>src/routes/auth.ts</Code>, <Code>src/middleware/rate-limit.ts</Code> (neu),{' '}
                      <Code>src/middleware/rate-limit.spec.ts</Code> (neu)
                    </p>
                    <p>
                      <strong>Branch:</strong> <Code>beton/rate-limiter-7f3k</Code> · <strong>Offen:</strong> Register-Route, Redis-Ausfall
                    </p>
                    <p className="text-muted-foreground">[… 4 ältere Turns gekürzt, letzte 3 Turns vollständig …]</p>
                    <p className="text-muted-foreground">
                      Vollständig in <Code>.beton/handover/ses_7f3k.md</Code> · 31 % des Kontextfensters
                    </p>
                  </div>
                </UserMessage>
              </F>
              <AgentMessage harness="codex">
                <p>Übergabe gelesen. Ich prüfe zuerst die Middleware auf Race-Conditions beim Bucket-Refill.</p>
              </AgentMessage>
            </>
          )}

          {state === 'fork-rebuild' && (
            <>
              <F id="HAR-019">
                <SystemNote>Fork von „Rate-Limiter für die Login-API“ ab Turn 3 von 5 · Verlauf nativ übernommen (Claude Code)</SystemNote>
                <p className="mt-2 ml-9 text-[12px] text-muted-foreground">
                  Claude Code kennt Turn 1–3 vollständig inklusive Tool-Ergebnissen. Turn 4–5 gibt es nur in der Ursprungs-Session.
                </p>
              </F>
              <UserMessage>Bau den Limiter bitte auf Redis um, damit er über mehrere Instanzen hinweg zählt.</UserMessage>
              <AgentMessage harness="claude" streaming>
                <p>Ich ersetze die In-Memory-Map durch einen Redis-Zähler mit INCR und EXPIRE pro Fenster.</p>
              </AgentMessage>
            </>
          )}

          {state === 'mcp-failed' && (
            <>
              <F id={['HAR-009', 'AGT-006']}>
                <SystemNote tone="deny">MCP-Server github konnte nicht starten · Session läuft ohne seine Tools weiter</SystemNote>
                <div className="mt-3 ml-9 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
                  <p>
                    <Code>github-mcp-server stdio</Code> wurde nicht gefunden. Tools wie <Code>create_pull_request</Code> fehlen dem Agent deshalb.
                  </p>
                  <TerminalOutput>{`spawn github-mcp-server ENOENT
PATH=/opt/homebrew/bin:/usr/bin:/bin`}</TerminalOutput>
                  <div className="mt-2 flex gap-2">
                    <Btn variant="outline" size="sm">
                      MCP-Server verwalten
                    </Btn>
                    <Btn variant="ghost" size="sm">
                      Erneut starten
                    </Btn>
                  </div>
                </div>
              </F>
              <Earlier />
            </>
          )}

          {state === 'unmapped' && (
            <>
              <Earlier />
              <F id={['HAR-004', 'HAR-001']}>
                <div className="ml-9 flex items-center gap-2 text-[12px] text-muted-foreground">
                  <span className="size-1.5 rounded-full border border-muted-foreground" />
                  1 Meldung der claude-CLI konnte beton nicht zuordnen. Sie ist unverändert gespeichert und beeinflusst die Session nicht.
                  <button className="underline hover:text-foreground">Rohdaten anzeigen</button>
                </div>
              </F>
            </>
          )}

          {state === 'compaction' && (
            <>
              <Earlier />
              <UserMessage>/compact</UserMessage>
              <F id="HAR-022">
                <SystemNote tone="ok">Kontext komprimiert: 164.200 → 38.500 Tokens (Claude Code, nativ)</SystemNote>
              </F>
            </>
          )}
        </div>
      </div>
      <Composer harness={harness} />
    </AppLayout>
  )
}
