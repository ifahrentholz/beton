import { AppLayout } from '@/app/app-layout'
import { Composer, SessionHeader, WorkspaceRail } from '@/app/session-chrome'
import {
  AgentMessage,
  ApprovalCard,
  Diff,
  Reasoning,
  SystemNote,
  TerminalOutput,
  ToolCall,
  TurnFooter,
  UserMessage,
} from '@/app/stream'
import { ApprovalBar, type BarItem, type BarNotice } from '@/app/kit/approvals'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { currentSession, rateLimiterDiff, type SessionStatus } from '@/mock/data'
import { editApproval, lintQuestion, prApproval, pushApproval } from './approval-data'

/**
 * Referenz-Gruppe: zeigt, wie Screens aufgebaut sind.
 * - Ein Screen = eine Komponente mit `state`-Prop; Zustände über `states`.
 * - Bereiche mit `<F id="…">` markieren, welches Feature sie zeigen.
 * - App-Bausteine aus `@/app/*`, Beispieldaten aus `@/mock/*`.
 */

function ChangesPanel() {
  return (
    <F id={['SES-018', 'WEB-011']} className="flex flex-col gap-3 p-3">
      <div className="flex items-center gap-2 text-xs">
        <span className="rounded-md bg-accent px-2 py-0.5 font-medium">Nicht committet</span>
        <span className="px-2 py-0.5 text-muted-foreground">Branch</span>
        <span className="px-2 py-0.5 text-muted-foreground">Dieser Turn</span>
        <span className="ml-auto text-muted-foreground">2 Dateien · +14 −1</span>
      </div>
      <Diff file="src/routes/auth.ts" lines={rateLimiterDiff} />
      <Diff
        file="src/middleware/rate-limit.ts (neu)"
        lines={[
          { kind: 'add', n: 1, text: "import type { RequestHandler } from 'express'" },
          { kind: 'add', n: 2, text: '' },
          { kind: 'add', n: 3, text: 'export function rateLimit(opts: Options): RequestHandler {' },
          { kind: 'add', n: 4, text: '  const buckets = new Map<string, Bucket>()' },
          { kind: 'add', n: 5, text: '  return (req, res, next) => { … }' },
          { kind: 'add', n: 6, text: '}' },
        ]}
      />
    </F>
  )
}

/** Was die Freigabe-Leiste (WEB-007) im jeweiligen Zustand zeigt. */
function barFor(state: string): { items: BarItem[]; expanded?: boolean; denying?: string; notice?: BarNotice } | undefined {
  switch (state) {
    case 'approval':
      return { items: [pushApproval] }
    case 'approval-stack':
      return { items: [pushApproval, editApproval, prApproval, lintQuestion] }
    case 'approval-args':
      return { items: [prApproval], expanded: true }
    case 'approval-diff':
      return { items: [editApproval, prApproval], expanded: true }
    case 'approval-deny':
      return { items: [pushApproval, editApproval], denying: 'Bitte erst die CI auf dem Branch abwarten' }
    case 'approval-decided':
      return {
        items: [editApproval, prApproval],
        notice: { kind: 'decided', summary: pushApproval.summary, decision: 'allow', by: 'Mara', where: 'am Handy, gerade eben' },
      }
    case 'approval-timeout':
      return {
        items: [],
        notice: { kind: 'expired', summary: 'git push -u origin beton/rate-limiter-7f3k', after: '30 Min.', applied: 'abbrechen (strengster Wert aus „ablehnen“ und „abbrechen“)' },
      }
    case 'question':
      return { items: [lintQuestion], }
  }
  return undefined
}

const BAR_STATES = ['approval', 'approval-stack', 'approval-args', 'approval-diff', 'approval-deny', 'approval-decided', 'approval-timeout', 'question']

