import { ArrowRight, Container, Laptop, Server, Ship } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { HarnessBadge } from '@/app/harness'
import { Ask, Cursor, L, Prompt, S, TermBlock } from '../cli/term'
import { Btn, C, KV, LabelChip, Mark, Premise, Section, SettingsShell, type MarkKind } from './settings-shell'

/* ───────────────────────── Beispieldaten ───────────────────────── */

type Host = {
  id: string
  name: string
  kind: 'local' | 'remote' | 'cluster'
  state: MarkKind
  stateLabel: string
  os: string
  version: string
  runners: [number, number]
  heartbeat: string
  labels: string[]
  auto: string[]
  providers: string[]
}

const hosts: Host[] = [
  {
    id: 'local',
    name: 'MacBook Pro (dieser Rechner)',
    kind: 'local',
    state: 'ok',
    stateLabel: 'online',
    os: 'macOS 26 · arm64',
    version: '1.3.0',
    runners: [2, 8],
    heartbeat: 'lokal',
    labels: [],
    auto: ['beton.os=macos', 'beton.arch=arm64', 'beton.provider.local', 'beton.provider.docker', 'beton.harness.claude', 'beton.harness.codex', 'beton.harness.gemini'],
    providers: ['local', 'docker'],
  },
  {
    id: 'build-box-01',
    name: 'build-box-01',
    kind: 'remote',
    state: 'ok',
    stateLabel: 'online',
    os: 'Ubuntu 24.04 · amd64',
    version: '1.3.0',
    runners: [1, 8],
    heartbeat: 'vor 4 s',
    labels: ['os=linux', 'arch=amd64', 'repo=beton', 'tier=ci'],
    auto: ['beton.os=linux', 'beton.arch=amd64', 'beton.provider.local', 'beton.provider.docker', 'beton.harness.claude', 'beton.harness.codex'],
    providers: ['local', 'docker'],
  },
  {
    id: 'build-box-02',
    name: 'build-box-02',
    kind: 'remote',
    state: 'ok',
    stateLabel: 'online',
    os: 'Debian 13 · amd64',
    version: '1.3.0',
    runners: [6, 8],
    heartbeat: 'vor 11 s',
    labels: ['os=linux', 'arch=amd64', 'repo=beton'],
    auto: ['beton.os=linux', 'beton.provider.docker', 'beton.harness.claude', 'beton.harness.codex'],
    providers: ['docker'],
  },
  {
    id: 'k8s-runners',
    name: 'k8s-runners',
    kind: 'cluster',
    state: 'ok',
    stateLabel: 'online',
    os: 'Kubernetes 1.34 · Namespace beton-runners',
    version: '1.3.0',
    runners: [3, 20],
    heartbeat: 'vor 2 s',
    labels: ['os=linux', 'tier=async'],
    auto: ['beton.os=linux', 'beton.arch=amd64', 'beton.provider.kubernetes', 'beton.harness.claude', 'beton.harness.codex', 'beton.harness.gemini'],
    providers: ['kubernetes'],
  },
  {
    id: 'gpu-ws',
    name: 'gpu-ws',
    kind: 'remote',
    state: 'off',
    stateLabel: 'offline',
    os: 'Ubuntu 24.04 · amd64',
    version: '1.3.0',
    runners: [0, 2],
    heartbeat: 'vor 2 Std.',
    labels: ['os=linux', 'gpu=true'],
    auto: ['beton.os=linux', 'beton.provider.local', 'beton.harness.claude'],
    providers: ['local'],
  },
]

const kindIcon = { local: Laptop, remote: Server, cluster: Ship }

/* ───────────────────────── Hosts ───────────────────────── */

function HostRow({ h, active, override }: { h: Host; active?: boolean; override?: { state: MarkKind; label: string } }) {
  const Icon = kindIcon[h.kind]
  const st = override ?? { state: h.state, label: h.stateLabel }
  return (
    <div className={cn('grid grid-cols-[1.5rem_minmax(0,1.6fr)_7rem_5rem_6rem] items-center gap-3 border-b border-border px-2 py-2', active ? 'bg-accent' : 'hover:bg-accent/50')}>
      <Icon className="size-4 text-muted-foreground" />
      <div className="min-w-0">
        <div className="truncate text-[13px] font-medium">{h.name}</div>
        <div className="truncate text-[11px] text-muted-foreground">{h.os}</div>
      </div>
      <Mark kind={st.state} label={st.label} />
      <span className="font-mono text-[12px] tabular-nums">
        {h.runners[0]}/{h.runners[1]}
      </span>
      <span className="text-[12px] text-muted-foreground">{h.heartbeat}</span>
    </div>
  )
}

function CapabilityTable({ providers }: { providers: string[] }) {
  const caps: Record<string, { iso: string; ws: string; exec: boolean; snap: boolean; limits: string; source: string }> = {
    local: { iso: 'Prozess', ws: 'host_path', exec: true, snap: false, limits: 'cpu, memory (best effort)', source: 'eingebaut' },
    docker: { iso: 'Container', ws: 'bind, volume, clone', exec: true, snap: false, limits: 'cpu, memory, pids, disk, timeout', source: 'eingebaut' },
    kubernetes: { iso: 'Pod', ws: 'volume, clone', exec: true, snap: true, limits: 'alle', source: 'eingebaut' },
    hetzner: { iso: 'VM', ws: 'clone', exec: true, snap: false, limits: 'cpu, memory', source: 'Plugin 0.3.1' },
  }
  return (
    <table className="w-full text-[12px]">
      <thead>
        <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
          <th className="py-1 pr-2 font-normal">Provider</th>
          <th className="py-1 pr-2 font-normal">Isolation</th>
          <th className="py-1 pr-2 font-normal">Workspaces</th>
          <th className="py-1 pr-2 font-normal">PTY-Exec</th>
          <th className="py-1 pr-2 font-normal">Snapshot</th>
          <th className="py-1 font-normal">Limits</th>
        </tr>
      </thead>
      <tbody>
        {providers.map((p) => {
          const c = caps[p]
          return (
            <tr key={p} className="border-b border-border">
              <td className="py-1 pr-2">
                <span className="font-mono">{p}</span> <span className="text-[11px] text-muted-foreground">{c.source}</span>
              </td>
              <td className="py-1 pr-2">{c.iso}</td>
              <td className="py-1 pr-2 font-mono text-[11px]">{c.ws}</td>
              <td className="py-1 pr-2">{c.exec ? 'ja' : '–'}</td>
              <td className="py-1 pr-2">{c.snap ? 'ja' : '–'}</td>
              <td className="py-1 text-[11px]">{c.limits}</td>
            </tr>
          )
        })}
      </tbody>
    </table>
  )
}

