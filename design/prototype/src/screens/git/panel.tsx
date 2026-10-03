import { ArrowUpRight, CornerDownRight, ExternalLink, GitPullRequest, Link2, Send, Upload } from 'lucide-react'
import { Composer, WorkspaceRail } from '@/app/session-chrome'
import { ApprovalCard, SystemNote, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { ShortStream } from '../workspace/bits'
import { Avatar, Btn, Callout, FakeInput, NetNote, PanelHeader, Segmented, SessionShell, Tag } from '../workspace/parts'
import { CheckRow, CrStateTag, OriginTag } from './parts'

/* ---------- PR-Panel ---------- */

function LogTail() {
  return (
    <pre className="dark mx-3 mb-2 overflow-x-auto rounded-md bg-sunken p-2 font-mono text-[11px] leading-relaxed text-foreground">{`  ✗ login › sperrt nach 5 Versuchen (playwright)
    Expected status: 429
    Received status: 200
      at tests/e2e/login.spec.ts:41:7
  1 failed, 23 passed (1m 48s)`}</pre>
  )
}

function SubTabs({ value, reviews = 3 }: { value: string; reviews?: number }) {
  return (
    <div className="flex items-center gap-2 border-b border-border px-3 py-2">
      <Segmented
        value={value}
        items={[
          { id: 'overview', label: 'Übersicht' },
          { id: 'diff', label: 'Diff' },
          { id: 'reviews', label: `Reviews · ${reviews}` },
        ]}
      />
    </div>
  )
}

function CrHead({ gitlab }: { gitlab?: boolean }) {
  return (
    <div className="border-b border-border px-3 py-2.5">
      <div className="flex items-center gap-2">
        <GitPullRequest className="size-4 shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate text-[14px] font-semibold">{gitlab ? 'Bundle-Preise serverseitig berechnen' : 'Rate-Limiter für die Login-API'}</span>
        <span className="shrink-0 font-mono text-[12px] text-muted-foreground">{gitlab ? '!118' : '#482'}</span>
        <ExternalLink className="ml-auto size-3.5 shrink-0 text-muted-foreground" aria-label="Beim Provider öffnen" />
      </div>
      <div className="mt-1.5 flex flex-wrap items-center gap-2">
        <CrStateTag state="open" />
        <OriginTag origin={gitlab ? 'attached' : 'created'} />
        <span className="font-mono text-[11px] text-muted-foreground">{gitlab ? 'feature/bundle-prices → develop' : 'beton/rate-limiter-7f3k → main'}</span>
      </div>
      <dl className="mt-2 grid grid-cols-[110px_1fr] gap-y-1 text-[12px]">
        <dt className="text-muted-foreground">Mergebar</dt>
        <dd>Keine Konflikte · blockiert durch 1 fehlgeschlagenen {gitlab ? 'Job' : 'Check'}</dd>
        <dt className="text-muted-foreground">Reviews</dt>
        <dd className="flex items-center gap-1.5">
          <Avatar name="Anna Becker" size="sm" /> Änderungen gewünscht
          <span className="text-muted-foreground">· Jonas Weber ausstehend</span>
        </dd>
      </dl>
    </div>
  )
}

function GithubChecks() {
  return (
    <div>
      <div className="px-3 pt-2 pb-1 text-[12px] font-medium">Checks · 6</div>
      <CheckRow
        name="e2e / login-flow"
        status="failure"
        detail="1 Min. 48 s"
        action={
          <Btn size="sm" variant="primary">
            <Send className="size-3" /> An Agent senden
          </Btn>
        }
      >
        <LogTail />
      </CheckRow>
      <CheckRow name="build" status="success" detail="1 Min. 12 s" />
      <CheckRow name="unit (node 22)" status="success" detail="48 s" />
      <CheckRow name="lint" status="running" detail="seit 20 s" />
      <CheckRow name="deploy-preview" status="queued" />
      <CheckRow name="codeql (legacy status)" status="skipped" />
    </div>
  )
}

function GitlabPipeline() {
  return (
    <div>
      <div className="px-3 pt-2 pb-1 text-[12px] font-medium">Pipeline #9921 · 3 Stufen</div>
      <div className="px-3 pb-1 text-[11px] text-muted-foreground">build</div>
      <CheckRow name="build:web" status="success" detail="2 Min. 4 s" />
      <div className="px-3 pt-2 pb-1 text-[11px] text-muted-foreground">test</div>
      <CheckRow
        name="test:unit"
        status="failure"
        detail="58 s"
        action={
          <Btn size="sm" variant="primary">
            <Send className="size-3" /> An Agent senden
          </Btn>
        }
      >
        <pre className="dark mx-3 mb-2 overflow-x-auto rounded-md bg-sunken p-2 font-mono text-[11px] leading-relaxed text-foreground">{`FAIL src/pricing/bundle.spec.ts
  ● bundle › rundet Rabatt kaufmännisch
    expect(received).toBe(expected)
    Expected: 19.99   Received: 19.98`}</pre>
      </CheckRow>
      <CheckRow name="test:e2e" status="cancelled" />
      <div className="px-3 pt-2 pb-1 text-[11px] text-muted-foreground">deploy</div>
      <CheckRow name="deploy:review-app" status="skipped" />
    </div>
  )
}

function OtherCrs({ attach }: { attach?: boolean }) {
  return (
    <div className="border-t border-border px-3 py-3">
      <div className="mb-2 text-[12px] font-medium">Weitere verknüpfte Requests</div>
      <div className="flex items-center gap-2 py-1 text-[13px]">
        <GitPullRequest className="size-3.5 text-muted-foreground" />
        <span className="min-w-0 truncate">Checkout: Validierung vereinheitlichen</span>
        <span className="font-mono text-[11px] text-muted-foreground">#479</span>
        <OriginTag origin="inferred" />
        <Btn size="sm" variant="ghost" className="ml-auto">
          Verwerfen
        </Btn>
      </div>
      <div className="mt-2 flex gap-2">
        <FakeInput value={attach ? 'https://github.com/acme-internal/payments/pull/77' : ''} placeholder="PR- oder MR-Adresse einfügen" mono className="h-7 flex-1" focus={attach} />
        <Btn size="sm">
          <Link2 className="size-3.5" /> Verknüpfen
        </Btn>
      </div>
      {attach && (
        <Callout tone="deny" className="mt-2">
          Auf <span className="font-mono text-[12px]">acme-internal/payments</span> hat dein Konto @ifahrentholz keinen Zugriff. Bitte jemanden mit Zugriff, dich einzuladen, oder
          verbinde ein anderes Konto.
        </Callout>
      )}
    </div>
  )
}

function PollingFooter({ limited }: { limited?: boolean }) {
  return (
    <div className="border-t border-border px-3 py-2">
      <NetNote>{limited ? 'Fragt github.com ab · pausiert bis 15:00' : 'Fragt github.com ab · aktualisiert vor 12 s · alle 30 s, solange du hinsiehst'}</NetNote>
    </div>
  )
}

function PanelBody({ state }: { state: string }) {
  if (state === 'no-provider') {
    return (
      <F id={['GIT-001', 'GIT-005']} className="px-4 py-10 text-center">
        <p className="text-[14px] font-medium">Kein Provider für dieses Remote</p>
        <p className="mx-auto mt-1 max-w-xs text-[13px] text-muted-foreground">
          <code className="font-mono text-[12px]">git@git.internal:legacy/tools.git</code> gehört zu keinem eingetragenen Host. Pushen und Committen funktionieren trotzdem.
        </p>
        <Btn size="sm" className="mt-3">
          Host eintragen
        </Btn>
      </F>
    )
  }
  if (state === 'connect-account') {
    return (
      <F id="GIT-004" className="px-4 py-10 text-center">
        <p className="text-[14px] font-medium">Verbinde dein GitHub-Konto</p>
        <p className="mx-auto mt-1 max-w-xs text-[13px] text-muted-foreground">
          Diese Session gehört Ingo Fahrentholz. Status, Checks und Reviews zeigt beton dir mit deinem eigenen Konto, nicht mit Ingos Zugang.
        </p>
        <Btn variant="primary" className="mt-3">
          Mit GitHub verbinden
        </Btn>
        <NetNote className="mt-3">Optional · verbindet beton mit github.com</NetNote>
      </F>
    )
  }

  const gitlab = state === 'gitlab'
  return (
    <F id={['GIT-005', 'GIT-006', gitlab ? 'GIT-003' : 'GIT-002']} className="flex min-h-full flex-col">
      <PanelHeader
        title="Pull Requests"
        meta={gitlab ? 'gitlab.acme.corp · shop/backend/api' : 'github.com · acme/shop-frontend'}
        actions={
          <Btn size="sm" variant="ghost">
            <ArrowUpRight className="size-3.5" />
          </Btn>
        }
      />
      {state === 'rate-limit' && (
        <div className="border-b border-border px-3 py-2">
          <Callout>
            <p className="font-medium">GitHub-Abfragelimit erreicht</p>
            <p className="mt-0.5 text-muted-foreground">beton fragt ab 15:00 wieder ab (in 23 Min.). Du siehst den Stand von 14:37.</p>
          </Callout>
        </div>
      )}
      <CrHead gitlab={gitlab} />
      <SubTabs value="overview" />
      {gitlab ? <GitlabPipeline /> : <GithubChecks />}
      <OtherCrs attach={state === 'attach'} />
      <div className="mt-auto">{gitlab ? <div className="border-t border-border px-3 py-2"><NetNote>Fragt gitlab.acme.corp ab · aktualisiert vor 8 s</NetNote></div> : <PollingFooter limited={state === 'rate-limit'} />}</div>
    </F>
  )
}

export function PrPanelScreen({ state }: { state: string }) {
  const sent = state === 'sent'
  return (
    <SessionShell
      rail={
        <WorkspaceRail active="pr" width="w-[480px]">
          <PanelBody state={sent ? 'github' : state} />
        </WorkspaceRail>
      }
      composer={
        sent ? (
          <F id="GIT-006">
            <Composer harness="claude" running queued={['Check „e2e / login-flow“ fehlgeschlagen · 200 Log-Zeilen angehängt · bitte beheben']} />
          </F>
        ) : undefined
      }
      status={sent ? 'running' : 'idle'}
      connection={state === 'connect-account' ? 'server' : 'local'}
    >
      <ShortStream />
    </SessionShell>
  )
}

/* ---------- Review & Diff ---------- */

function ReviewThread({ resolved, address }: { resolved?: boolean; address?: boolean }) {
  return (
    <div className={cn('rounded-md border border-border bg-card font-sans', resolved && 'opacity-55')}>
      <div className="flex items-center gap-2 border-b border-border px-2.5 py-1.5 text-[11px] text-muted-foreground">
        <span className="font-mono">{resolved ? 'src/routes/auth.ts Z. 3' : 'src/middleware/rate-limit.ts Z. 12'}</span>
        <span className="ml-auto">{resolved ? <Tag tone="muted">erledigt</Tag> : <Tag>Änderung gewünscht</Tag>}</span>
      </div>
      <div className="flex flex-col gap-2 p-2.5 text-[13px]">
        <div className="flex gap-2">
          <Avatar name="Anna Becker" size="sm" />
          <div>
            <span className="font-medium">anna-becker</span> <span className="text-[11px] text-muted-foreground">auf GitHub · 14:20</span>
            <p className="mt-0.5">{resolved ? 'Import sortieren, bitte.' : 'buckets wächst ohne Obergrenze. Bitte nach Ablauf des Fensters aufräumen oder eine LRU mit fester Größe nehmen.'}</p>
          </div>
        </div>
        {!resolved && (
          <div className="flex gap-2">
            <Avatar name="Ingo Fahrentholz" size="sm" />
            <div>
              <span className="font-medium">ifahrentholz</span> <span className="text-[11px] text-muted-foreground">aus beton · 14:24</span>
              <p className="mt-0.5">Guter Punkt, lasse ich den Agent umsetzen.</p>
            </div>
          </div>
        )}
      </div>
      {!resolved && (
        <div className="flex items-center gap-1 border-t border-border px-2 py-1.5">
          <FakeInput placeholder="Antworten als @ifahrentholz auf GitHub" className="h-7 flex-1 text-xs" />
          <Btn size="sm" variant={address ? 'primary' : 'ghost'}>
            <CornerDownRight className="size-3.5" /> An Agent
          </Btn>
        </div>
      )}
    </div>
  )
}

type DLine = { k: 'ctx' | 'add' | 'del'; n: number; t: string }
const rlLines: DLine[] = [
  { k: 'add', n: 8, t: 'export function rateLimit(opts: Options): RequestHandler {' },
  { k: 'add', n: 9, t: '  const buckets = new Map<string, Bucket>()' },
  { k: 'add', n: 10, t: '  const refillPerMs = opts.max / WINDOW_MS[opts.window]' },
  { k: 'add', n: 11, t: '' },
  { k: 'add', n: 12, t: '  return (req, res, next) => {' },
  { k: 'add', n: 13, t: "    const id = opts.key === 'ip' ? req.ip : req.user?.id" },
]

function StackedDiff() {
  const files = [
    { path: 'src/middleware/rate-limit.ts', add: 26, del: 0, open: true },
    { path: 'src/routes/auth.ts', add: 2, del: 1, open: false },
    { path: 'src/middleware/rate-limit.spec.ts', add: 48, del: 0, open: false },
  ]
  return (
    <div className="flex flex-col gap-2 p-3">
      {files.map((f) => (
        <div key={f.path} className="overflow-hidden rounded-md border border-border bg-card font-mono text-[12px]">
          <div className="flex items-center gap-2 border-b border-border bg-sunken px-3 py-1 text-[11px]">
            <span aria-hidden>{f.open ? '▾' : '▸'}</span>
            <span className="truncate">{f.path}</span>
            <span className="ml-auto">
              <span className="text-ok">+{f.add}</span> <span className="text-deny">−{f.del}</span>
            </span>
          </div>
          {f.open &&
            rlLines.map((l) => (
              <div key={l.n}>
                <div className={cn('flex', l.k === 'add' && 'bg-ok-soft', l.k === 'del' && 'bg-deny-soft')}>
                  <span className="w-10 shrink-0 pr-2 text-right text-muted-foreground select-none">{l.n}</span>
                  <span className="w-4 shrink-0 text-muted-foreground select-none">{l.k === 'add' ? '+' : l.k === 'del' ? '−' : ''}</span>
                  <span className="whitespace-pre">{l.t}</span>
                </div>
                {l.n === 12 && (
                  <div className="border-y border-border bg-background p-2">
                    <ReviewThread />
                  </div>
                )}
              </div>
            ))}
        </div>
      ))}
    </div>
  )
}

export function ReviewScreen({ state }: { state: string }) {
  const tab = state === 'threads' ? 'reviews' : 'diff'
  return (
    <SessionShell
      sessionList={false}
      rail={
        <WorkspaceRail active="pr" width="w-[640px]">
          <div className="flex min-h-full flex-col">
            <PanelHeader title="#482 Rate-Limiter für die Login-API" meta="github.com · acme/shop-frontend" />
            <SubTabs value={tab} reviews={2} />
            {state === 'unpushed' && (
              <F id="GIT-008" className="border-b border-border px-3 py-2">
                <Callout className="flex items-start gap-3">
                  <span className="flex-1">
                    <span className="font-medium">2 Commits nicht gepusht</span> · dazu 1 Datei nicht committet. Der Diff unten zeigt den Stand auf GitHub.
                    <span className="mt-1 block font-mono text-[11px] text-muted-foreground">7c2e0aa Buckets nach Ablauf aufräumen · 1f9b3d4 Register-Route limitieren</span>
                  </span>
                  <Btn size="sm">
                    <Upload className="size-3.5" /> Pushen
                  </Btn>
                </Callout>
                <p className="mt-1 text-[11px] text-muted-foreground">Pushen läuft über die Policy des Projects; sie fragt dich vorher.</p>
              </F>
            )}
            {tab === 'diff' ? (
              <F id={['GIT-008', 'GIT-007']}>
                <div className="flex items-center gap-2 px-3 pt-2 text-[12px] text-muted-foreground">
                  Diff wie auf GitHub, gegen main · 3 Dateien · <span className="text-ok">+76</span> <span className="text-deny">−1</span>
                </div>
                <StackedDiff />
              </F>
            ) : (
              <F id="GIT-007" className="flex flex-col gap-2 p-3">
                <ReviewThread address />
                <ReviewThread resolved />
                <p className="text-[12px] text-muted-foreground">Antworten erscheinen auf GitHub unter deinem Namen. „An Agent“ reiht den Thread samt Codeausschnitt in die Warteschlange ein.</p>
              </F>
            )}
            <div className="mt-auto border-t border-border px-3 py-2">
              <NetNote>Fragt github.com ab · aktualisiert vor 12 s</NetNote>
            </div>
          </div>
        </WorkspaceRail>
      }
    >
      <ShortStream compact />
    </SessionShell>
  )
}

/* ---------- PR aus der Session erstellen ---------- */

function CreateForm({ disabled }: { disabled?: boolean }) {
  return (
    <F id="GIT-009" className="flex min-h-full flex-col">
      <PanelHeader title="Pull Request erstellen" meta="github.com · acme/shop-frontend" />
      <div className="flex flex-col gap-3 p-3">
        <div className="font-mono text-[12px] text-muted-foreground">beton/rate-limiter-7f3k → main {disabled ? '· 0 Commits' : '· 2 Commits'}</div>
        <label className="text-[12px] font-medium">
          Titel
          <FakeInput value="Rate-Limiter für die Login-API" className="mt-1" />
        </label>
        <label className="text-[12px] font-medium">
          Beschreibung <span className="font-normal text-muted-foreground">· aus Session und Änderungen vorgeschlagen</span>
          <div className="mt-1 rounded-md border border-input bg-card p-2.5 font-mono text-[12px] leading-relaxed">{`## Was
Begrenzt POST /auth/login auf 5 Versuche pro Minute und IP.
Danach 429 mit Retry-After.

## Wie
- Neue Middleware src/middleware/rate-limit.ts (Token-Bucket)
- 10 Tests, alle grün

---
Erstellt mit beton · Session ses_7f3k`}</div>
        </label>
        <div className="flex flex-col gap-1.5 text-[13px]">
          <label className="flex items-center gap-2">
            <span role="checkbox" aria-checked={false} className="size-3.5 rounded-[3px] border border-muted-foreground" />
            Als Entwurf erstellen
          </label>
          <label className="flex items-center gap-2">
            <span role="checkbox" aria-checked className="flex size-3.5 items-center justify-center rounded-[3px] border border-foreground bg-foreground text-[10px] text-background">
              ✓
            </span>
            Hinweis „Erstellt mit beton“ anhängen
          </label>
        </div>
        <ol className="flex flex-col gap-1 border-t border-border pt-3 text-[12px] text-muted-foreground">
          <li>1. Branch pushen · Policy git-push-fragen: du wirst gefragt</li>
          <li>2. Pull Request auf github.com anlegen, mit dieser Session verknüpfen</li>
        </ol>
        {disabled ? (
          <div>
            <Btn variant="primary" disabled>
              Pushen und PR erstellen
            </Btn>
            <p className="mt-1.5 text-[12px] text-muted-foreground">
              Der Branch hat noch keine Commits gegenüber main. 3 Dateien sind nicht committet; lass den Agent committen oder committe selbst.
            </p>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <Btn variant="primary">Pushen und PR erstellen</Btn>
            <NetNote>Pusht zu github.com</NetNote>
          </div>
        )}
      </div>
    </F>
  )
}

export function CreatePrScreen({ state }: { state: string }) {
  const showForm = state === 'form' || state === 'disabled'
  return (
    <SessionShell
      status={state === 'approval' ? 'waiting' : 'idle'}
      rail={
        <WorkspaceRail active="pr" width="w-[460px]">
          {showForm ? (
            <CreateForm disabled={state === 'disabled'} />
          ) : (
            <F id={['GIT-009', 'GIT-005']} className="flex flex-col">
              <PanelHeader title="Pull Request erstellen" meta="github.com · acme/shop-frontend" />
              <ol className="flex flex-col gap-2 p-3 text-[13px]">
                <li className="flex items-center gap-2">
                  {state === 'approval' ? <span className="chamfer-sm size-2.5 bg-signal" /> : state === 'denied' ? <span className="size-2 rotate-45 bg-deny" /> : <span className="text-ok">✓</span>}
                  Branch pushen
                  <span className="text-[12px] text-muted-foreground">{state === 'approval' ? 'wartet auf deine Freigabe' : state === 'denied' ? 'von Policy abgelehnt' : 'erledigt'}</span>
                </li>
                <li className={cn('flex items-center gap-2', state !== 'done' && 'text-muted-foreground')}>
                  {state === 'done' ? <span className="text-ok">✓</span> : <span className="size-2 rounded-full border border-muted-foreground" />}
                  PR anlegen
                  {state === 'done' && <span className="text-[12px] text-muted-foreground">#482 · in dieser Session erstellt</span>}
                  {state === 'denied' && <span className="text-[12px]">nicht angelegt</span>}
                </li>
              </ol>
            </F>
          )}
        </WorkspaceRail>
      }
    >
      <ShortStream />
      {state === 'approval' && (
        <>
          <UserMessage>PR erstellen</UserMessage>
          <ToolCall kind="git" name="Shell" target="git push -u origin beton/rate-limiter-7f3k" status="waiting" policy="git-push-fragen" />
          <F id="GIT-009">
            <ApprovalCard
              tool="Shell"
              command="git push -u origin beton/rate-limiter-7f3k"
              rule="git-push-fragen (Project shop-frontend)"
              reason="Für den Pull Request muss der Branch zu github.com. Die Projekt-Policy verlangt dafür deine Freigabe."
            />
          </F>
        </>
      )}
      {state === 'done' && (
        <>
          <ToolCall kind="git" name="Shell" target="git push -u origin beton/rate-limiter-7f3k" duration="1,9 s" />
          <F id={['GIT-009', 'GIT-005']}>
            <SystemNote tone="ok">Pull Request #482 erstellt · github.com/acme/shop-frontend/pull/482</SystemNote>
          </F>
        </>
      )}
      {state === 'denied' && (
        <>
          <ToolCall kind="git" name="Shell" target="git push -u origin beton/rate-limiter-7f3k" status="denied" policy="kein-push-freitags" />
          <F id="GIT-009">
            <SystemNote tone="deny">Push abgelehnt durch deine Policy kein-push-freitags · kein Pull Request angelegt</SystemNote>
          </F>
        </>
      )}
    </SessionShell>
  )
}
