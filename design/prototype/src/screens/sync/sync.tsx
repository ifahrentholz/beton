import type { ReactNode } from 'react'
import { ArrowUp, CloudOff, GitFork, Inbox, Laptop, RefreshCw, Server } from 'lucide-react'
import { Composer, SessionHeader } from '@/app/session-chrome'
import { AgentMessage, SystemNote, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, C, Callout, InlineDialog, KV, PageHead, Premise, PrimaryButton, Scroll, Section, SettingsLayout, Shell, Tag } from '../policies/kit'

/** Home-Knoten und Sync-Zustand an der Session (SYNC-001). */
function HomeBadge({ home, epoch, sync }: { home: string; epoch: number; sync: 'synced' | 'behind' | 'offline' | 'local' | 'provisional' }) {
  return (
    <F id="SYNC-001" as="span" badge="bottom-left">
      <span className="inline-flex items-center gap-1.5 rounded-md border border-border px-2 py-0.5 text-[11px]" title={`Home-Knoten ${home}, Epoche ${epoch}`}>
        {home === 'dieser Rechner' ? <Laptop className="size-3" /> : <Server className="size-3" />}
        <span>Home: {home}</span>
        {sync !== 'local' && <span className="text-muted-foreground">· Epoche {epoch}</span>}
        <span className="text-muted-foreground">
          ·{' '}
          {sync === 'synced'
            ? 'synchron'
            : sync === 'behind'
              ? 'holt auf'
              : sync === 'offline'
                ? 'offline'
                : sync === 'provisional'
                  ? 'vorläufig'
                  : 'nur lokal'}
        </span>
        {sync === 'synced' && <span className="size-1.5 rounded-full bg-ok" />}
        {sync === 'offline' && <span className="size-1.5 rounded-full border border-muted-foreground" />}
        {sync === 'provisional' && <span className="size-1.5 rotate-45 bg-foreground" />}
      </span>
    </F>
  )
}

function Banner({ children, icon }: { children: ReactNode; icon?: ReactNode }) {
  return <div className="flex shrink-0 items-center gap-2 border-b border-border bg-sunken px-4 py-1.5 text-[12px]">{icon}{children}</div>
}

function Stream({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">{children}</div>
    </div>
  )
}

const baseStream = (
  <>
    <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.</UserMessage>
    <ToolCall kind="edit" name="Bearbeiten" target="src/middleware/rate-limit.ts" duration="0,3 s" />
    <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth" duration="4,8 s" />
    <AgentMessage harness="claude">
      <p>Die Login-Route ist jetzt begrenzt. Alle 10 Tests laufen. Soll ich den Branch pushen?</p>
    </AgentMessage>
  </>
)

/* ------------------------------------------------------------------ Home & Replica */

