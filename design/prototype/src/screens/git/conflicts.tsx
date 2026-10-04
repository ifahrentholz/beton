import type { ReactNode } from 'react'
import { Bot, Check, ChevronDown, FileWarning, GitMerge, Play, Trash2, Undo2 } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { SessionHeader, WorkspaceRail } from '@/app/session-chrome'
import { ApprovalCard } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Callout, DialogPanel, LocalNote, NetNote, Overlay, Tag } from '@/app/kit/workspace'
import { CrStateTag, OriginTag } from './parts'

/*
 * Merge-Konflikte lösen (GIT-010 bis GIT-013).
 * Beispiel: Der Branch „beton/rate-limiter-7f3k“ hat die Login-Route mit einem Rate-Limiter versehen,
 * auf main hat inzwischen jemand CSRF-Schutz an derselben Zeile eingebaut.
 */

type Side = 'base' | 'ours' | 'theirs' | 'result' | 'agent'

const sideStyle: Record<Side, { label: string; tone: string }> = {
  base: { label: 'Basis', tone: 'bg-sunken' },
  ours: { label: 'Dein Branch', tone: 'bg-voice-claude/10' },
  theirs: { label: 'Ziel-Branch (main)', tone: 'bg-voice-codex/10' },
  result: { label: 'Ergebnis', tone: 'bg-card' },
  agent: { label: 'Vorschlag des Agents', tone: 'bg-card' },
}

function CodePane({
  side,
  meta,
  lines,
  start,
  marks,
  className,
  editable,
}: {
  side: Side
  meta?: ReactNode
  lines: string[]
  start: number
  marks?: Partial<Record<number, 'ours' | 'theirs' | 'agent' | 'marker'>>
  className?: string
  editable?: boolean
}) {
  const s = sideStyle[side]
  return (
    <div className={cn('flex min-w-0 flex-col overflow-hidden rounded-md border border-border', className)}>
      <div className="flex items-baseline gap-2 border-b border-border bg-sunken px-2.5 py-1 text-[11px]">
        <span className="font-medium text-foreground">{s.label}</span>
        {meta && <span className="min-w-0 truncate text-muted-foreground">{meta}</span>}
        {editable && <span className="ml-auto text-muted-foreground">bearbeitbar</span>}
      </div>
      <div className={cn('overflow-x-auto py-1 font-mono text-[12px] leading-[1.6]', s.tone)}>
        {lines.map((l, i) => {
          const m = marks?.[i]
          return (
            <div
              key={i}
              className={cn(
                'flex border-l-2 border-transparent',
                m === 'ours' && 'border-l-voice-claude bg-voice-claude/15',
                m === 'theirs' && 'border-l-voice-codex bg-voice-codex/15',
                m === 'agent' && 'border-l-signal bg-signal-soft',
                m === 'marker' && 'border-l-deny bg-deny-soft text-deny',
              )}
            >
              {/* Auf stimmfarbig getönten Zeilen reicht --muted-foreground nicht für AA (UX-005). */}
              <span className={cn('w-9 shrink-0 pr-2 text-right select-none', m === 'ours' || m === 'theirs' ? 'text-foreground/85' : 'text-muted-foreground')}>{start + i}</span>
              <span className="pr-3 whitespace-pre">{l}</span>
            </div>
          )
        })}
      </div>
    </div>
  )
}

/* ---------- Dateiliste ---------- */

type FileState = 'open' | 'resolved' | 'proposal' | 'special'

function FileRow({ path, detail, state, active }: { path: string; detail: string; state: FileState; active?: boolean }) {
  return (
    <div
      className={cn(
        'flex items-start gap-2 border-l-2 px-3 py-2',
        active ? 'border-signal bg-accent' : 'border-transparent hover:bg-accent/60',
      )}
    >
      <span className="mt-0.5">
        {state === 'open' && <span className="chamfer-sm block size-2.5 bg-signal" title="offen" />}
        {state === 'proposal' && <Bot className="size-3.5 text-muted-foreground" aria-label="Vorschlag des Agents" />}
        {state === 'resolved' && <Check className="size-3.5 text-ok" aria-label="gelöst" />}
        {state === 'special' && <FileWarning className="size-3.5 text-deny" aria-label="Sonderfall" />}
      </span>
      <div className="min-w-0 flex-1">
        <div className="truncate font-mono text-[12px]">{path}</div>
        <div className="text-[11px] text-muted-foreground">{detail}</div>
      </div>
    </div>
  )
}