function HostsOverview({ state }: { state: string }) {
  if (state === 'local') {
    const h = hosts[0]
    return (
      <SettingsShell
        active="hosts"
        title="Hosts"
        description="Ein Host ist ein Rechner, auf dem Sessions laufen. Lokal ist das immer dieser Rechner – mehr brauchst du nicht."
      >
        <F id={['RUN-002', 'RUN-005']}>
          <div className="border-t border-border">
            <HostRow h={h} active />
          </div>
        </F>
        <div className="mt-5 grid grid-cols-2 gap-8">
          <F id={['RUN-001', 'PLG-001']}>
            <Section title="Provider auf diesem Rechner">
              <CapabilityTable providers={['local', 'docker']} />
              <p className="mt-2 text-[12px] text-muted-foreground">
                <C>local</C> startet pro Session einen Runner-Prozess, verbunden über einen Unix-Socket mit dem Daemon. <C>docker</C> nutzt Podman 5.2 über
                dessen Docker-API (rootless).
              </p>
            </Section>
          </F>
          <F id="RUN-005">
            <Section title="Labels">
              <div className="flex flex-wrap gap-1">
                {h.auto.map((l) => (
                  <LabelChip key={l}>{l}</LabelChip>
                ))}
              </div>
              <p className="mt-2 text-[12px] text-muted-foreground">Automatisch ermittelt. Lokal gibt es keinen Dispatch – alle Sessions laufen hier.</p>
            </Section>
          </F>
        </div>
        <div className="mt-6 border-t border-border pt-4">
          <h2 className="text-[13px] font-semibold">Weitere Rechner einbinden (optional)</h2>
          <p className="mt-1 max-w-2xl text-[13px] text-muted-foreground">
            Andere Rechner, Docker-Hosts oder ein Kubernetes-Cluster werden zu Hosts, wenn du einen eigenen beton-Server betreibst. Es gibt keinen beton-Cloud-Dienst –
            der Server läuft bei dir.
          </p>
          <div className="mt-2 flex gap-2">
            <Btn>Mit eigenem Server verbinden…</Btn>
            <Btn variant="ghost">Wie geht das?</Btn>
          </div>
        </div>
      </SettingsShell>
    )
  }

  const selected = state === 'offline' ? hosts[4] : hosts[1]
  return (
    <SettingsShell
      active="hosts"
      connection="server"
      title="Hosts"
      description={
        <>
          Rechner, die bei deinem Server <C>beton.team.example</C> angemeldet sind. Hosts verbinden sich ausgehend; Provider-Zugänge bleiben auf dem Host.
        </>
      }
      actions={<Btn variant="primary">Host hinzufügen</Btn>}
      bodyClassName="p-0"
    >
      <div className="flex h-full min-h-0">
        <div className="min-w-0 flex-1 overflow-y-auto px-6 py-4">
          <F id={['RUN-004', 'RUN-019', 'RUN-007']}>
            <div className="grid grid-cols-[1.5rem_minmax(0,1.6fr)_7rem_5rem_6rem] gap-3 border-b border-border px-2 pb-1 text-[11px] text-muted-foreground">
              <span />
              <span>Host</span>
              <span>Zustand</span>
              <span>Runner</span>
              <span>Heartbeat</span>
            </div>
            {hosts.map((h) => (
              <HostRow key={h.id} h={h} active={h.id === selected.id} />
            ))}
            {state === 'offline' && (
              <F id="DIST-018">
                <HostRow
                  h={{ ...hosts[1], id: 'old-box', name: 'old-box', os: 'Fedora 42 · amd64 · beton 1.1.0', heartbeat: '–', runners: [0, 4] }}
                  override={{ state: 'fail', label: 'abgelehnt' }}
                />
              </F>
            )}
          </F>
          {state === 'offline' && (
            <p className="mt-2 text-[12px] text-muted-foreground">
              <span className="text-deny">old-box abgelehnt:</span> beton 1.1.0 liegt außerhalb des Kompatibilitätsfensters (Server 1.3.0, erlaubt 1.2–1.4). Auf dem Host:{' '}
              <C>beton upgrade</C>.
            </p>
          )}
        </div>

        <aside className="w-[400px] shrink-0 overflow-y-auto border-l border-border px-5 py-4">
          <div className="flex items-center gap-2">
            <Server className="size-4 text-muted-foreground" />
            <h2 className="text-[15px] font-semibold">{selected.name}</h2>
            <Mark kind={selected.state} label={selected.stateLabel} className="ml-auto" />
          </div>
          {state === 'offline' ? (
            <F id="RUN-007" className="mt-3">
              <div className="rounded-md border border-border bg-card p-3 text-[13px]">
                <p className="font-semibold">Keine Verbindung seit 14:02 Uhr</p>
                <p className="mt-1 text-muted-foreground">
                  60 s ohne Antwort auf Ping – der Host gilt als verloren. Er versucht selbst, sich neu zu verbinden; nächster Versuch in 8 s (Backoff 0,5 s bis
                  30 s).
                </p>
                <p className="mt-2 text-muted-foreground">
                  Seine Runner wurden nach 5 Min. ohne Rückkehr beendet; die 2 Sessions sind gestoppt und lassen sich fortsetzen.
                </p>
                <div className="mt-2 flex gap-2">
                  <Btn>Sessions woanders fortsetzen</Btn>
                  <Btn variant="ghost">Host entfernen</Btn>
                </div>
              </div>
            </F>
          ) : (
            <F id={['RUN-004', 'RUN-006', 'RUN-007']} className="mt-3">
              <KV
                rows={[
                  ['Verbindung', <>Tunnel ausgehend · Ping 18 ms · seit 3 Tg.</>],
                  ['Heartbeat', 'alle 20 s · zuletzt vor 4 s'],
                  ['Version', <>beton 1.3.0 · Protokoll v1 <span className="text-muted-foreground">(passt zum Server)</span></>],
                  ['Dienst', <>systemd --user · startet nach Neustart</>],
                  ['Angemeldet', 'per Pairing-Code von ingo@diva-e.com'],
                  ['Runner', '1 von 8 belegt'],
                ]}
              />
            </F>
          )}
          <F id="RUN-005" className="mt-4">
            <Section title="Labels" aside="aus host.yaml · automatisch">
              <div className="flex flex-wrap gap-1">
                {[...selected.labels, ...selected.auto].map((l) => (
                  <LabelChip key={l}>{l}</LabelChip>
                ))}
              </div>
            </Section>
          </F>
          <F id={['RUN-001', 'PLG-001']}>
            <Section title="Provider & Fähigkeiten">
              <CapabilityTable providers={selected.providers} />
            </Section>
          </F>
        </aside>
      </div>

      {state === 'pair' && <PairDialog />}
    </SettingsShell>
  )
}

function PairDialog() {
  return (
    <div className="absolute inset-0 z-10 flex items-start justify-center bg-foreground/20 pt-20">
      <F id={['RUN-004', 'RUN-006']} className="w-[520px] rounded-lg border border-border bg-popover p-5 shadow-xl">
        <h2 className="type-wide text-[16px] font-[650]">Host hinzufügen</h2>
        <ol className="mt-3 flex flex-col gap-3 text-[13px]">
          <li>
            <span className="font-medium">1. Auf dem Rechner ausführen</span>
            <TermBlock className="mt-1">
              <Prompt host="ingo@build-box-03" cwd="~">
                beton host pair --server https://beton.team.example
              </Prompt>
            </TermBlock>
          </li>
          <li>
            <span className="font-medium">2. Angezeigten Code eingeben</span>
            <div className="mt-1 flex items-center gap-2">
              <span className="chamfer-sm flex h-9 w-44 items-center border-2 border-signal bg-card px-3 font-mono text-[15px] tracking-widest">
                K7QF-2M9X
                <Cursor />
              </span>
              <span className="text-[12px] text-muted-foreground">gültig noch 9:12 Min.</span>
            </div>
          </li>
          <li>
            <span className="font-medium">3. Optional: als Dienst einrichten</span>
            <p className="text-muted-foreground">
              Mit <C>beton host enable</C> startet der Host nach jedem Neustart (launchd, systemd --user oder geplante Aufgabe unter Windows).
            </p>
          </li>
        </ol>
        <div className="mt-4 flex items-center gap-2 border-t border-border pt-3">
          <Premise kind="local">Der Host öffnet keine Ports; er verbindet sich zu deinem Server.</Premise>
          <div className="ml-auto flex gap-2">
            <Btn variant="ghost">Abbrechen</Btn>
            <Btn variant="signal">Host bestätigen</Btn>
          </div>
        </div>
      </F>
    </div>
  )
}