export function SyncHome({ state }: { state: string }) {
  const replica = state === 'replica' || state === 'unreachable'
  const unreachable = state === 'unreachable'
  return (
    <Shell nav="sessions" activeSession="ses_7f3k" connection={state === 'local-only' ? 'local' : 'server'}>
      <SessionHeader
        title="Rate-Limiter für die Login-API"
        harness="claude"
        status="idle"
        branch="beton/rate-limiter-7f3k"
        extra={
          <HomeBadge
            home={replica ? 'ingo-mbp' : 'dieser Rechner'}
            epoch={4}
            sync={state === 'local-only' ? 'local' : unreachable ? 'offline' : 'synced'}
          />
        }
      />
      {state === 'home-here' && (
        <F id={['SYNC-001', 'SYNC-002']}>
          <Banner icon={<Laptop className="size-3.5 text-muted-foreground" />}>
            Läuft auf diesem Rechner und schreibt das Log. Kopie auf beton.example.com, für Mara (Lesen) live sichtbar · bestätigt bis seq 1 842.
          </Banner>
        </F>
      )}
      {state === 'local-only' && (
        <F id="SYNC-001">
          <Banner icon={<Laptop className="size-3.5 text-muted-foreground" />}>
            Nur lokal: Diese Session verlässt deinen Rechner nicht. Kein Team-Server eingerichtet.
          </Banner>
        </F>
      )}
      {replica && (
        <F id={['SYNC-002', 'SYNC-003']}>
          <Banner icon={<Server className="size-3.5 text-muted-foreground" />}>
            Du siehst eine Kopie. Die Session läuft auf <b>ingo-mbp</b>; deine Eingaben und Freigaben werden dorthin weitergeleitet.
            <span className="ml-auto">
              <Btn tone="quiet">Session hierher holen …</Btn>
            </span>
          </Banner>
        </F>
      )}
      <Stream>
        {baseStream}
        {state === 'replica' && (
          <F id="SYNC-003" className="flex flex-col gap-1">
            <UserMessage>Ja, push und öffne einen PR.</UserMessage>
            <span className="ml-9 text-[11px] text-muted-foreground">An ingo-mbp weitergeleitet · angenommen 14:22:08</span>
          </F>
        )}
        {unreachable && (
          <F id={['SYNC-003', 'SYNC-008']} className="flex flex-col gap-3">
            <UserMessage queued>Ja, push und öffne einen PR.</UserMessage>
            <Callout
              title="ingo-mbp ist gerade nicht erreichbar"
              actions={
                <>
                  <PrimaryButton>Zurückhalten und später zustellen</PrimaryButton>
                  <Btn>Verwerfen</Btn>
                  <Btn tone="quiet">Session erzwungen übernehmen …</Btn>
                </>
              }
            >
              <p>Die Session läuft dort und nur dort weiter. Deine Nachricht wurde nicht zugestellt.</p>
              <p className="text-muted-foreground">Zurückgehaltene Eingaben bleiben 24 Std. in der Reihenfolge, in der du sie geschickt hast.</p>
            </Callout>
          </F>
        )}
      </Stream>
      <Composer harness="claude" placeholder={replica ? 'Nachricht – wird an ingo-mbp weitergeleitet' : undefined} />
    </Shell>
  )
}

/* ------------------------------------------------------------------ Geplante Übernahme */

export function SyncTake({ state }: { state: string }) {
  const waiting = state === 'waiting'
  const done = state === 'done'
  return (
    <Shell
      nav="sessions"
      activeSession="ses_7f3k"
      connection="server"
      overlay={
        state === 'confirm' ? (
          <InlineDialog
            title="Session auf diesen Rechner holen?"
            footer={
              <>
                <Btn tone="quiet">Abbrechen</Btn>
                <PrimaryButton>Hierher holen</PrimaryButton>
              </>
            }
          >
            <p>
              „Rate-Limiter für die Login-API“ läuft auf <b>ingo-mbp</b>. Nach der Übernahme läuft sie auf <b>MacBook Air (Zug)</b> – auch ohne Netz.
            </p>
            <ol className="ml-4 list-decimal space-y-0.5 text-[13px]">
              <li>ingo-mbp beendet den laufenden Turn (höchstens 10 Min.)</li>
              <li>alle Ereignisse werden hierher übertragen</li>
              <li>der Agent startet hier neu und setzt fort</li>
            </ol>
            <p className="text-[12px] text-muted-foreground">Nur Owner und Org-Admins können Sessions übernehmen. Danach schreibt nur noch dieser Rechner.</p>
          </InlineDialog>
        ) : undefined
      }
    >
      <SessionHeader
        title="Rate-Limiter für die Login-API"
        harness="claude"
        status={waiting ? 'running' : 'idle'}
        branch="beton/rate-limiter-7f3k"
        extra={<HomeBadge home={done ? 'dieser Rechner' : 'ingo-mbp'} epoch={done ? 5 : 4} sync={waiting ? 'behind' : 'synced'} />}
      />
      {waiting && (
        <F id="SYNC-004">
          <div className="flex shrink-0 items-center gap-3 border-b border-border bg-sunken px-4 py-2 text-[12px]">
            <RefreshCw className="size-3.5 animate-spin text-muted-foreground" />
            <span>Übernahme angefragt – wartet, bis der Agent auf ingo-mbp den Turn beendet (noch höchstens 8:40).</span>
            <span className="ml-auto flex gap-2">
              <Btn>Turn unterbrechen und übernehmen</Btn>
              <Btn tone="quiet">Abbrechen</Btn>
            </span>
          </div>
        </F>
      )}
      <Stream>
        {baseStream}
        {waiting && <ToolCall kind="shell" name="Shell" target="pnpm vitest run --coverage" status="running" />}
        {done && (
          <F id="SYNC-004" className="flex flex-col gap-3">
            <SystemNote>Session übernommen · jetzt auf diesem Rechner · Epoche 5 (geplant)</SystemNote>
            <SystemNote>Claude Code hier neu gestartet und fortgesetzt</SystemNote>
          </F>
        )}
      </Stream>
      <Composer harness="claude" running={waiting} />
    </Shell>
  )
}