function FileList({ state }: { state: string }) {
  const done = state === 'finish' || state === 'offline'
  return (
    <F id="GIT-011" className="flex w-72 shrink-0 flex-col border-r border-border bg-sidebar">
      <div className="border-b border-border px-3 py-2.5">
        <div className="text-[13px] font-semibold">Konfliktdateien</div>
        <div className="text-[11px] text-muted-foreground">{done ? 'alle gelöst' : '3 Dateien · 2 noch offen'}</div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto py-1">
        <FileRow
          path="src/routes/auth.ts"
          detail={done ? '2 Stellen gelöst · 1 vom Agent' : state === 'agent' ? 'Stelle 2 von 2 · Vorschlag liegt vor' : 'Stelle 2 von 2 offen'}
          state={done ? 'resolved' : state === 'agent' ? 'proposal' : 'open'}
          active={state !== 'delete-modify'}
        />
        <FileRow
          path="src/legacy/session.ts"
          detail={done ? 'behalten (deine Änderung)' : 'auf main gelöscht, bei dir geändert'}
          state={done ? 'resolved' : 'special'}
          active={state === 'delete-modify'}
        />
        <FileRow path="pnpm-lock.yaml" detail="Ziel übernommen und neu erzeugt" state="resolved" />
      </div>
      <F id="GIT-012" className="border-t border-border p-3">
        <Btn size="sm" className="w-full justify-center" disabled={done}>
          <Bot className="size-3.5" /> Alle offenen Stellen vom Agent lösen lassen
        </Btn>
        <p className="mt-1.5 text-[11px] text-muted-foreground">
          Läuft über Claude Code in dieser Session (Claude Max). Vorschläge musst du bestätigen.
        </p>
      </F>
    </F>
  )
}

/* ---------- Kopf des Resolvers ---------- */

function ResolverBar({ state }: { state: string }) {
  const done = state === 'finish' || state === 'offline'
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-border px-4 py-2.5">
      <GitMerge className="size-4 text-muted-foreground" />
      <div className="min-w-0">
        <div className="text-[14px] font-semibold">Konflikte lösen</div>
        <div className="font-mono text-[11px] text-muted-foreground">
          {state === 'rebase' ? 'Rebase auf main · Commit 2 von 5 · „Rate-Limiter-Middleware“' : 'Merge main → beton/rate-limiter-7f3k'}
        </div>
      </div>
      {state === 'rebase' && (
        <ol className="flex items-center gap-1" aria-label="Commits im Rebase">
          {[1, 2, 3, 4, 5].map((n) => (
            <li
              key={n}
              className={cn(
                'flex size-5 items-center justify-center rounded-sm border text-[10px] tabular-nums',
                n < 2 && 'border-ok text-ok',
                n === 2 && 'chamfer-sm border-signal bg-signal-soft font-semibold',
                n > 2 && 'border-border text-muted-foreground',
              )}
            >
              {n}
            </li>
          ))}
        </ol>
      )}
      <LocalNote className="ml-auto">Läuft im Worktree auf diesem Rechner</LocalNote>
      <Btn size="sm" variant="ghost">
        <Undo2 className="size-3.5" /> Abbrechen
      </Btn>
      <F id="GIT-013" as="span" badge="bottom-right">
        <Btn size="sm" variant={done ? 'signal' : 'primary'} disabled={!done} title={done ? undefined : 'Noch 2 Stellen offen'}>
          <Check className="size-3.5" /> Abschließen
        </Btn>
      </F>
    </div>
  )
}

/* ---------- Hunk-Ansicht ---------- */

const BASE = ["import { Router } from 'express'", "import { login } from '../handlers/login'", '', "auth.post('/login', login)"]
const OURS = [
  "import { Router } from 'express'",
  "import { rateLimit } from '../middleware/rate-limit'",
  "import { login } from '../handlers/login'",
  '',
  "auth.post('/login', rateLimit({ window: '1m', max: 5, key: 'ip' }), login)",
]
const THEIRS = [
  "import { Router } from 'express'",
  "import { csrf } from '../middleware/csrf'",
  "import { login } from '../handlers/login'",
  '',
  "auth.post('/login', csrf(), login)",
]

function HunkActions({ agent }: { agent?: boolean }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Btn size="sm">Deine Seite</Btn>
      <Btn size="sm">Ziel-Seite</Btn>
      <Btn size="sm">
        Beide (deine zuerst) <ChevronDown className="size-3" />
      </Btn>
      <Btn size="sm" variant="ghost">
        Bearbeiten
      </Btn>
      <F id="GIT-012" as="span" badge="top-right" className="ml-auto">
        <Btn size="sm" variant="outline" disabled={agent}>
          <Bot className="size-3.5" /> Agent lösen lassen
        </Btn>
      </F>
    </div>
  )
}