/* ───────────────────────── Runner ───────────────────────── */

const LIFECYCLE = ['requested', 'provisioning', 'starting', 'connected', 'busy', 'idle', 'draining', 'terminated'] as const

type Runner = {
  id: string
  session: string
  title: string
  harness: 'claude' | 'codex' | 'gemini'
  host: string
  provider: string
  state: string
  mark: MarkKind
  res: string
  since: string
}

const runners: Runner[] = [
  { id: 'run_2b7x', session: 'ses_7f3k', title: 'Rate-Limiter für die Login-API', harness: 'claude', host: 'local', provider: 'local', state: 'idle', mark: 'idle', res: '–', since: '42 Min.' },
  { id: 'run_2b9c', session: 'ses_7f3m', title: 'Review: Rate-Limiter', harness: 'codex', host: 'local', provider: 'docker', state: 'busy', mark: 'busy', res: '2 CPU · 4 GiB', since: '6 Min.' },
  { id: 'run_4c1a', session: 'ses_6n1c', title: 'Nächtliches Dependency-Update', harness: 'claude', host: 'k8s-runners', provider: 'kubernetes', state: 'busy', mark: 'busy', res: '1–4 CPU · 2–8 GiB', since: '2 Std.' },
  { id: 'run_4c3e', session: 'ses_9a10', title: 'Flaky-Tests einsammeln', harness: 'codex', host: 'build-box-02', provider: 'docker', state: 'provisioning', mark: 'pending', res: '2 CPU · 4 GiB', since: '12 s' },
  { id: 'run_3f0d', session: 'ses_6p9z', title: 'Event-Log: seq lückenlos halten', harness: 'codex', host: 'build-box-01', provider: 'docker', state: 'draining', mark: 'idle', res: '2 CPU · 4 GiB', since: '1 Std.' },
]

function Lifecycle({ highlight, fail }: { highlight: string[]; fail?: 'failed' | 'lost' }) {
  return (
    <div className="flex flex-wrap items-center gap-1 text-[11px]">
      {LIFECYCLE.map((s, i) => (
        <span key={s} className="inline-flex items-center gap-1">
          <span
            className={cn(
              'rounded-sm border px-1.5 py-0.5 font-mono',
              highlight.includes(s) ? 'border-foreground bg-foreground text-background' : 'border-border text-muted-foreground',
            )}
          >
            {s}
          </span>
          {i < LIFECYCLE.length - 1 && <ArrowRight className="size-3 text-muted-foreground" />}
        </span>
      ))}
      <span className="ml-3 inline-flex items-center gap-1 text-muted-foreground">
        Abzweige:
        {['failed', 'lost', 'reconnecting'].map((s) => (
          <span
            key={s}
            className={cn(
              'rounded-sm border px-1.5 py-0.5 font-mono',
              fail === s || (fail === 'lost' && s === 'reconnecting') ? 'border-deny bg-deny-soft text-deny' : 'border-dashed border-border',
            )}
          >
            {s}
          </span>
        ))}
      </span>
    </div>
  )
}