function SessionStream({ state }: { state: string }) {
  const bar = barFor(state)
  const waiting = state === 'approval-m0' || (bar !== undefined && bar.items.length > 0)
  const status: SessionStatus =
    state === 'streaming' ? 'running' : waiting ? 'waiting' : state === 'failed' ? 'failed' : 'idle'

  if (state === 'empty') {
    return (
      <AppLayout activeSession="new" rail={undefined}>
        <SessionHeader title="Neue Session" harness="claude" status="idle" />
        <F id="SES-001" className="concrete-grain flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
          <p className="type-wide text-xl font-[700]">Woran soll der Agent arbeiten?</p>
          <p className="max-w-md text-sm text-muted-foreground">
            Beschreibe die Aufgabe. Die Session läuft mit Claude Code über deine Claude-Max-Anmeldung; Harness und
            Modell kannst du unten wechseln.
          </p>
        </F>
        <Composer harness="claude" />
      </AppLayout>
    )
  }

  return (
    <AppLayout
      activeSession={currentSession.id}
      rail={
        <WorkspaceRail active="changes">
          <ChangesPanel />
        </WorkspaceRail>
      }
    >
      <F id={['SES-001', 'HAR-004']} badge="bottom-left">
        <SessionHeader title={currentSession.title} harness="claude" status={status} branch={currentSession.branch} />
      </F>
      <F id={['WEB-002', 'SES-002', 'PROTO-005']} className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
          <UserMessage>
            Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.
          </UserMessage>
          <Reasoning summary="Middleware oder Gateway?">
            Ein Gateway-Limit gibt es im Projekt nicht. Eine Express-Middleware mit Token-Bucket pro IP reicht und ist
            testbar.
          </Reasoning>
          <ToolCall kind="search" name="Suche" target="rg 'auth.post' src/" duration="0,2 s" />
          <ToolCall kind="read" name="Lesen" target="src/routes/auth.ts" duration="0,1 s" />
          <ToolCall kind="edit" name="Bearbeiten" target="src/routes/auth.ts" duration="0,3 s">
            <Diff file="src/routes/auth.ts" lines={rateLimiterDiff} />
          </ToolCall>
          <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth" duration="4,8 s">
            <TerminalOutput>{`✓ auth/login.spec.ts (6)
✓ middleware/rate-limit.spec.ts (4)

Test Files  2 passed (2)
     Tests  10 passed (10)`}</TerminalOutput>
          </ToolCall>
          <AgentMessage harness="claude" streaming={state === 'streaming'}>
            <p>
              Die Login-Route ist jetzt auf 5 Versuche pro Minute und IP begrenzt. Danach antwortet sie mit{' '}
              <code className="rounded-sm bg-muted px-1 text-[13px]">429</code> und einem{' '}
              <code className="rounded-sm bg-muted px-1 text-[13px]">Retry-After</code>-Header.
            </p>
            {state !== 'streaming' && <p>Alle 10 Tests laufen. Soll ich den Branch pushen und einen PR öffnen?</p>}
          </AgentMessage>

          {state === 'done' && <TurnFooter duration="1 Min. 12 s" tokens="38.410" cost="Subscription" />}

          {state === 'approval-m0' && (
            <>
              <UserMessage>Ja, push und PR.</UserMessage>
              <ToolCall kind="git" name="Shell" target="git push -u origin beton/rate-limiter-7f3k" status="waiting" policy="git_guard" />
              <F id={['WEB-018', 'HAR-005']}>
                <ApprovalCard
                  variant="minimal"
                  tool="Shell"
                  command="git push -u origin beton/rate-limiter-7f3k"
                  rule="git-push-fragen (Projekt shop-frontend)"
                  reason="Pushes verlassen deinen Rechner. Die Projekt-Policy verlangt dafür deine Freigabe."
                />
              </F>
            </>
          )}

          {BAR_STATES.includes(state) && state !== 'question' && (
            <>
              <UserMessage>Ja, push und PR.</UserMessage>
              {state === 'approval-decided' ? (
                <>
                  <ToolCall kind="git" name="Shell" target="git push -u origin beton/rate-limiter-7f3k" status="running" policy="erlaubt von Mara" />
                  <F id={['WEB-007', 'WEB-018']}>
                    <ApprovalCard tool="Shell" command="git push -u origin beton/rate-limiter-7f3k" reason="" resolved="allowed" by="Mara (am Handy)" />
                  </F>
                </>
              ) : state === 'approval-timeout' ? (
                <>
                  <ToolCall kind="git" name="Shell" target="git push -u origin beton/rate-limiter-7f3k" status="denied" policy="Zeit abgelaufen" />
                  <SystemNote tone="deny">Turn abgebrochen · Freigabe abgelaufen um 15:42</SystemNote>
                </>
              ) : (
                ['approval', 'approval-stack', 'approval-deny'].includes(state) && (
                  <ToolCall kind="git" name="Shell" target="git push -u origin beton/rate-limiter-7f3k" status="waiting" policy="wartet auf Freigabe" />
                )
              )}
              {['approval-stack', 'approval-diff', 'approval-deny', 'approval-decided'].includes(state) && (
                <ToolCall kind="edit" name="Bearbeiten" target="src/routes/auth.ts" status="waiting" policy="wartet auf Freigabe" />
              )}
              {['approval-stack', 'approval-args', 'approval-diff', 'approval-decided'].includes(state) && (
                <ToolCall kind="other" name="MCP github" target="create_pull_request · beton/rate-limiter-7f3k → main" status="waiting" policy="wartet auf Freigabe" />
              )}
            </>
          )}

          {state === 'question' && (
            <F id="AGT-007">
              <ToolCall kind="other" name="Frage an dich" target="ask_user · Limiter pro IP oder pro Account?" status="waiting" policy="wartet auf Antwort" />
            </F>
          )}

          {state === 'failed' && (
            <>
              <F id={['HAR-001', 'RUN-003']}>
                <SystemNote tone="deny">Claude Code wurde unerwartet beendet (Exit-Code 137)</SystemNote>
              </F>
              <div className="ml-9 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
                <p className="font-semibold">Die Session ist angehalten.</p>
                <p className="mt-1 text-muted-foreground">
                  Der Prozess wurde vom Betriebssystem beendet, vermutlich wegen Speichermangel. Der Verlauf ist
                  gespeichert; du kannst die Session fortsetzen.
                </p>
                <TerminalOutput>{`stderr (letzte Zeilen):
FATAL ERROR: Reached heap limit Allocation failed - JavaScript heap out of memory`}</TerminalOutput>
                <div className="mt-2 flex gap-2">
                  <button className="rounded-md bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Fortsetzen</button>
                  <button className="rounded-md border border-border px-3 py-1.5 text-[13px]">Mit Codex fortsetzen</button>
                </div>
              </div>
            </>
          )}

          {state === 'interrupted' && (
            <F id="SES-005">
              <SystemNote>Von dir unterbrochen. Der laufende Tool-Call wurde abgebrochen.</SystemNote>
            </F>
          )}
        </div>
      </F>
      {bar && (
        <F id={state === 'question' ? ['WEB-007', 'AGT-007', 'UX-001'] : ['WEB-007', 'POL-010', 'POL-014']} badge="top-right">
          <ApprovalBar key={state} {...bar} />
        </F>
      )}
      <Composer
        harness="claude"
        running={state === 'streaming'}
        queued={state === 'streaming' ? ['Danach bitte auch die Register-Route absichern'] : undefined}
      />
    </AppLayout>
  )
}