function HunkView({ state }: { state: string }) {
  const agent = state === 'agent'
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-4 py-2 text-[12px]">
        <span className="font-mono">src/routes/auth.ts</span>
        <Tag tone="muted">Inhaltskonflikt</Tag>
        <span className="text-muted-foreground">Stelle 2 von 2 · Zeile 16</span>
        <span className="ml-auto text-muted-foreground">Stelle 1 (Imports): beide Seiten übernommen</span>
      </div>
      <div className="flex flex-col gap-3 p-4">
        <CodePane side="base" meta="gemeinsamer Vorfahr · a41c0e2" lines={BASE} start={13} marks={{ 3: 'marker' }} />
        <div className="grid grid-cols-2 gap-3">
          <CodePane side="ours" meta="9f3k2a1 · „Rate-Limiter-Middleware“ · Claude Code" lines={OURS} start={12} marks={{ 1: 'ours', 4: 'ours' }} />
          <CodePane side="theirs" meta="c77d1b9 · „CSRF-Schutz für Login“ · Anna Becker" lines={THEIRS} start={12} marks={{ 1: 'theirs', 4: 'theirs' }} />
        </div>

        {!agent ? (
          <div className="chamfer border-l-4 border-signal bg-signal-soft p-3">
            <div className="mb-2 text-[13px] font-semibold">Wie soll Zeile 16 aussehen?</div>
            <HunkActions />
            <div className="mt-3">
              <CodePane
                side="result"
                editable
                lines={[
                  "import { Router } from 'express'",
                  "import { rateLimit } from '../middleware/rate-limit'",
                  "import { csrf } from '../middleware/csrf'",
                  "import { login } from '../handlers/login'",
                  '',
                  '<<<<<<< beton/rate-limiter-7f3k',
                  "auth.post('/login', rateLimit({ window: '1m', max: 5, key: 'ip' }), login)",
                  '=======',
                  "auth.post('/login', csrf(), login)",
                  '>>>>>>> main',
                ]}
                start={12}
                marks={{ 1: 'ours', 2: 'theirs', 5: 'marker', 7: 'marker', 9: 'marker' }}
              />
            </div>
          </div>
        ) : (
          <F id="GIT-012" className="chamfer border-l-4 border-signal bg-signal-soft p-3">
            <div className="mb-1 flex items-center gap-2 text-[13px] font-semibold">
              <Bot className="size-4" /> Vorschlag von Claude Code
            </div>
            <p className="mb-2 max-w-3xl text-[13px]">
              Beide Middlewares bleiben. Das Rate-Limit steht vor dem CSRF-Check, damit auch Anfragen ohne gültiges
              Token gezählt werden und Brute-Force-Versuche nicht am Limit vorbeikommen.
            </p>
            <CodePane
              side="agent"
              lines={[
                "import { Router } from 'express'",
                "import { rateLimit } from '../middleware/rate-limit'",
                "import { csrf } from '../middleware/csrf'",
                "import { login } from '../handlers/login'",
                '',
                "auth.post('/login', rateLimit({ window: '1m', max: 5, key: 'ip' }), csrf(), login)",
              ]}
              start={12}
              marks={{ 5: 'agent' }}
            />
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <Btn size="sm" variant="signal">
                <Check className="size-3.5" /> Vorschlag übernehmen
              </Btn>
              <Btn size="sm">Bearbeiten</Btn>
              <Btn size="sm" variant="ghost">
                Ablehnen
              </Btn>
              <span className="ml-auto text-[11px] text-muted-foreground">
                1 Turn · über Claude Max · Schreibzugriff von Policy erlaubt
              </span>
            </div>
          </F>
        )}
      </div>
    </div>
  )
}