function RunnersScreen({ state }: { state: string }) {
  const list =
    state === 'crashed'
      ? runners.map((r) => (r.id === 'run_2b9c' ? { ...r, state: 'failed', mark: 'fail' as MarkKind, since: 'vor 1 Min.' } : r))
      : state === 'reconnect'
        ? runners.map((r) => (r.id === 'run_3f0d' ? { ...r, state: 'reconnecting', mark: 'fail' as MarkKind, since: '38 s' } : r))
        : runners
  const sel = state === 'crashed' ? list[1] : state === 'reconnect' ? list[4] : state === 'snapshot' ? list[2] : list[1]
  return (
    <SettingsShell
      active="runners"
      connection="server"
      title="Runner"
      description="Ein Runner führt genau eine Session aus und meldet jeden Zustandswechsel. Untätige Runner enden nach 1 Std.; die Session bleibt fortsetzbar."
      bodyClassName="p-0"
    >
      <div className="flex h-full min-h-0">
        <div className="min-w-0 flex-1 overflow-y-auto px-6 py-4">
          <F id="RUN-003">
            <Lifecycle highlight={[sel.state]} fail={state === 'crashed' ? 'failed' : state === 'reconnect' ? 'lost' : undefined} />
          </F>
          <F id={['RUN-019', 'RUN-003']} className="mt-4">
            <table className="w-full text-[12.5px]">
              <thead>
                <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
                  <th className="py-1.5 pr-3 font-normal">Session</th>
                  <th className="py-1.5 pr-3 font-normal">Host · Provider</th>
                  <th className="py-1.5 pr-3 font-normal">Zustand</th>
                  <th className="py-1.5 pr-3 font-normal">Ressourcen</th>
                  <th className="py-1.5 font-normal">seit</th>
                </tr>
              </thead>
              <tbody>
                {list.map((r) => (
                  <tr key={r.id} className={cn('border-b border-border', r.id === sel.id && 'bg-accent')}>
                    <td className="py-1.5 pr-3">
                      <div className="font-medium">{r.title}</div>
                      <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                        <HarnessBadge id={r.harness} className="text-[11px]" />
                        <span className="font-mono">{r.id}</span>
                      </div>
                    </td>
                    <td className="py-1.5 pr-3">
                      {r.host} <span className="font-mono text-[11px] text-muted-foreground">{r.provider}</span>
                    </td>
                    <td className="py-1.5 pr-3">
                      <Mark kind={r.mark} label={<span className="font-mono">{r.state}</span>} />
                    </td>
                    <td className="py-1.5 pr-3 text-[12px]">{r.res}</td>
                    <td className="py-1.5 text-[12px] text-muted-foreground">{r.since}</td>
                  </tr>
                ))}
                {state === 'queued' && (
                    <tr className="border-b border-border bg-accent" data-features="RUN-016">
                      <td className="py-1.5 pr-3">
                        <div className="font-medium">GPU-Benchmark neu rechnen</div>
                        <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                          <HarnessBadge id="claude" className="text-[11px]" />
                          <span className="font-mono">ses_9b44</span>
                        </div>
                      </td>
                      <td className="py-1.5 pr-3 text-muted-foreground">
                        <code className="font-mono text-[11px]">gpu=true</code> – kein Host online
                      </td>
                      <td className="py-1.5 pr-3">
                        <Mark kind="pending" label={<span className="font-mono">waiting_for_runner</span>} />
                      </td>
                      <td className="py-1.5 pr-3 text-[12px]">–</td>
                      <td className="py-1.5 text-[12px] text-muted-foreground">2:20 Min.</td>
                    </tr>
                )}
              </tbody>
            </table>
          </F>
        </div>

        <aside className="w-[420px] shrink-0 overflow-y-auto border-l border-border px-5 py-4">
          {state === 'queued' ? (
            <F id="RUN-016">
              <h2 className="text-[15px] font-semibold">GPU-Benchmark neu rechnen</h2>
              <p className="mt-2 text-[13px]">
                Wartet auf einen Runner. Gesucht: <C>gpu=true</C> mit <C>beton.harness.claude</C>. Der einzige passende Host, gpu-ws, ist offline.
              </p>
              <p className="mt-1 text-[13px] text-muted-foreground">Kommt er binnen 7:40 Min. zurück, startet die Session dort; sonst schlägt der Auftrag fehl.</p>
              <div className="mt-3 flex gap-2">
                <Btn>Selector ändern</Btn>
                <Btn variant="ghost">Abbrechen</Btn>
              </div>
            </F>
          ) : (
            <>
              <div className="flex items-center gap-2">
                <h2 className="text-[15px] font-semibold">{sel.title}</h2>
              </div>
              <div className="mt-1 flex items-center gap-2 text-[12px] text-muted-foreground">
                <span className="font-mono">{sel.id}</span> · {sel.host} · <span className="font-mono">{sel.provider}</span>
              </div>

              {state === 'crashed' && (
                <F id={['RUN-003', 'RUN-017']} className="mt-3 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
                  <p className="font-semibold">Runner abgestürzt: Speicherlimit erreicht</p>
                  <p className="mt-1 text-muted-foreground">
                    Der Container wurde bei 4 GiB beendet (Ursache <C>oom</C>, Exit-Code 137). Die Session ist angehalten, der Verlauf gespeichert, der Container
                    aufgeräumt.
                  </p>
                  <div className="mt-2 flex gap-2">
                    <Btn variant="primary">Mit 8 GiB fortsetzen</Btn>
                    <Btn>Logs ansehen</Btn>
                  </div>
                </F>
              )}
              {state === 'reconnect' && (
                <F id={['RUN-007', 'RUN-003']} className="mt-3 rounded-md border border-border bg-card p-3 text-[13px]">
                  <p className="font-semibold">Verbindung unterbrochen – Runner verbindet sich neu</p>
                  <p className="mt-1 text-muted-foreground">
                    build-box-01 antwortet seit 38 s nicht. Der Runner arbeitet weiter und puffert 37 Events ab seq 1204; nach dem Wiederverbinden gehen sie ohne
                    Lücke weiter.
                  </p>
                  <p className="mt-1 text-muted-foreground">Nächster Versuch in 4 s. Kommt er binnen 4:22 Min. nicht zurück, wird der Runner beendet.</p>
                </F>
              )}
              {state === 'snapshot' && (
                <F id="RUN-018" className="mt-3 rounded-md border border-border bg-card p-3 text-[13px]">
                  <p className="font-semibold">Workspace gesichert</p>
                  <p className="mt-1 text-muted-foreground">
                    Snapshot <C>snap_6n1c_03</C> · 1,2 GiB · VolumeSnapshot in <C>beton-runners</C>. Ein Fork kann mit genau diesem Arbeitsstand starten.
                  </p>
                  <div className="mt-2 flex gap-2">
                    <Btn variant="primary">Fork mit Arbeitsstand</Btn>
                  </div>
                </F>
              )}

              <F id={['RUN-017', 'RUN-010']} className="mt-4">
                <Section title="Umgebung">
                  <KV
                    rows={[
                      ['Workspace', sel.provider === 'local' ? <>host_path · <C>~/code/shop-frontend</C></> : <>clone · Volume <C>beton-ws-{sel.session}</C></>],
                      ['Limits', sel.res === '–' ? 'keine (lokal, best effort)' : `${sel.res} · pids 1024 · timeout 8 Std.`],
                      ['Leerlauf-Ende', 'nach 1 Std.'],
                      ['Image', sel.provider === 'local' ? '–' : <C>beton-runner:1.3.0</C>],
                    ]}
                  />
                </Section>
              </F>

              <F id="RUN-019">
                <Section title="Logs" aside={<span>beton runners logs {sel.id}</span>}>
                  <TermBlock>
                    {state === 'crashed' ? (
                      <>
                        <L tone="dim">14:21:07 runner connected seq=1</L>
                        <L tone="dim">14:27:44 tool.started shell "pnpm vitest run --coverage"</L>
                        <L tone="deny">14:28:10 container killed: OOMKilled (memory 4Gi)</L>
                        <L tone="deny">14:28:10 runner failed cause=oom exit=137</L>
                      </>
                    ) : (
                      <>
                        <L tone="dim">14:21:07 provisioned {sel.provider === 'kubernetes' ? 'pod beton-run-4c1a' : 'container beton-run-2b9c'}</L>
                        <L tone="dim">14:21:09 runner connected seq=1 token=bt_run_••••</L>
                        <L tone="dim">14:21:10 harness started codex app-server</L>
                        <L>14:27:02 turn.started t_04</L>
                      </>
                    )}
                  </TermBlock>
                </Section>
              </F>

              <div className="mt-1 flex gap-2">
                <F id="RUN-018" as="span">
                  <Btn className={cn(sel.provider !== 'kubernetes' && 'pointer-events-none opacity-40')}>Workspace sichern</Btn>
                </F>
                <Btn variant="danger">Runner stoppen</Btn>
              </div>
              {sel.provider !== 'kubernetes' && (
                <p className="mt-1 text-[11px] text-muted-foreground">Sichern geht nur bei Providern mit Snapshot-Fähigkeit (hier: kubernetes).</p>
              )}
            </>
          )}
        </aside>
      </div>
    </SettingsShell>
  )
}

/* ───────────────────────── Provider & Workspaces ───────────────────────── */

function Field({ label, value, hint, mono = true }: { label: string; value: string; hint?: string; mono?: boolean }) {
  return (
    <label className="grid grid-cols-[11rem_1fr] items-start gap-3 py-1.5 text-[13px]">
      <span className="pt-1 text-muted-foreground">{label}</span>
      <span>
        <span className={cn('flex h-7 items-center rounded-md border border-input bg-card px-2', mono && 'font-mono text-[12px]')}>{value}</span>
        {hint && <span className="mt-0.5 block text-[11px] text-muted-foreground">{hint}</span>}
      </span>
    </label>
  )
}

const workspaceModes: { id: string; title: string; desc: string; on: string[] }[] = [
  { id: 'host_path', title: 'Projektordner', desc: 'Direkt im Projektverzeichnis oder einem Worktree auf dem Host.', on: ['local'] },
  { id: 'bind', title: 'Host-Pfad einbinden', desc: 'Ordner des Hosts in den Container gemountet. Remote nur mit remote_paths.', on: ['docker'] },
  { id: 'volume', title: 'Volume je Session', desc: 'Benanntes Volume bzw. PVC, bleibt über Neustarts erhalten.', on: ['docker', 'remote', 'kubernetes'] },
  { id: 'clone', title: 'Repo klonen', desc: 'Der Runner klont das Repository vor dem Start in die Umgebung.', on: ['docker', 'remote', 'kubernetes'] },
]