/* ------------------------------------------------------------------ Erzwungene Übernahme & Fork */

function Lane({ label, from, to, kept, children }: { label: ReactNode; from: number; to: number; kept?: boolean; children?: ReactNode }) {
  return (
    <div className="flex items-center gap-3">
      <div className="w-56 shrink-0 text-[12px]">{label}</div>
      <div className="relative h-6 flex-1">
        <span className="absolute top-1/2 h-px w-full bg-border" />
        <span
          className={cn('absolute top-1/2 h-2 -translate-y-1/2 rounded-sm', kept ? 'bg-foreground/70' : 'border border-dashed border-foreground/60 bg-transparent')}
          style={{ left: `${from}%`, width: `${to - from}%` }}
        />
        {children}
      </div>
    </div>
  )
}

export function SyncForced({ state }: { state: string }) {
  const fork = state === 'fork'
  const provisional = state === 'provisional'
  const confirmed = state === 'confirmed'
  return (
    <Shell
      nav="sessions"
      activeSession="ses_7f3k"
      connection={provisional ? 'offline' : 'server'}
      overlay={
        state === 'force-dialog' ? (
          <InlineDialog
            title="Übernahme erzwingen?"
            footer={
              <>
                <Btn tone="quiet">Abbrechen</Btn>
                <Btn tone="deny">Erzwungen übernehmen</Btn>
              </>
            }
          >
            <p>
              <b>ingo-mbp</b> antwortet seit 47 Min. nicht. Du kannst hier trotzdem weiterarbeiten – vorläufig, ab seq 1 840.
            </p>
            <p>
              Schreibt ingo-mbp in der Zwischenzeit nichts, wird die Übernahme beim nächsten Kontakt bestätigt. Hat er weitergeschrieben, entsteht automatisch ein
              Fork: Nichts geht verloren, aber es wird nie zusammengeführt.
            </p>
          </InlineDialog>
        ) : undefined
      }
    >
      <SessionHeader
        title="Rate-Limiter für die Login-API"
        harness="claude"
        status="idle"
        branch="beton/rate-limiter-7f3k"
        extra={<HomeBadge home={provisional || confirmed ? 'dieser Rechner' : 'ingo-mbp'} epoch={provisional || confirmed ? 5 : 4} sync={provisional ? 'provisional' : state === 'force-dialog' ? 'offline' : 'synced'} />}
      />
      {provisional && (
        <F id="SYNC-005">
          <Banner icon={<CloudOff className="size-3.5 text-muted-foreground" />}>
            Vorläufig übernommen (erzwungen, ab seq 1 840). Wird beim nächsten Kontakt mit dem Team-Server bestätigt – oder als Fork abgetrennt.
          </Banner>
        </F>
      )}
      <Stream>
        {fork ? (
          <F id="SYNC-005" className="flex flex-col gap-4">
            <Callout
              title="Zwei Rechner haben gleichzeitig geschrieben – deine Arbeit liegt in einem Fork"
              actions={
                <>
                  <PrimaryButton>
                    <GitFork className="size-3.5" /> Fork öffnen
                  </PrimaryButton>
                  <Btn tone="quiet">Verstanden</Btn>
                </>
              }
            >
              <p>
                Während du auf dem MacBook Air erzwungen übernommen hattest, lief die Session auf ingo-mbp weiter. Die Session behält den Stand von ingo-mbp
                (so steht es im Verzeichnis des Team-Servers). Deine 15 Ereignisse ab seq 1 840 sind in einen neuen Fork gewandert.
              </p>
            </Callout>
            <div className="rounded-md border border-border bg-card p-4">
              <div className="mb-3 text-[12px] font-semibold">Was passiert ist</div>
              <div className="space-y-2">
                <Lane label="Gemeinsamer Verlauf · seq 1–1 839" from={0} to={55} kept />
                <Lane
                  label={
                    <span>
                      ingo-mbp · seq 1 840–1 862 <Tag mono={false}>behält ses_7f3k</Tag>
                    </span>
                  }
                  from={55}
                  to={95}
                  kept
                />
                <Lane
                  label={
                    <span>
                      MacBook Air · 15 Ereignisse <Tag mono={false}>neu: ses_8a1x</Tag>
                    </span>
                  }
                  from={55}
                  to={82}
                >
                  <span className="absolute top-1/2 left-[55%] -translate-x-1/2 -translate-y-1/2 bg-card px-1">
                    <GitFork className="size-3.5" />
                  </span>
                </Lane>
              </div>
              <p className="mt-3 text-[12px] text-muted-foreground">
                Der Fork enthält den gemeinsamen Verlauf bis seq 1 839 plus deine Ereignisse. Hinweis liegt auch in deiner Inbox.
              </p>
            </div>
          </F>
        ) : (
          <>
            {baseStream}
            {provisional && (
              <F id="SYNC-005" className="flex flex-col gap-3">
                <SystemNote>Erzwungen übernommen · Epoche 5 (vorläufig) · Basis seq 1 840</SystemNote>
                <UserMessage>Ok, dann push ich später. Schreib bitte noch die Doku für die Middleware.</UserMessage>
                <ToolCall kind="edit" name="Bearbeiten" target="docs/middleware.md" duration="0,4 s" />
              </F>
            )}
            {confirmed && (
              <F id="SYNC-005">
                <SystemNote tone="ok">Übernahme bestätigt: ingo-mbp hat seit seq 1 840 nichts geschrieben · kein Fork nötig</SystemNote>
              </F>
            )}
          </>
        )}
      </Stream>
    </Shell>
  )
}

