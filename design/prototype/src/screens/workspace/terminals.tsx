import { CircleStop, Lock, LockOpen, Pencil, Plus, RotateCw, SquareTerminal, X } from 'lucide-react'
import { WorkspaceRail } from '@/app/session-chrome'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Avatar, Callout, SessionShell, ShortStream } from '@/app/kit/workspace'
import { TermView } from './parts'

type Tab = { id: string; name: string; by: string; running?: boolean; ended?: boolean }

const TABS: Tab[] = [
  { id: 'zsh', name: 'zsh', by: 'von dir' },
  { id: 'dev', name: 'Dev-Server', by: 'vom Agent gestartet', running: true },
  { id: 'tui', name: 'Claude Code (TUI)', by: 'Session im PTY-Modus' },
]

function TermTabs({ active, tabs = TABS }: { active: string; tabs?: Tab[] }) {
  return (
    <div role="tablist" className="flex h-9 shrink-0 items-end gap-0.5 border-b border-border bg-sidebar px-1">
      {tabs.map((t) => (
        <span
          key={t.id}
          role="tab"
          aria-selected={t.id === active}
          title={t.by}
          className={cn(
            '-mb-px flex h-8 items-center gap-1.5 rounded-t-md border border-b-0 px-2.5 text-[12px]',
            t.id === active ? 'border-border bg-card font-medium' : 'border-transparent text-muted-foreground',
            t.ended && 'italic',
          )}
        >
          {t.ended && <CircleStop className="size-3 text-muted-foreground" aria-label="beendet" />}
          {t.running && <span className="size-1.5 animate-pulse rounded-full bg-ok" aria-label="läuft" />}
          {t.name}
          {t.id === active && !t.ended && <Pencil className="size-3 text-muted-foreground" aria-label="Umbenennen" />}
          <X className="size-3 text-muted-foreground" aria-label={t.ended ? `${t.name} ausblenden` : `${t.name} schließen`} />
        </span>
      ))}
      <button aria-label="Neues Terminal" className="mb-1 ml-1 flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent">
        <Plus className="size-3.5" />
      </button>
    </div>
  )
}

function ShellTerm() {
  return (
    <TermView>
      {`~/.beton/worktrees/shop-frontend-3fa2c1d9/rate-limiter-7f3k  beton/rate-limiter-7f3k
❯ pnpm vitest run middleware
 ✓ src/middleware/rate-limit.spec.ts (4 tests) 38ms
 Test Files  1 passed (1)

❯ env | grep -i token
GITHUB_TOKEN=bt_cred_gh_7Hq2wQ
NPM_TOKEN=bt_cred_npm_K81zaP

❯ cat ~/.ssh/id_rsa
cat: /Users/ingo/.ssh/id_rsa: Operation not permitted
`}
      <span className="text-deny">{'beton: von der Sandbox blockiert (Profil „Tools“, Regel ssh-keys-lesen)\n'}</span>
      {'\n❯ '}
    </TermView>
  )
}

function DevTerm({ reattached }: { reattached?: boolean }) {
  return (
    <TermView>
      {reattached && <span className="text-muted-foreground">{'… 1.240 Zeilen Verlauf wiederhergestellt …\n'}</span>}
      {`❯ pnpm dev

  VITE v8.3.0  ready in 412 ms

  ➜  Local:   `}
      <span className="underline">http://localhost:5173/</span>
      {`
  ➜  Network: nicht freigegeben (--host fehlt)

14:02:11 [api] POST /auth/login 200 41ms
14:02:12 [api] POST /auth/login 200 38ms
14:02:12 [api] POST /auth/login 429 2ms  Retry-After: 12
14:06:40 [vite] hmr update /src/pages/Login.tsx
`}
    </TermView>
  )
}

function TuiTerm() {
  return (
    <TermView cursor={false}>
      {`╭───────────────────────────────────────────────────────────╮
│ ✻ Claude Code                                              │
│   /help für Hilfe · cwd: ~/code/shop-frontend               │
╰───────────────────────────────────────────────────────────╯

> Erkläre mir die Retry-Logik im Checkout

● Read(src/checkout/retry.ts)
  ⎿  Read 84 lines

● Die Retry-Logik versucht fehlgeschlagene Zahlungen bis zu drei Mal
  mit exponentiellem Backoff (500 ms, 1 s, 2 s) …

╭───────────────────────────────────────────────────────────╮
│ > █                                                        │
╰───────────────────────────────────────────────────────────╯
  ? für Tastenkürzel                          Claude Max · opus`}
    </TermView>
  )
}

/** Ein beendetes Terminal: Tab bleibt (kursiv) mit dem letzten Bildschirm aus `terminal.snapshot`. */
const CLOSED_TABS: Tab[] = [{ id: 'zsh', name: 'zsh', by: 'von dir' }, { id: 'test', name: 'Tests', by: 'von dir', ended: true }]

function ClosedTerm() {
  return (
    <TermView cursor={false} className="opacity-70">
      {`❯ pnpm vitest run
 ✓ src/middleware/rate-limit.spec.ts (4 tests) 38ms
 ✓ src/pages/Login.spec.tsx (7 tests) 112ms
 Test Files  2 passed (2)
      Tests  11 passed (11)
❯ exit
`}
    </TermView>
  )
}