export const group: ScreenGroup = {
  id: 'session',
  title: 'Session',
  order: 10,
  screens: [
    {
      id: 'session-stream',
      title: 'Session-Verlauf',
      description:
        'Die Hauptansicht: Verlauf mit Nachrichten, Überlegungen und Tool-Calls, rechts die Änderungen, unten die Eingabe. Offene Freigaben und Fragen stehen in der gelben Leiste mit Fase über der Eingabe (gestapelt, per Tastatur bedienbar: ⌘↵, ⌘⇧↵, ⌘⌫, ⌘⇧A); klickbar.',
      features: [
        'WEB-001', 'WEB-002', 'WEB-003', 'WEB-004', 'WEB-007', 'WEB-008', 'WEB-011', 'WEB-018',
        'SES-001', 'SES-002', 'SES-004', 'SES-005', 'SES-012', 'SES-018', 'WEB-005',
        'HAR-001', 'HAR-004', 'HAR-005', 'HAR-021', 'USE-004', 'USE-008', 'POL-010', 'POL-014', 'POL-017', 'AGT-007', 'UX-001',
        'RUN-003', 'PROTO-005', 'VOI-004', 'VOI-007', 'CLI-002',
      ],
      states: [
        { id: 'streaming', title: 'Agent arbeitet' },
        { id: 'approval', title: 'Freigabe offen (Leiste)' },
        { id: 'approval-stack', title: 'Mehrere Freigaben gestapelt' },
        { id: 'approval-args', title: 'Argumente ausgeklappt' },
        { id: 'approval-diff', title: 'Datei-Edit mit Diff' },
        { id: 'approval-deny', title: 'Ablehnen mit Grund' },
        { id: 'approval-decided', title: 'Entschieden von Mara' },
        { id: 'approval-timeout', title: 'Freigabe abgelaufen' },
        { id: 'question', title: 'Frage des Agents (ask_user)' },
        { id: 'approval-m0', title: 'Freigabe (M0, minimal)' },
        { id: 'done', title: 'Fertig' },
        { id: 'interrupted', title: 'Unterbrochen' },
        { id: 'failed', title: 'Harness abgestürzt' },
        { id: 'empty', title: 'Neue Session' },
      ],
      component: SessionStream,
    },
  ],
  noUi: {
    'WEB-015': {
      reason:
        'Performance-Budgets (60 fps beim Streaming, schneller Session-Wechsel, begrenzter Speicher) werden in der CI gemessen; spürbar beim Streaming im Session-Verlauf.',
      visibleIn: 'session-stream',
    },
  },
}