/* ------------------------------------------------------------------ Offline */

export function SyncOffline({ state }: { state: string }) {
  const stale = state === 'stale'
  const reconnect = state === 'reconnect'
  return (
    <Shell nav="sessions" activeSession="ses_7f3k" connection={reconnect ? 'server' : 'offline'}>
      <SessionHeader
        title="Rate-Limiter für die Login-API"
        harness="claude"
        status={stale ? 'waiting' : 'running'}
        branch="beton/rate-limiter-7f3k"
        extra={<HomeBadge home="dieser Rechner" epoch={5} sync={reconnect ? 'synced' : 'offline'} />}
      />
      <F id={['SYNC-006', 'SYNC-007', 'POL-021']}>
        <div className="grid shrink-0 grid-cols-3 gap-0 border-b border-border bg-sunken text-[12px]">
          <div className="border-r border-border px-4 py-2">
            <div className="flex items-center gap-1.5 font-medium">
              {reconnect ? <RefreshCw className="size-3.5" /> : <CloudOff className="size-3.5" />}
              {reconnect ? 'Wieder verbunden' : 'Offline seit 2 Std. 10 Min.'}
            </div>
            <div className="text-muted-foreground">{reconnect ? '23 Ereignisse übertragen, bestätigt bis seq 1 871' : 'Sessions auf diesem Rechner laufen weiter'}</div>
          </div>
          <div className="border-r border-border px-4 py-2">
            <div className="font-medium">Team-Policies: Bundle v41</div>
            <div className="text-muted-foreground">
              {stale ? 'älter als 7 Tage – jeder Tool-Call fragt nach' : reconnect ? 'aktuell (v42 geladen, gilt ab der nächsten Auswertung)' : 'zwischengespeichert, gültig noch 4 Tage · lokal nur verschärfbar'}
            </div>
          </div>
          <div className="px-4 py-2">
            <div className="font-medium">Offline-Budget</div>
            <div className="text-muted-foreground">{reconnect ? '6,60 USD abgerechnet, 3,40 USD freigegeben' : 'noch 3,40 von 10,00 USD · bis heute 24:00'}</div>
          </div>
        </div>
      </F>
      <Stream>
        {baseStream}
        {stale ? (
          <F id={['SYNC-006', 'POL-008']} className="flex flex-col gap-3">
            <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth --coverage" status="waiting" policy="Policy-Cache veraltet" />
            <Callout
              tone="wait"
              title="Freigabe nötig: Team-Policies sind veraltet"
              actions={
                <>
                  <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Erlauben</button>
                  <Btn>Ablehnen</Btn>
                </>
              }
            >
              <p>
                Der Rechner hatte 8 Tage keinen Kontakt zum Team-Server. Weil die Org-Regeln inzwischen anders sein könnten, fragt beton bei jedem Tool-Call und
                jeder Modell-Anfrage nach. Läuft niemand zu, wird abgelehnt.
              </p>
              <p className="text-muted-foreground">
                Neue Sessions starten erst wieder nach einer Aktualisierung (<C>max_age</C> 7 Tage).
              </p>
            </Callout>
          </F>
        ) : reconnect ? (
          <F id={['SYNC-008', 'SYNC-007', 'SYNC-002']} className="flex flex-col gap-3">
            <SystemNote tone="ok">Wieder verbunden · Ereignisse und Offline-Kosten abgeglichen</SystemNote>
            <div className="ml-9 rounded-md border border-border bg-card p-3 text-[12px]">
              <div className="mb-1 flex items-center gap-1.5 font-semibold">
                <Inbox className="size-3.5" /> Zurückgehaltene Eingaben von anderen Geräten
              </div>
              <div className="flex items-center gap-2 py-0.5">
                <ArrowUp className="size-3 text-ok" /> Mara (Handy): „Bitte auch die Register-Route absichern“ – zugestellt, eingereiht
              </div>
              <div className="flex items-center gap-2 py-0.5 text-muted-foreground">
                <span className="size-2 rotate-45 bg-deny" /> Ingo (Handy): Freigabe „git push“ – verworfen, die Freigabe war schon per Timeout abgelehnt
              </div>
            </div>
          </F>
        ) : (
          <F id={['SYNC-006', 'SYNC-007']} className="flex flex-col gap-3">
            <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth --coverage" status="running" policy="Bundle v41 (Cache)" />
          </F>
        )}
      </Stream>
      <Composer harness="claude" running={!stale && !reconnect} />
    </Shell>
  )
}