function EmptyTerms() {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
      <SquareTerminal className="size-6 text-muted-foreground" />
      <div className="text-[13px] font-medium">Noch kein Terminal in dieser Session</div>
      <p className="max-w-[46ch] text-[12px] text-muted-foreground">
        Eine Shell im Arbeitsverzeichnis der Session, in derselben Sandbox wie die Tools des Agents. Sie läuft weiter, wenn du das Fenster schließt, und endet mit dem Runner.
      </p>
      <button className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-card px-3 text-[12px] font-medium hover:bg-accent">
        <Plus className="size-3.5" /> Neues Terminal
      </button>
    </div>
  )
}

function NoSandboxTerm() {
  return (
    <TermView>
      {`~/code/shop-frontend  main
❯ cat ~/.ssh/id_rsa | head -1
-----BEGIN OPENSSH PRIVATE KEY-----
❯ `}
    </TermView>
  )
}

export function TerminalsScreen({ state }: { state: string }) {
  const active = state === 'dev' || state === 'reattach' ? 'dev' : state === 'tui' ? 'tui' : state === 'closed' ? 'test' : 'zsh'
  const readonly = state === 'readonly'
  const tabs = state === 'empty' || state === 'error' ? [] : state === 'nosandbox' ? [TABS[0]!] : state === 'closed' ? CLOSED_TABS : TABS
  return (
    <SessionShell
      sessionList={false}
      rail={
        <WorkspaceRail active="terminal" width="w-[680px]">
          <F id={['SES-019', 'WEB-010']} className="flex h-full min-h-0 flex-col">
            <TermTabs active={active} tabs={tabs} />
            {state === 'error' && (
              <div className="p-2">
                <F id="SBX-017">
                  <Callout tone="deny">
                    <div className="font-medium">Terminal nicht gestartet</div>
                    <div className="text-[12px]">
                      Die Sandbox ist nicht verfügbar (<span className="font-mono">sandbox_unavailable</span>: Profil „Tools“ ließ sich nicht anwenden). Ohne Sandbox startet beton keine Shell. Prüfe die Sandbox unter{' '}
                      <span className="underline">Diagnose → Sandbox</span> und versuche es erneut.
                    </div>
                  </Callout>
                </F>
              </div>
            )}
            {(state === 'empty' || state === 'error') && <EmptyTerms />}
            {tabs.length > 0 && (
              <div className="flex h-8 shrink-0 items-center gap-3 border-b border-border px-3 text-[11px] text-muted-foreground">
                {active === 'zsh' && state !== 'nosandbox' && (
                  <>
                    <span className="inline-flex items-center gap-1">
                      <Lock className="size-3" /> Sandbox: Profil „Tools“
                    </span>
                    <span>120 × 32</span>
                  </>
                )}
                {state === 'nosandbox' && (
                  <F id="SBX-006" className="inline-flex items-center gap-3">
                    <span className="inline-flex items-center gap-1 font-medium text-deny">
                      <LockOpen className="size-3" /> Ohne Sandbox (backend: none)
                    </span>
                    <span>Die Shell sieht alles, was dein Benutzer sieht.</span>
                    <span>120 × 32</span>
                  </F>
                )}
                {active === 'test' && (
                  <span>
                    Beendet um 14:21 (Exit 0) · letzter Bildschirm, keine Eingabe mehr möglich
                  </span>
                )}
                {active === 'dev' && (
                  <>
                    <span>Vom Agent gestartet · läuft seit 14 Min.</span>
                    <span>Läuft weiter, auch wenn alle Fenster geschlossen sind</span>
                  </>
                )}
                {active === 'tui' && <span>Claude Code läuft im Terminal-Modus; beton spiegelt die Oberfläche 1:1.</span>}
                {state !== 'nosandbox' && state !== 'closed' && (
                  <span className="ml-auto inline-flex items-center gap-1.5">
                    Sieht gerade: <Avatar name="Ingo Fahrentholz" size="sm" ring /> <Avatar name="Anna Becker" size="sm" ring />
                  </span>
                )}
              </div>
            )}
            {state === 'reattach' && (
              <div className="flex items-center gap-2 border-b border-border bg-ok-soft px-3 py-1.5 text-[12px]">
                <RotateCw className="size-3.5 text-ok" />
                Wieder verbunden. Der Dev-Server lief weiter, während du weg warst; der Verlauf ist vollständig.
              </div>
            )}
            {active === 'zsh' && tabs.length > 0 && (state === 'nosandbox' ? <NoSandboxTerm /> : <ShellTerm />)}
            {active === 'dev' && <DevTerm reattached={state === 'reattach'} />}
            {active === 'tui' && <TuiTerm />}
            {active === 'test' && (
              <F id="DATA-011" className="flex min-h-0 flex-1 flex-col">
                <ClosedTerm />
              </F>
            )}
            {readonly && (
              <div className="border-t border-border p-2">
                <Callout>
                  Du siehst dieses Terminal als <span className="font-medium">Kommentieren &amp; Freigeben</span>. Eingaben sind deaktiviert; dafür braucht es die Rolle{' '}
                  <span className="font-medium">Mitsteuern</span>.
                </Callout>
              </div>
            )}
          </F>
        </WorkspaceRail>
      }
    >
      <ShortStream compact />
    </SessionShell>
  )
}