function ProvidersScreen({ state }: { state: string }) {
  const prov = state === 'kubernetes' ? 'kubernetes' : state === 'remote' ? 'remote' : 'docker'
  const selectedMode = state === 'docker' ? 'bind' : 'clone'
  return (
    <SettingsShell
      active="providers"
      connection={state === 'docker' ? 'local' : 'server'}
      title="Provider & Workspaces"
      description="Wo und wie isoliert ein Runner startet. Provider-Zugänge (Docker-Socket, Kubernetes-Konto) bleiben auf dem Host und erreichen nie den Server."
      actions={<Btn variant="primary">Speichern</Btn>}
    >
      <div className="mb-4 flex gap-1 border-b border-border text-[13px]">
        {[
          ['docker', 'Docker / Podman lokal', Container],
          ['remote', 'Docker remote', Server],
          ['kubernetes', 'Kubernetes', Ship],
        ].map(([id, label, Icon]) => {
          const I = Icon as typeof Server
          return (
            <span
              key={id as string}
              className={cn('-mb-px inline-flex items-center gap-1.5 border-b-2 px-2 pb-2', id === prov ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground')}
            >
              <I className="size-3.5" />
              {label as string}
            </span>
          )
        })}
        <span className="-mb-px ml-auto px-2 pb-2 text-[12px] text-muted-foreground">Host: {prov === 'kubernetes' ? 'k8s-runners' : prov === 'remote' ? 'build-box-02' : 'dieser Rechner'}</span>
      </div>

      <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-10">
        <div>
          {prov === 'docker' && (
            <F id={['RUN-008', 'PLG-001']}>
              <Section title="Container-Engine" aside={<Mark kind="ok" label="Podman 5.2 erkannt (rootless)" />}>
                <Field label="Endpunkt" value="unix:///run/user/501/podman/podman.sock" hint="Docker-kompatible API; Docker Desktop oder Colima gehen genauso." />
                <Field label="Runner-Image" value="ghcr.io/ifahrentholz/beton-runner:1.3.0" hint="Lokal vorhanden · siehe Runner-Image" />
                <Field label="Verbindung zum Daemon" value="/run/beton/daemon.sock (eingebunden)" hint="Kein Netzwerk zwischen Container und Host nötig." />
              </Section>
            </F>
          )}
          {prov === 'remote' && (
            <F id="RUN-009">
              <Section title="Entfernter Docker-Daemon" aside={<Mark kind="ok" label="erreichbar · Docker 28.1" />}>
                <Field label="Endpunkt" value="ssh://ci@docker-02.lan" hint="Oder tcp://docker-02:2376 mit mTLS-Zertifikaten." />
                <Field label="Runner verbinden sich mit" value="https://beton.team.example" hint="Direkt zum Server, mit Binding-Token für genau eine Session." />
                <Field label="remote_paths" value="– (bind deaktiviert)" hint="Bind-Mounts beziehen sich auf die Platte des Remote-Hosts; deshalb Standard: clone." />
              </Section>
            </F>
          )}
          {prov === 'kubernetes' && (
            <F id={['RUN-012', 'PLG-001']}>
              <Section title="Cluster" aside={<Mark kind="ok" label="ServiceAccount beton-host · RBAC nur Namespace" />}>
                <Field label="Namespace" value="beton-runners" />
                <Field label="Art" value="job  (backoffLimit 0, TTL 1 Std.)" hint="Empfohlen für Async-Agents; pod für interaktive Sessions." />
                <Field label="StorageClass" value="encrypted-ssd" />
                <Field label="Node-Selector" value="kubernetes.io/arch=amd64" />
                <Field label="Egress" value="NetworkPolicy: nur Server + Egress-Proxy" mono={false} />
              </Section>
            </F>
          )}

          <F id="RUN-017">
            <Section title="Ressourcen-Limits je Runner">
              <div className="grid grid-cols-[6rem_1fr_9rem] items-center gap-x-3 gap-y-1.5 text-[13px]">
                {[
                  ['cpu', prov === 'kubernetes' ? '1 → 4' : '2', 'durchgesetzt'],
                  ['memory', prov === 'kubernetes' ? '2Gi → 8Gi' : '4Gi', 'durchgesetzt'],
                  ['pids', '1024', 'durchgesetzt'],
                  ['disk', '20Gi', prov === 'docker' ? 'nicht durchsetzbar' : 'durchgesetzt'],
                  ['timeout', '8h', 'durchgesetzt'],
                  ['idle_timeout', '1h', 'durchgesetzt'],
                ].map(([k, v, s]) => (
                  <div key={k} className="contents">
                    <code className="font-mono text-[12px]">{k}</code>
                    <span className="flex h-7 items-center rounded-md border border-input bg-card px-2 font-mono text-[12px]">{v}</span>
                    <Mark kind={s === 'durchgesetzt' ? 'ok' : 'warn'} label={s} />
                  </div>
                ))}
              </div>
              {prov === 'docker' && (
                <p className="mt-2 text-[12px] text-muted-foreground">
                  Podman rootless kann <C>disk</C> mit diesem Storage-Treiber nicht begrenzen. Sessions, die es verlangen, starten mit Warnung – oder gar nicht mit{' '}
                  <C>resources.strict: true</C>.
                </p>
              )}
            </Section>
          </F>
        </div>

        <div>
          <F id="RUN-010">
            <Section title="Workspace">
              <div role="radiogroup" className="flex flex-col">
                {workspaceModes.map((m) => {
                  const avail = m.on.includes(prov)
                  const on = m.id === selectedMode
                  return (
                    <div key={m.id} className={cn('flex gap-3 border-b border-border py-2', !avail && 'opacity-45')}>
                      <span className={cn('mt-0.5 size-4 shrink-0 rounded-full border', on ? 'border-[5px] border-foreground' : 'border-input')} />
                      <div>
                        <div className="text-[13px] font-medium">
                          {m.title} <code className="ml-1 font-mono text-[11px] text-muted-foreground">{m.id}</code>
                        </div>
                        <div className="text-[12px] text-muted-foreground">{avail ? m.desc : 'Bei diesem Provider nicht verfügbar.'}</div>
                      </div>
                    </div>
                  )
                })}
              </div>
              <div className="mt-2 flex items-center gap-3 text-[13px]">
                <span className="text-muted-foreground">Nach Session-Ende</span>
                <span className="rounded-md border border-input bg-card px-2 py-0.5">7 Tage behalten</span>
                <span className="text-muted-foreground">oder sofort löschen</span>
              </div>
            </Section>
          </F>
          {selectedMode === 'clone' && (
            <F id="RUN-011">
              <Section title="Repository klonen">
                <Field label="URL" value="https://github.com/acme/shop-frontend.git" />
                <Field label="Ref · Tiefe" value="main · depth 50 · filter blob:none" />
                <Field label="Submodule" value="nein" mono={false} />
                <p className="mt-1 text-[12px] text-muted-foreground">
                  Der Git-Zugang wird beim Klonen vom Credential-Proxy eingesetzt und landet nie in der Umgebung. Danach legt beton den Session-Branch an.
                </p>
              </Section>
            </F>
          )}
        </div>
      </div>
    </SettingsShell>
  )
}

/* ───────────────────────── Runner-Image ───────────────────────── */

function ImageScreen({ state }: { state: string }) {
  const opts = [
    { id: 'pull', title: 'Offizielles Image laden', sub: 'ghcr.io/ifahrentholz/beton-runner:1.3.0 · amd64 + arm64 · 1,1 GB', net: true },
    { id: 'build', title: 'Lokal bauen', sub: 'Aus deploy/runner-image mit den gepinnten CLI-Versionen; eigene CLIs per EXTRA_CLIS.', net: false },
    { id: 'file', title: 'Aus Datei laden', sub: 'Ein mit docker save exportiertes Image, z. B. von einem anderen Rechner.', net: false },
  ]
  return (
    <SettingsShell
      active="image"
      title="Runner-Image"
      description="Container-Runner starten aus diesem Image. Es enthält beton, die offiziellen CLIs claude, codex und gemini, git und ripgrep und läuft ohne Root (UID 10001)."
    >
      <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-10">
        <F id={['RUN-014', 'DIST-010']}>
          <Section title="Herkunft">
            {opts.map((o) => (
              <div key={o.id} className="flex gap-3 border-b border-border py-2.5">
                <span className={cn('mt-0.5 size-4 shrink-0 rounded-full border', o.id === state ? 'border-[5px] border-foreground' : 'border-input')} />
                <div className="min-w-0">
                  <div className="text-[13px] font-medium">{o.title}</div>
                  <div className="text-[12px] text-muted-foreground">{o.sub}</div>
                  <div className="mt-1">
                    <Premise kind={o.net ? 'network' : 'local'}>{o.net ? 'lädt von ghcr.io, wenn du auf „Laden“ klickst' : 'kein Download durch beton'}</Premise>
                  </div>
                </div>
              </div>
            ))}
            <div className="mt-3 flex gap-2">
              {state === 'pull' && <Btn variant="primary">Laden und prüfen</Btn>}
              {state === 'build' && <Btn variant="primary">Image bauen</Btn>}
              {state === 'file' && <Btn variant="primary">Datei wählen…</Btn>}
              <Btn variant="ghost">Variante -slim (ohne CLIs)</Btn>
            </div>
          </Section>
        </F>

        <div>
          {state === 'pull' && (
            <F id={['RUN-014', 'DIST-010']}>
              <Section title="Prüfung nach dem Laden">
                <TermBlock>
                  <L>
                    <S tone="ok">✓</S> Signatur (cosign keyless) · …/beton/.github/workflows/release.yml@refs/tags/v1.3.0
                  </L>
                  <L>
                    <S tone="ok">✓</S> SBOM (CycloneDX) angehängt · 412 Pakete
                  </L>
                  <L>
                    <S tone="ok">✓</S> Wöchentlicher Rebuild 1.3.0-r4 vom 29.09. (Basis-Updates)
                  </L>
                  <L>
                    <S tone="ok">✓</S> läuft als UID 10001
                  </L>
                </TermBlock>
              </Section>
            </F>
          )}
          {state === 'build' && (
            <F id="RUN-014">
              <Section title="Bauen" aside="dauert ca. 4 Min.">
                <TermBlock>
                  <Prompt cwd="~/Develop/ai/beton">{'docker build deploy/runner-image -t beton-runner:1.3.0-local \\'}</Prompt>
                  <L>{'    --build-arg EXTRA_CLIS="@qwen-code/qwen-code"'}</L>
                  <L tone="dim">[3/9] RUN npm install -g @anthropic-ai/claude-code@2.3.1 @openai/codex@0.61.0 @google/gemini-cli@0.14.2</L>
                  <L tone="dim">[4/9] RUN npm install -g @qwen-code/qwen-code</L>
                  <L>
                    [6/9] RUN beton doctor --json <S tone="dim">…</S> <Cursor />
                  </L>
                </TermBlock>
              </Section>
            </F>
          )}
          {state === 'file' && (
            <F id="RUN-014">
              <Section title="Aus Datei">
                <TermBlock>
                  <Prompt>docker load -i ~/Transfer/beton-runner-1.3.0-arm64.tar</Prompt>
                  <L>Loaded image: ghcr.io/ifahrentholz/beton-runner:1.3.0</L>
                  <L>
                    <S tone="ok">✓</S> Signatur aus beton-runner-1.3.0.sigstore.json offline geprüft
                  </L>
                </TermBlock>
              </Section>
            </F>
          )}
          <F id="RUN-014">
            <Section title="Inhalt" aside={<C>deploy/runner-image/versions.toml</C>}>
              <table className="w-full text-[12.5px]">
                <tbody>
                  {[
                    ['beton', '1.3.0'],
                    ['Node.js', '22 LTS'],
                    ['claude', '2.3.1'],
                    ['codex', '0.61.0'],
                    ['gemini', '0.14.2'],
                    ['git · ripgrep · tini', '2.51 · 15.0 · 0.19'],
                  ].map(([k, v]) => (
                    <tr key={k} className="border-b border-border">
                      <td className="py-1 pr-3 font-mono">{k}</td>
                      <td className="py-1 font-mono text-muted-foreground">{v}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <p className="mt-2 text-[12px] text-muted-foreground">Keine Anmeldedaten im Image. Die CLIs meldest du pro Host einmal an (CLI-Login im Container).</p>
            </Section>
          </F>
        </div>
      </div>
    </SettingsShell>
  )
}

/* ───────────────────────── CLI-Login im Container ───────────────────────── */

function LoginScreen({ state }: { state: string }) {
  const rows: { host: string; harness: 'claude' | 'codex' | 'gemini'; status: MarkKind; label: string; store: string }[] = [
    { host: 'dieser Rechner (docker)', harness: 'claude', status: 'ok', label: 'angemeldet · Claude Max', store: 'verschlüsseltes Archiv, Schlüssel im macOS-Schlüsselbund' },
    { host: 'dieser Rechner (docker)', harness: 'codex', status: state === 'device' ? 'wait' : 'off', label: state === 'device' ? 'Anmeldung läuft' : 'nicht angemeldet', store: '–' },
    { host: 'build-box-01', harness: 'claude', status: 'ok', label: 'angemeldet · Claude Max', store: 'verschlüsseltes Archiv, Schlüssel im Host-Keyring' },
    { host: 'k8s-runners', harness: 'claude', status: state === 'k8s' ? 'fail' : 'ok', label: state === 'k8s' ? 'abgelehnt' : 'angemeldet · Claude Max', store: state === 'k8s' ? 'keine verschlüsselnde StorageClass' : 'PVC auf encrypted-ssd' },
  ]
  return (
    <SettingsShell
      active="login"
      connection="server"
      title="CLI-Login im Container"
      description="Damit Container-Runner deine Subscription nutzen, meldest du die offizielle CLI einmal pro Host selbst an – per Device-Flow im eigenen Browser. beton liest die Tokens nie."
    >
      <F id="RUN-015">
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
              <th className="py-1.5 pr-3 font-normal">Host</th>
              <th className="py-1.5 pr-3 font-normal">CLI</th>
              <th className="py-1.5 pr-3 font-normal">Status</th>
              <th className="py-1.5 pr-3 font-normal">Ablage</th>
              <th className="py-1.5 font-normal" />
            </tr>
          </thead>
          <tbody>
            {rows.map((r, i) => (
              <tr key={i} className="border-b border-border">
                <td className="py-1.5 pr-3">{r.host}</td>
                <td className="py-1.5 pr-3">
                  <HarnessBadge id={r.harness} />
                </td>
                <td className="py-1.5 pr-3">
                  <Mark kind={r.status} label={r.label} />
                </td>
                <td className="py-1.5 pr-3 text-[12px] text-muted-foreground">{r.store}</td>
                <td className="py-1.5 text-right">
                  {r.status === 'off' && <Btn variant="primary">Anmelden</Btn>}
                  {r.status === 'ok' && <Btn variant="ghost">Abmelden</Btn>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </F>

      {state === 'device' && (
        <F id="RUN-015" className="mt-5 max-w-3xl">
          <div className="mb-1.5 flex items-center gap-2 text-[12px] text-muted-foreground">
            <span>Interaktive Sitzung im Container</span>
            <span className="rounded-sm border border-border px-1">wird nicht aufgezeichnet</span>
          </div>
          <TermBlock title="beton runner login --harness codex --host local">
            <L tone="dim">Starte codex login --device-auth in beton-run-login-7a1 (PTY) …</L>
            <L>Sign in with ChatGPT using device code</L>
            <Ask title="Im eigenen Browser bestätigen" question="Warte auf Bestätigung …" choice="Ctrl+C bricht ab">
              <L>
                {'  '}1. Öffne <S tone="bold">https://auth.openai.com/codex/device</S>
              </L>
              <L>
                {'  '}2. Gib den Code ein: <span className="bg-signal px-1 font-semibold text-signal-foreground">QX7R-K2PD</span>
              </L>
            </Ask>
          </TermBlock>
          <p className="mt-2 text-[12px] text-muted-foreground">
            Die Anmeldung landet in <C>~/.codex</C> auf einem Volume nur für dich und diesen Host, verschlüsselt mit AES-256-GCM. Der Schlüssel verlässt den Host nie;
            nichts davon geht an den Server, in Logs oder in Diagnose-Bundles.
          </p>
        </F>
      )}
      {state === 'k8s' && (
        <F id="RUN-015" className="mt-5 max-w-3xl rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
          <p className="font-semibold">Anmeldung auf k8s-runners nicht möglich</p>
          <p className="mt-1 text-muted-foreground">
            Für Anmeldedaten verlangt beton eine verschlüsselnde StorageClass, sonst lägen deine Tokens im Klartext auf dem Cluster. Trage sie in der Host-Konfiguration
            ein:
          </p>
          <TermBlock className="mt-2">
            <L>providers:</L>
            <L>{'  - id: k8s'}</L>
            <L>
              {'    '}credential_storage_class: <S tone="ok">encrypted-ssd</S>
            </L>
          </TermBlock>
        </F>
      )}
      {state === 'overview' && (
        <div className="mt-5 flex max-w-3xl flex-col gap-1.5">
          <Premise kind="local">Kein API-Schlüssel nötig: Die Container nutzen dieselbe Subscription wie deine lokale CLI.</Premise>
          <Premise kind="local">Die Anmeldedaten von User A werden nie in Runner von User B eingebunden.</Premise>
          <p className="text-[12px] text-muted-foreground">
            Im Terminal: <C>beton runner login --harness codex --host build-box-01</C>
          </p>
        </div>
      )}
    </SettingsShell>
  )
}

/* ───────────────────────── Labels & Dispatch ───────────────────────── */

function DispatchScreen({ state }: { state: string }) {
  const selector = state === 'error' ? 'os=linux,repo in (beton,web,!gpu' : state === 'nomatch' ? 'os=linux,gpu=true' : 'os=linux,repo=beton,!gpu'
  const cands: { h: string; load: [number, number]; ok: boolean; why: string; pick?: boolean }[] =
    state === 'nomatch'
      ? [
          { h: 'gpu-ws', load: [0, 2], ok: false, why: 'offline' },
          { h: 'build-box-01', load: [1, 8], ok: false, why: 'gpu fehlt' },
          { h: 'k8s-runners', load: [3, 20], ok: false, why: 'gpu fehlt' },
        ]
      : [
          { h: 'build-box-01', load: [1, 8], ok: true, why: 'passt · geringste Auslastung', pick: true },
          { h: 'build-box-02', load: [6, 8], ok: true, why: 'passt' },
          { h: 'k8s-runners', load: [3, 20], ok: false, why: 'repo fehlt' },
          { h: 'gpu-ws', load: [0, 2], ok: false, why: 'offline · gpu=true' },
          { h: 'dieser Rechner', load: [2, 8], ok: false, why: 'os=macos' },
        ]
  return (
    <SettingsShell
      active="dispatch"
      connection="server"
      title="Labels & Dispatch"
      description="Sessions, Zeitpläne und Async-Agents wählen ihren Host über einen Selector. Unter den passenden Hosts gewinnt der mit der geringsten Auslastung."
    >
      <F id="RUN-016" className="max-w-3xl">
        <div className="grid grid-cols-[7rem_1fr] items-center gap-3 text-[13px]">
          <span className="text-muted-foreground">Selector</span>
          <span
            className={cn(
              'flex h-8 items-center rounded-md border bg-card px-2 font-mono text-[12.5px]',
              state === 'error' ? 'border-deny' : 'border-input outline-2 outline-ring',
            )}
          >
            {selector}
          </span>
          <span className="text-muted-foreground">Provider</span>
          <span className="flex h-8 w-40 items-center rounded-md border border-input bg-card px-2 font-mono text-[12.5px]">docker</span>
          <span className="text-muted-foreground">Harness</span>
          <span>
            <HarnessBadge id="codex" /> <span className="text-[12px] text-muted-foreground">→ verlangt beton.harness.codex</span>
          </span>
        </div>
        {state === 'error' && (
          <div className="mt-2 ml-[7.75rem] font-mono text-[12px]">
            <div className="text-deny">{' '.repeat(26)}^</div>
            <p className="font-sans text-deny">Zeichen 27: „!“ ist in einer Liste nicht erlaubt; schließe die Liste mit „)“ und setze „!gpu“ als eigenen Term.</p>
          </div>
        )}
        <p className="mt-2 ml-[7.75rem] text-[12px] text-muted-foreground">
          Terme mit Komma = und: <C>key=value</C> <C>key!=value</C> <C>key in (a,b)</C> <C>key notin (a,b)</C> <C>key</C> <C>!key</C>
        </p>
      </F>

      {state !== 'error' && (
        <F id={['RUN-016', 'RUN-005']} className="mt-5 max-w-3xl">
          <Section title="Passende Hosts" aside={state === 'nomatch' ? 'keiner online' : '2 von 5'}>
            {cands.map((c) => (
              <div key={c.h} className={cn('grid grid-cols-[1.25rem_10rem_8rem_1fr] items-center gap-3 border-b border-border py-1.5 text-[13px]', c.pick && 'bg-accent')}>
                <span>{c.ok ? <span className="text-ok">✓</span> : <span className="text-muted-foreground">–</span>}</span>
                <span className={cn(c.pick && 'font-semibold')}>{c.h}</span>
                <span className="flex items-center gap-2">
                  <span className="h-1.5 w-14 overflow-hidden rounded-full bg-muted">
                    <span className="block h-full bg-foreground/70" style={{ width: `${(c.load[0] / c.load[1]) * 100}%` }} />
                  </span>
                  <span className="font-mono text-[11px]">
                    {c.load[0]}/{c.load[1]}
                  </span>
                </span>
                <span className="text-[12px] text-muted-foreground">{c.pick ? <span className="text-foreground">Wird gewählt – {c.why}</span> : c.why}</span>
              </div>
            ))}
          </Section>
          {state === 'nomatch' && (
            <p className="text-[13px]">
              Kein passender Host online. Der Auftrag wartet als <C>waiting_for_runner</C> bis zu 10 Min.; kommt ein passender Host online, startet er dort.
            </p>
          )}
        </F>
      )}
    </SettingsShell>
  )
}

/* ───────────────────────── Reaper ───────────────────────── */

function ReaperScreen({ state }: { state: string }) {
  const dry = state === 'dry-run'
  const items: [string, string, string, string][] = [
    ['Container', 'beton-run-4a2f', 'build-box-02', 'Runner dem Server unbekannt'],
    ['Pod', 'beton-run-3k1q', 'k8s-runners', '42 Min. lost (Grenze 30 Min.)'],
    ['Volume', 'beton-ws-ses_5x8d', 'build-box-01', 'Aufbewahrung (7 Tage) abgelaufen'],
    ['PVC', 'beton-ws-ses_5v2a', 'k8s-runners', 'Session beendet, retain: delete'],
  ]
  return (
    <SettingsShell
      active="reaper"
      connection="server"
      title="Aufräumen"
      description="Jeder Host gleicht alle 10 Min. seine Container, Pods und Volumes mit dem Server ab und entfernt, was zu keinem lebenden Runner mehr gehört."
      actions={dry ? <Btn variant="primary">Jetzt aufräumen</Btn> : <Btn>Vorschau</Btn>}
    >
      <F id="RUN-013" className="max-w-4xl">
        <div className="mb-3 flex items-center gap-4 text-[13px]">
          {dry ? <Mark kind="pending" label="Vorschau – noch nichts entfernt" /> : <Mark kind="ok" label="Letzter Lauf vor 3 Min. · 4 entfernt" />}
          <span className="text-muted-foreground">Intervall 10 Min. · Gnadenfrist für verlorene Runner 30 Min.</span>
        </div>
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
              <th className="py-1.5 pr-3 font-normal">Art</th>
              <th className="py-1.5 pr-3 font-normal">Name</th>
              <th className="py-1.5 pr-3 font-normal">Host</th>
              <th className="py-1.5 pr-3 font-normal">Grund</th>
              <th className="py-1.5 font-normal">{dry ? 'würde' : 'Ergebnis'}</th>
            </tr>
          </thead>
          <tbody>
            {items.map(([k, n, h, why]) => (
              <tr key={n} className="border-b border-border">
                <td className="py-1.5 pr-3">{k}</td>
                <td className="py-1.5 pr-3 font-mono text-[12px]">{n}</td>
                <td className="py-1.5 pr-3">{h}</td>
                <td className="py-1.5 pr-3 text-muted-foreground">{why}</td>
                <td className="py-1.5">{dry ? <span className="text-muted-foreground">{k === 'Volume' || k === 'PVC' ? 'gelöscht' : 'beendet'}</span> : k === 'Volume' || k === 'PVC' ? 'gelöscht' : 'beendet'}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="mt-3 flex flex-col gap-1">
          <Premise kind="local">Ressourcen ohne beton-Labels werden nie angefasst – auch nicht, wenn sie im selben Namespace liegen.</Premise>
          <p className="text-[12px] text-muted-foreground">
            Auch beim Entfernen eines Runner-Plugins bleiben dessen Ressourcen stehen, bis der Reaper sie über <C>list_managed</C> findet.
          </p>
        </div>
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Gruppe ───────────────────────── */

export const group: ScreenGroup = {
  id: 'hosts',
  title: 'Hosts & Runner',
  order: 160,
  screens: [
    {
      id: 'hosts-overview',
      title: 'Hosts',
      description:
        'Lokal ist dieser Rechner der einzige Host. Mit eigenem Server kommen weitere Rechner, Docker-Hosts und Cluster dazu – mit Labels, Heartbeat, Fähigkeiten der Provider und Pairing per Code.',
      features: ['RUN-001', 'RUN-002', 'RUN-004', 'RUN-005', 'RUN-006', 'RUN-007', 'RUN-019', 'PLG-001', 'DIST-018'],
      states: [
        { id: 'local', title: 'Nur dieser Rechner' },
        { id: 'team', title: 'Mehrere Hosts' },
        { id: 'offline', title: 'Host offline' },
        { id: 'pair', title: 'Host hinzufügen' },
      ],
      component: HostsOverview,
    },
    {
      id: 'hosts-runners',
      title: 'Runner',
      description: 'Alle Runner mit ihrem Platz im Lebenszyklus, Umgebung, Limits und Logs. Abstürze, Wiederverbinden, wartende Aufträge und Snapshots als Zustände.',
      features: ['RUN-003', 'RUN-007', 'RUN-010', 'RUN-016', 'RUN-017', 'RUN-018', 'RUN-019'],
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'crashed', title: 'Runner abgestürzt' },
        { id: 'reconnect', title: 'Verbindet neu' },
        { id: 'queued', title: 'Wartet auf Runner' },
        { id: 'snapshot', title: 'Snapshot' },
      ],
      component: RunnersScreen,
    },
    {
      id: 'hosts-providers',
      title: 'Provider & Workspaces',
      description: 'Docker/Podman lokal, entfernter Docker-Daemon und Kubernetes: Endpunkt, Workspace-Strategie, Repo-Clone und Ressourcen-Limits – mit Hinweis, was ein Provider nicht durchsetzen kann.',
      features: ['RUN-008', 'RUN-009', 'RUN-010', 'RUN-011', 'RUN-012', 'RUN-017', 'PLG-001'],
      states: [
        { id: 'docker', title: 'Docker / Podman' },
        { id: 'remote', title: 'Docker remote' },
        { id: 'kubernetes', title: 'Kubernetes' },
      ],
      component: ProvidersScreen,
    },
    {
      id: 'hosts-image',
      title: 'Runner-Image',
      description: 'Das Image für Container-Runner: offiziell laden (signiert, mit SBOM), lokal bauen oder aus einer Datei einspielen. Inhalt und gepinnte CLI-Versionen.',
      features: ['RUN-014', 'DIST-010'],
      states: [
        { id: 'pull', title: 'Offiziell laden' },
        { id: 'build', title: 'Lokal bauen' },
        { id: 'file', title: 'Aus Datei' },
      ],
      component: ImageScreen,
    },
    {
      id: 'hosts-login',
      title: 'CLI-Login im Container',
      description: 'Subscription auch in Containern: Die offizielle CLI wird einmal pro Host per Device-Flow angemeldet, die Anmeldung liegt verschlüsselt auf einem Volume nur für dich.',
      features: ['RUN-015'],
      states: [
        { id: 'overview', title: 'Übersicht' },
        { id: 'device', title: 'Device-Flow' },
        { id: 'k8s', title: 'StorageClass fehlt' },
      ],
      component: LoginScreen,
    },
    {
      id: 'hosts-dispatch',
      title: 'Labels & Dispatch',
      description: 'Selector eingeben und sofort sehen, welche Hosts passen und welcher gewählt wird; ohne Treffer wartet der Auftrag, Syntaxfehler zeigen die Stelle.',
      features: ['RUN-016', 'RUN-005'],
      states: [
        { id: 'match', title: 'Treffer' },
        { id: 'nomatch', title: 'Kein Host' },
        { id: 'error', title: 'Syntaxfehler' },
      ],
      component: DispatchScreen,
    },
    {
      id: 'hosts-reaper',
      title: 'Aufräumen (Reaper)',
      description: 'Verwaiste Container, Pods und Volumes, die der Reaper gefunden und entfernt hat – oder in der Vorschau entfernen würde.',
      features: ['RUN-013'],
      states: [
        { id: 'default', title: 'Letzter Lauf' },
        { id: 'dry-run', title: 'Vorschau' },
      ],
      component: ReaperScreen,
    },
  ],
}