function DeleteModifyView() {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-4 py-2 text-[12px]">
        <span className="font-mono">src/legacy/session.ts</span>
        <Tag tone="deny">Löschen gegen Ändern</Tag>
      </div>
      <div className="flex max-w-3xl flex-col gap-3 p-4">
        <div className="chamfer border-l-4 border-signal bg-signal-soft p-3 text-[13px]">
          <p className="font-semibold">Auf main wurde diese Datei gelöscht, in deinem Branch geändert.</p>
          <p className="mt-1 text-muted-foreground">
            Anna Becker hat sie in c77d1b9 entfernt („Alte Session-Logik entfernt“). Dein Branch ruft daraus noch{' '}
            <code className="font-mono">touchSession()</code> auf (src/routes/auth.ts:9).
          </p>
          <div className="mt-2.5 flex flex-wrap gap-2">
            <Btn size="sm">Deine Version behalten</Btn>
            <Btn size="sm">
              <Trash2 className="size-3.5" /> Löschen übernehmen
            </Btn>
            <Btn size="sm" variant="outline">
              <Bot className="size-3.5" /> Agent: Aufruf ersetzen und löschen
            </Btn>
          </div>
        </div>
        <CodePane
          side="ours"
          meta="deine Änderung · 2 Zeilen"
          lines={['export function touchSession(id: string) {', '  sessions.get(id)?.refresh()', '  metrics.inc("session_touch")', '}']}
          start={18}
          marks={{ 2: 'ours' }}
        />
      </div>
    </div>
  )
}

/* ---------- Zustände vor und nach dem Resolver ---------- */

function DetectedPanel() {
  return (
    <WorkspaceRail active="pr" width="w-[460px]">
      <div className="border-b border-border px-3 py-2.5">
        <div className="flex items-center gap-2 text-[14px] font-semibold">
          Rate-Limiter für die Login-API <span className="font-mono text-[12px] font-normal text-muted-foreground">#482</span>
        </div>
        <div className="mt-1.5 flex items-center gap-2">
          <CrStateTag state="open" />
          <OriginTag origin="created" />
          <span className="font-mono text-[11px] text-muted-foreground">beton/rate-limiter-7f3k → main</span>
        </div>
      </div>
      <F id={['GIT-010', 'GIT-006']} className="flex flex-col gap-3 p-3">
        <div className="chamfer border-l-4 border-signal bg-signal-soft p-3">
          <div className="text-[13px] font-semibold">Konflikte mit main · 3 Dateien</div>
          <ul className="mt-1.5 space-y-0.5 font-mono text-[12px]">
            <li>src/routes/auth.ts <span className="font-sans text-muted-foreground">· Inhalt, 2 Stellen</span></li>
            <li>src/legacy/session.ts <span className="font-sans text-muted-foreground">· gelöscht gegen geändert</span></li>
            <li>pnpm-lock.yaml <span className="font-sans text-muted-foreground">· Lockfile</span></li>
          </ul>
          <div className="mt-2.5 text-[12px] text-muted-foreground">Wie zusammenführen?</div>
          <div className="mt-1 flex flex-col gap-1 text-[13px]">
            <label className="flex items-start gap-2">
              <input type="radio" name="strategy" defaultChecked className="mt-1" />
              <span>
                <span className="font-medium">main in deinen Branch mergen</span>
                <span className="block text-[12px] text-muted-foreground">Empfohlen: keine umgeschriebene History, kein Force-Push.</span>
              </span>
            </label>
            <label className="flex items-start gap-2">
              <input type="radio" name="strategy" className="mt-1" />
              <span>
                <span className="font-medium">Auf main rebasen</span>
                <span className="block text-[12px] text-muted-foreground">Lineare History; danach ist ein Force-Push nötig, der eine Freigabe braucht.</span>
              </span>
            </label>
          </div>
          <div className="mt-3 flex items-center gap-2">
            <Btn variant="signal">
              <GitMerge className="size-3.5" /> Konflikte lösen
            </Btn>
            <Btn variant="ghost">An den Agent geben</Btn>
          </div>
        </div>
        <div className="flex flex-col gap-1">
          <LocalNote>Lokal geprüft gegen main von vor 12 Min. (git merge-tree, Arbeitsverzeichnis unverändert)</LocalNote>
          <NetNote>GitHub meldet ebenfalls: nicht mergebar · zuletzt abgefragt vor 1 Min.</NetNote>
        </div>
      </F>
    </WorkspaceRail>
  )
}