/* ------------------------------------------------------------------ Handy: Weiterleitung & Outbox */

export function SyncMobile({ state }: { state: string }) {
  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3">
        <span className="truncate text-[14px] font-semibold">Rate-Limiter für die Login-API</span>
      </div>
      <div className="flex shrink-0 items-center gap-1.5 border-b border-border bg-sunken px-3 py-1 text-[11px] text-muted-foreground">
        <Laptop className="size-3" /> läuft auf ingo-mbp · {state === 'forwarded' ? 'erreichbar' : 'nicht erreichbar'}
      </div>
      <F id={['SYNC-003', 'SYNC-008']} className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3 text-[13px]">
        <div className="rounded-md border border-border bg-card p-2.5">
          <div className="text-[11px] text-muted-foreground">Claude Code</div>
          <p>Alle 10 Tests laufen. Soll ich den Branch pushen?</p>
        </div>
        {state === 'forwarded' && (
          <>
            <div className="rounded-md border border-border bg-card p-2.5">
              <div className="font-semibold">Erlaubt: git push</div>
              <div className="mt-1 text-[12px] text-muted-foreground">An ingo-mbp zugestellt · wirksam nach 1,2 s</div>
            </div>
          </>
        )}
        {state === 'home-offline' && (
          <div className="chamfer border-l-4 border-signal bg-signal-soft p-3">
            <div className="font-semibold">Freigabe nötig: git push</div>
            <p className="mt-1 text-[12px]">ingo-mbp ist offline. Deine Entscheidung kann erst zugestellt werden, wenn der Rechner wieder online ist.</p>
            <div className="mt-2 flex flex-col gap-1.5">
              <button className="chamfer-sm h-9 bg-foreground text-[13px] font-semibold text-background">Erlauben, sobald erreichbar</button>
              <button className="h-9 rounded-md border border-foreground/30 text-[13px]">Ablehnen, sobald erreichbar</button>
            </div>
          </div>
        )}
        {(state === 'pending' || state === 'discarded') && (
          <div className="rounded-md border border-dashed border-border p-2.5">
            <div className="mb-1 flex items-center gap-1.5 text-[12px] font-semibold">
              <Inbox className="size-3.5" /> Ausstehend
            </div>
            <div className="flex items-start gap-2 py-1">
              <span className={cn('mt-1.5 size-2 shrink-0', state === 'discarded' ? 'rotate-45 bg-deny' : 'rounded-full border border-muted-foreground')} />
              <span>
                Erlauben: git push
                <span className="block text-[11px] text-muted-foreground">
                  {state === 'discarded' ? 'Verworfen: Die Freigabe ist inzwischen abgelaufen (abgelehnt nach 30 Min.).' : 'wartet auf ingo-mbp · seit 12 Min.'}
                </span>
              </span>
            </div>
            <div className="flex items-start gap-2 py-1">
              <span className="mt-1.5 size-2 shrink-0 rounded-full border border-muted-foreground" />
              <span>
                „Bitte auch die Register-Route absichern“
                <span className="block text-[11px] text-muted-foreground">wartet · wird höchstens 24 Std. aufbewahrt</span>
              </span>
            </div>
          </div>
        )}
      </F>
      <div className="shrink-0 border-t border-border p-2">
        <div className="flex h-10 items-center rounded-md border border-input bg-card px-3 text-[13px] text-muted-foreground">
          {state === 'forwarded' ? 'Nachricht' : 'Nachricht – wird zugestellt, sobald ingo-mbp online ist'}
        </div>
      </div>
    </div>
  )
}