function FinishDialog({ offline }: { offline?: boolean }) {
  return (
    <Overlay>
      <DialogPanel
        title="Konfliktlösung abschließen"
        subtitle="3 Dateien gelöst: 1 Stelle von dir, 1 vom Agent, 1 beidseitig übernommen, Lockfile neu erzeugt."
        width="w-[600px]"
        footer={
          <>
            <Btn variant="ghost">Zurück zum Resolver</Btn>
            <span className="ml-auto" />
            <Btn variant="primary">
              <Check className="size-3.5" /> Merge-Commit erstellen
            </Btn>
          </>
        }
      >
        <F id="GIT-013" className="flex flex-col gap-3 text-[13px]">
          <div>
            <div className="mb-1 text-[12px] text-muted-foreground">Commit-Nachricht</div>
            <pre className="rounded-md border border-input bg-card px-2.5 py-2 font-mono text-[12px] whitespace-pre-wrap">{`Merge branch 'main' into beton/rate-limiter-7f3k

Konflikte gelöst in src/routes/auth.ts (Rate-Limit vor CSRF),
src/legacy/session.ts (behalten) und pnpm-lock.yaml (neu erzeugt).`}</pre>
          </div>
          <label className="flex items-center gap-2">
            <input type="checkbox" defaultChecked /> Danach die Tests laufen lassen (<code className="font-mono text-[12px]">pnpm vitest run</code>)
          </label>
          <label className="flex items-center gap-2">
            <input type="checkbox" defaultChecked={!offline} disabled={offline} /> Danach pushen und PR #482 aktualisieren
          </label>
          {offline ? (
            <Callout>
              Keine Netzverbindung. Der Merge-Commit entsteht lokal; der Push wartet und lässt sich später im PR-Panel auslösen.
            </Callout>
          ) : (
            <LocalNote>Der Push fragt nach deiner Freigabe (Policy git-push-fragen).</LocalNote>
          )}
        </F>
      </DialogPanel>
    </Overlay>
  )
}

function AbortDialog() {
  return (
    <Overlay>
      <DialogPanel
        role="alertdialog"
        waiting
        title="Konfliktlösung abbrechen?"
        subtitle="Der Merge wird verworfen (git merge --abort). Branch und Arbeitsverzeichnis sind danach wie vor dem Start."
        footer={
          <>
            <Btn variant="ghost">Weiter lösen</Btn>
            <span className="ml-auto" />
            <Btn variant="danger">
              <Undo2 className="size-3.5" /> Abbrechen und zurücksetzen
            </Btn>
          </>
        }
      >
        <p className="text-[13px]">Deine bisherigen Entscheidungen (2 Stellen) gehen dabei verloren.</p>
      </DialogPanel>
    </Overlay>
  )
}

function PushedView() {
  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 px-6 py-6">
      <div className="flex items-center gap-2 text-[13px]">
        <Check className="size-4 text-ok" /> Merge-Commit <span className="font-mono">e5a90c3</span> erstellt · Tests laufen
        <Play className="size-3.5 text-muted-foreground" />
      </div>
      <F id={['GIT-013', 'POL-017']}>
        <ApprovalCard
          tool="Shell"
          command="git push origin beton/rate-limiter-7f3k"
          rule="git-push-fragen (Projekt shop-frontend)"
          reason="Pushes verlassen deinen Rechner. Die Projekt-Policy verlangt dafür deine Freigabe."
        />
      </F>
    </div>
  )
}

/* ---------- Screen ---------- */

export function ConflictScreen({ state }: { state: string }) {
  if (state === 'detected') {
    return (
      <AppLayout activeSession="ses_7f3k" rail={<DetectedPanel />}>
        <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="idle" branch="beton/rate-limiter-7f3k" />
        <div className="concrete-grain flex flex-1 items-center justify-center p-8 text-center text-sm text-muted-foreground">
          Der Agent ist fertig. Bevor der PR gemergt werden kann, müssen die Konflikte mit main gelöst werden.
        </div>
      </AppLayout>
    )
  }

  if (state === 'pushed') {
    return (
      <AppLayout activeSession="ses_7f3k">
        <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="waiting" branch="beton/rate-limiter-7f3k" />
        <PushedView />
      </AppLayout>
    )
  }

  return (
    <div className="relative h-full">
      <AppLayout activeSession="ses_7f3k" sessionList={false}>
        <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="stopped" branch="beton/rate-limiter-7f3k" />
        <ResolverBar state={state} />
        <div className="flex min-h-0 flex-1">
          <FileList state={state} />
          {state === 'delete-modify' ? (
            <DeleteModifyView />
          ) : (
            <F id={['GIT-011', 'WEB-009']} className="flex min-h-0 min-w-0 flex-1 flex-col">
              <HunkView state={state === 'finish' || state === 'offline' ? 'resolve' : state} />
            </F>
          )}
        </div>
      </AppLayout>
      {(state === 'finish' || state === 'offline') && <FinishDialog offline={state === 'offline'} />}
      {state === 'abort' && <AbortDialog />}
    </div>
  )
}