/* ------------------------------------------------------------------ Einstellungen */

export function SyncSettings({ state }: { state: string }) {
  const off = state === 'off'
  return (
    <SettingsLayout active="team-server" connection={off ? 'local' : 'server'}>
      <PageHead title="Team-Server & Sync" sub="Optional. Ohne Team-Server läuft alles lokal und nichts verlässt deinen Rechner.">
        <div className="mt-2">
          {off ? <Premise>Nicht verbunden · lokal-only</Premise> : <Premise kind="team">Verbunden mit beton.example.com als Knoten „ingo-mbp“</Premise>}
        </div>
      </PageHead>
      <Scroll className="px-6 py-4">
        <div className="max-w-4xl space-y-5">
          {off ? (
            <F id="SYNC-001">
              <Callout title="Kein Team-Server eingerichtet" actions={<PrimaryButton>Mit Team-Server verbinden …</PrimaryButton>}>
                <p>
                  Alle Sessions, Policies und Budgets leben auf diesem Rechner. Ein Team-Server ist nur nötig, wenn du Sessions mit anderen teilen, vom Handy
                  außerhalb deines Netzes zugreifen oder Org-Policies nutzen willst.
                </p>
              </Callout>
            </F>
          ) : (
            <>
              <F id={['SYNC-001', 'SYNC-002']}>
                <Section title="Welche Sessions synchronisiert werden">
                  {[
                    { id: 'manual', t: 'Nur die, die ich teile oder hochlade', on: false },
                    { id: 'projects', t: 'Alle aus bestimmten Projekten: shop-frontend, infra', on: true },
                    { id: 'all', t: 'Alle', on: false },
                  ].map((o) => (
                    <div key={o.id} className="flex items-center gap-2 py-1 text-[13px]">
                      <span className={cn('size-3 rounded-full border-2', o.on ? 'border-foreground bg-foreground' : 'border-muted-foreground')} />
                      {o.t}
                    </div>
                  ))}
                  <p className="mt-1 text-[12px] text-muted-foreground">
                    Jede Session hat genau einen Home-Knoten, der ihr Log schreibt. Andere Knoten und Geräte sehen eine Kopie und leiten Eingaben weiter.
                  </p>
                </Section>
              </F>
              <div className="grid grid-cols-2 gap-6">
                <F id="SYNC-006">
                  <Section title="Team-Policies (Cache)">
                    <KV k="Bundle">v41 · signiert (Ed25519), geprüft</KV>
                    <KV k="Erhalten">vor 3 Tagen</KV>
                    <KV k="Gültig ohne Kontakt">7 Tage (max_age)</KV>
                    <KV k="Lokal">nur verschärfbar; ältere Versionen werden abgelehnt</KV>
                  </Section>
                </F>
                <F id="SYNC-007">
                  <Section title="Offline-Budget">
                    <KV k="Lease">10,00 USD · Team plattform, täglich</KV>
                    <KV k="Verbraucht">1,90 USD</KV>
                    <KV k="Gemeldet">alle 60 s, wenn online</KV>
                    <KV k="Wenn aufgebraucht">nachfragen</KV>
                  </Section>
                </F>
              </div>
            </>
          )}
        </div>
      </Scroll>
    </SettingsLayout>
  )
}
