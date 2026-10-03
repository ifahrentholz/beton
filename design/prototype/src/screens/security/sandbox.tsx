import type { ReactNode } from 'react'
import { ArrowRight, FileCode2, Lock, Plus } from 'lucide-react'
import { HarnessBadge } from '@/app/harness'
import { TerminalOutput } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, C, Callout, KV, Mark, PageHead, Premise, Scroll, Section, SettingsLayout, Tag } from '../policies/kit'

/* ------------------------------------------------------------------ Sandbox-Einstellungen */

type Os = 'macos' | 'linux' | 'linux-bwrap' | 'windows'

const backendInfo: Record<Os, { os: string; backend: string; detail: string; caps: [string, 'yes' | 'no' | 'partial', string?][]; beta?: boolean; feature: string[] }> = {
  macos: {
    os: 'macOS 26.0 · arm64',
    backend: 'Seatbelt (sandbox-exec)',
    detail: 'Profil pro Start aus Vorlage erzeugt, deny-default; Netz nur zu localhost:<Proxy-Port>.',
    caps: [['Dateien lesen/schreiben', 'yes'], ['Masken', 'yes'], ['Netz nur über Proxy', 'yes'], ['Environment', 'yes'], ['Prozesse', 'yes'], ['Ressourcen', 'partial', 'rlimits, ohne cgroups']],
    feature: ['SBX-007', 'SBX-001'],
  },
  linux: {
    os: 'Ubuntu 24.04 · Kernel 6.12 · x86_64',
    backend: 'Landlock + seccomp + Namespaces',
    detail: 'Nativ in beton, ohne externes Programm. Landlock-ABI 6 (inkl. TCP-Regeln und Signal-Scoping).',
    caps: [['Dateien lesen/schreiben', 'yes'], ['Masken', 'yes', 'Mount-Namespace'], ['Netz nur über Proxy', 'yes', 'leerer Net-NS + Relay'], ['Environment', 'yes'], ['Syscalls', 'yes', 'seccomp'], ['Ressourcen', 'yes', 'cgroup v2 über systemd']],
    feature: ['SBX-008', 'SBX-001', 'SBX-013'],
  },
  'linux-bwrap': {
    os: 'Ubuntu 24.04 · Kernel 6.8 · AppArmor sperrt User-Namespaces',
    backend: 'bubblewrap 0.11 (Ausweichlösung)',
    detail: 'Unprivilegierte User-Namespaces sind gesperrt. beton nutzt bwrap für Namespaces und wendet Landlock und seccomp danach selbst an.',
    caps: [['Dateien lesen/schreiben', 'yes'], ['Masken', 'yes'], ['Netz nur über Proxy', 'yes'], ['Environment', 'yes', 'nie über --setenv'], ['Syscalls', 'yes'], ['Ressourcen', 'partial', 'ohne cgroup-Delegation']],
    feature: ['SBX-009', 'SBX-008'],
  },
  windows: {
    os: 'Windows 11 24H2 · x64',
    backend: 'Windows (Beta): Restricted Token, Job Object, AppContainer',
    detail: 'Bestmögliche Isolation ohne Admin-Rechte. Für volle Isolation: Docker/Podman oder WSL2.',
    caps: [['Dateien lesen/schreiben', 'partial', 'Stufe 1 liest das Benutzerprofil'], ['Masken', 'no', 'nicht in freigegebenen Ordnern'], ['Netz nur über Proxy', 'no', 'Stufe 2 ohne Netz, Stufe 1 frei'], ['Environment', 'yes'], ['Prozesse', 'yes', 'Job Object'], ['Ressourcen', 'yes']],
    beta: true,
    feature: ['SBX-016'],
  },
}

function PathList({ items, add }: { items: { p: string; from?: string; err?: ReactNode }[]; add: string }) {
  return (
    <div className="space-y-1">
      {items.map((i) => (
        <div key={i.p}>
          <div className={cn('flex items-center gap-2 rounded-md border px-2 py-1 font-mono text-[12px]', i.err ? 'border-deny bg-deny-soft' : 'border-border bg-card')}>
            <span className="flex-1">{i.p}</span>
            {i.from && <Tag mono={false}>{i.from}</Tag>}
          </div>
          {i.err && <div className="mt-0.5 text-[12px] text-deny">{i.err}</div>}
        </div>
      ))}
      <button className="inline-flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground">
        <Plus className="size-3.5" /> {add}
      </button>
    </div>
  )
}

export function SandboxSettings({ state }: { state: string }) {
  const os: Os = state === 'config-error' ? 'macos' : (state as Os)
  const b = backendInfo[os]
  const err = state === 'config-error'
  return (
    <SettingsLayout active="sandbox">
      <PageHead
        title="Sandbox"
        sub="Jeder Harness und jedes Tool läuft isoliert. Gilt für alle Sessions; Projekte und Agents können nur verschärfen."
        actions={
          <Btn>
            <FileCode2 className="size-3.5" /> Als YAML bearbeiten
          </Btn>
        }
      >
        <div className="mt-2">
          <Premise kind="lock">~/.beton/config.yaml · ergänzt durch .beton/config.yaml im Projekt und sandbox: im Agent</Premise>
        </div>
      </PageHead>
      <Scroll className="px-6 py-4">
        <div className="max-w-4xl space-y-5">
          {err && (
            <F id="SBX-003">
              <Callout tone="deny" title="Sandbox-Konfiguration ungültig – neue Sessions starten nicht">
                <p>
                  <C>write_paths: ["~"]</C> würde den ganzen Home-Ordner beschreibbar machen. Erlaubt sind nur konkrete Ordner, z. B.{' '}
                  <C>~/.cache/sccache</C>. Laufende Sessions behalten ihre bisherige Sandbox.
                </p>
              </Callout>
            </F>
          )}
          <F id={[...b.feature, 'SBX-001', 'SBX-010']}>
            <Section title="Backend" hint={b.os} actions={<Btn tone="quiet">Erneut prüfen</Btn>}>
              <div className="flex gap-6">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="rounded-md border border-input bg-card px-2 py-1 text-[13px]">automatisch</span>
                    <ArrowRight className="size-3.5 text-muted-foreground" />
                    <span className="text-[14px] font-semibold">{b.backend}</span>
                    {b.beta && <Tag mono={false}>Beta</Tag>}
                  </div>
                  <p className="mt-1.5 text-[12px] text-muted-foreground">{b.detail}</p>
                  {os === 'windows' && (
                    <Callout className="mt-2" title="Nicht alles ist isoliert">
                      <p>
                        Netz in Stufe 1 läuft nicht über den Proxy. Stufe 2 hat kein Netz; mit <C>allow_unenforced_network</C> bekäme sie ungeprüftes Netz – das
                        ist aus. Sessions mit <C>allow_network: true</C> starten deshalb nicht.
                      </p>
                    </Callout>
                  )}
                  <F id="SBX-015" className="mt-2">
                    <p className="text-[12px] text-muted-foreground">
                      Weitere Backends: Docker/Podman (rootless bevorzugt, ab M5) – zwei Container je Session, Harness und Tools getrennt, ohne Docker-Socket.
                    </p>
                  </F>
                </div>
                <div className="w-64 shrink-0 space-y-1 border-l border-border pl-4">
                  {b.caps.map(([c, v, note]) => (
                    <div key={c}>
                      <Mark v={v}>
                        {c}
                        {note && <span className="text-muted-foreground"> · {note}</span>}
                      </Mark>
                    </div>
                  ))}
                </div>
              </div>
            </Section>
          </F>

          <F id="SBX-012">
            <Section title="Voreinstellung" hint="Eigene Werte ergänzen oder überschreiben die Voreinstellung">
              <div className="grid grid-cols-3 gap-0 overflow-hidden rounded-md border border-border">
                {[
                  { id: 'default', t: 'Standard', d: 'Workspace schreibbar, kein Netz außer den Anbieter-Hosts der CLI' },
                  { id: 'dev', t: 'Entwicklung', d: 'Zusätzlich lesend: npm, PyPI, crates.io, Go-Proxy, GitHub-Downloads und deren Caches' },
                  { id: 'readonly', t: 'Nur lesen', d: 'Workspace nur lesbar, kein Netz – für Reviews und Side-Chats' },
                ].map((p) => (
                  <div key={p.id} className={cn('border-r border-border p-2.5 last:border-r-0', p.id === 'dev' ? 'bg-accent' : 'bg-card')}>
                    <div className="flex items-center gap-2 text-[13px] font-medium">
                      <span className={cn('size-3 rounded-full border-2', p.id === 'dev' ? 'border-foreground bg-foreground' : 'border-muted-foreground')} />
                      {p.t} <span className="font-mono text-[11px] text-muted-foreground">{p.id}</span>
                    </div>
                    <p className="mt-1 text-[12px] text-muted-foreground">{p.d}</p>
                  </div>
                ))}
              </div>
            </Section>
          </F>

          <div className="grid grid-cols-2 gap-6">
            <F id={['SBX-003', 'SBX-012']}>
              <Section title="Dateien" hint="Workspace bzw. Worktree ist immer lesbar und schreibbar">
                <div className="space-y-3">
                  <div>
                    <div className="mb-1 text-[12px] text-muted-foreground">Zusätzlich lesen</div>
                    <PathList add="Ordner zum Lesen freigeben" items={[{ p: '~/.cargo/registry', from: 'dev' }, { p: '~/.npm/_cacache', from: 'dev' }, { p: '/opt/homebrew' }]} />
                  </div>
                  <div>
                    <div className="mb-1 text-[12px] text-muted-foreground">Zusätzlich schreiben</div>
                    <PathList
                      add="Ordner zum Schreiben freigeben"
                      items={[
                        { p: '~/.cache/sccache' },
                        ...(err ? [{ p: '~', err: <>Nicht erlaubt: das ganze Home-Verzeichnis. Verboten sind auch /, ~/.beton, ~/.ssh, das Docker-Socket, /proc, /sys.</> }] : []),
                      ]}
                    />
                  </div>
                  <p className="text-[12px] text-muted-foreground">
                    Nur <C>~</C> wird erweitert, keine <C>$VARIABLEN</C>. Symlinks werden aufgelöst; zeigt einer in einen gesperrten Ordner, ist das ein Fehler.
                    <C>.git/hooks</C> bleibt schreibgeschützt.
                  </p>
                </div>
              </Section>
            </F>
            <F id="SBX-004">
              <Section title="Masken" hint="Unsichtbar, auch innerhalb freigegebener Ordner">
                <div className="flex flex-wrap gap-1">
                  {['~/.ssh', '~/.aws', '~/.azure', '~/.config/gcloud', '~/.config/gh', '~/.netrc', '~/.git-credentials', '~/.docker/config.json', '~/.kube', '~/.gnupg', '~/.npmrc', '~/.cargo/credentials*', '~/.beton', '~/Library/Keychains'].map((m) => (
                    <span key={m} className="inline-flex items-center gap-1 rounded-sm bg-sunken px-1.5 py-0.5 font-mono text-[11px]">
                      <Lock className="size-2.5 text-muted-foreground" />
                      {m}
                    </span>
                  ))}
                </div>
                <div className="mt-2 text-[12px] text-muted-foreground">Im Workspace zusätzlich maskiert:</div>
                <div className="mt-1 flex flex-wrap gap-1">
                  {['.env', '.env.*', 'secrets/'].map((m) => (
                    <span key={m} className="rounded-sm border border-border bg-card px-1.5 py-0.5 font-mono text-[11px]">
                      {m}
                    </span>
                  ))}
                </div>
                <div className="mt-2 text-[12px] text-muted-foreground">Sichtbar trotz Punkt am Anfang (allow_hidden):</div>
                <div className="mt-1 flex flex-wrap gap-1">
                  {['.venv', '.cargo', '.github'].map((m) => (
                    <span key={m} className="rounded-sm border border-border bg-card px-1.5 py-0.5 font-mono text-[11px]">
                      {m}
                    </span>
                  ))}
                </div>
                <p className="mt-2 text-[12px] text-muted-foreground">In Stufe 2 außerdem: die Anmeldedateien und State-Ordner aller Harness-CLIs.</p>
              </Section>
            </F>
          </div>

          <div className="grid grid-cols-2 gap-6">
            <F id="SBX-005">
              <Section title="Environment" hint="Nichts wird geerbt">
                <KV k="Immer gesetzt">
                  <span className="font-mono text-[11px]">PATH HOME USER SHELL TERM LANG LC_* TZ TMPDIR BETON_SESSION_ID BETON_STAGE</span>
                </KV>
                <KV k="Durchreichen">
                  <span className="font-mono text-[11px]">RUST_LOG NODE_OPTIONS</span>
                </KV>
                <KV k="Fest setzen">
                  <span className="font-mono text-[11px]">CI=1</span>
                </KV>
                <KV k="Credentials">
                  <span className="font-mono text-[11px]">GH_TOKEN=bt_cred_…</span> <span className="text-[11px] text-muted-foreground">nur Platzhalter</span>
                </KV>
                <KV k="Immer entfernt">
                  <span className="font-mono text-[11px] text-muted-foreground">SSH_AUTH_SOCK DBUS_SESSION_BUS_ADDRESS BETON_TOKEN LD_PRELOAD DYLD_*</span>
                </KV>
              </Section>
            </F>
            <F id={['SBX-013', 'SBX-006']}>
              <Section title="Grenzen" hint="Pro Session, für den ganzen Prozessbaum">
                <KV k="Speicher">8 192 MB</KV>
                <KV k="Prozesse">1 024</KV>
                <KV k="CPU-Zeit">unbegrenzt</KV>
                <KV k="Beim Session-Ende">alle Prozesse beenden</KV>
                <div className="mt-3 border-t border-border pt-2">
                  <div className="flex items-center gap-2 text-[13px] font-medium">Ohne Sandbox starten</div>
                  <p className="mt-0.5 text-[12px] text-muted-foreground">
                    Nur mit <C>backend: none</C> und Bestätigung pro Session; per Policy verbietbar. YOLO-Mode startet nie ohne Sandbox. Fehlt eine
                    geforderte Fähigkeit, startet die Session nicht – kein stiller Rückfall.
                  </p>
                </div>
              </Section>
            </F>
          </div>
        </div>
      </Scroll>
    </SettingsLayout>
  )
}

/* ------------------------------------------------------------------ Zwei Stufen */

type Cell = ['yes' | 'no' | 'partial', string?]
const matrix: { res: string; s0: Cell; s1: Cell; s2: Cell }[] = [
  { res: 'Workspace lesen', s0: ['yes'], s1: ['yes'], s2: ['yes'] },
  { res: 'Workspace schreiben', s0: ['yes'], s1: ['yes', 'Edit-Tools der CLI'], s2: ['yes', 'außer Preset „Nur lesen“'] },
  { res: '.git/hooks, .git/config', s0: ['yes'], s1: ['partial', 'nur lesen'], s2: ['partial', 'nur lesen'] },
  { res: 'Eigene Anmeldung der CLI', s0: ['no', 'beton liest sie nie'], s1: ['yes', 'nur die deklarierten Dateien'], s2: ['no', 'maskiert'] },
  { res: 'Schlüsselbund', s0: ['yes', 'für beton-Secrets'], s1: ['partial', 'nur wenn der Adapter es braucht'], s2: ['no'] },
  { res: '~/.ssh, ~/.aws, ~/.config/gh …', s0: ['no'], s1: ['no'], s2: ['no'] },
  { res: 'SSH-Agent, Docker-Socket', s0: ['yes'], s1: ['no'], s2: ['no'] },
  { res: 'Netzwerk', s0: ['yes', 'als Proxy-Upstream'], s1: ['partial', 'nur Proxy; Anbieter-Hosts als Tunnel'], s2: ['partial', 'nur Proxy, Regeln, entschlüsselt'] },
  { res: 'Environment', s0: ['yes'], s1: ['partial', 'Basis + Variablen der CLI'], s2: ['partial', 'Basis + Platzhalter'] },
  { res: 'Andere Prozesse', s0: ['yes'], s1: ['no'], s2: ['no'] },
]

const adapters = [
  {
    h: 'claude' as const,
    creds: '~/.claude/.credentials.json · Schlüsselbund „Claude Code-credentials“',
    hosts: 'api.anthropic.com, claude.ai, console.anthropic.com, statsig.anthropic.com',
    iso: 'Bash-Tool über Shell-Präfix bt-exec',
  },
  { h: 'codex' as const, creds: '~/.codex/auth.json', hosts: 'api.openai.com, chatgpt.com, auth.openai.com', iso: 'bt-exec als bash/sh vorn im PATH' },
  { h: 'gemini' as const, creds: '~/.gemini/oauth_creds.json', hosts: 'generativelanguage.googleapis.com, oauth2.googleapis.com', iso: 'ACP: beton führt Tools selbst aus' },
]

export function SandboxStages({ state }: { state: string }) {
  const inherited = state === 'inherited'
  return (
    <SettingsLayout active="stages">
      <PageHead title="Stufen & Harnesses" sub="Die CLI eines Anbieters und die Tools, die sie startet, laufen in zwei getrennten Sandboxes.">
        <div className="mt-2 flex flex-wrap gap-x-5">
          <Premise kind="lock">Deine Subscription bleibt in der CLI: Stufe 1 darf ihre eigene Anmeldung lesen, beton und die Tools in Stufe 2 nicht.</Premise>
        </div>
      </PageHead>
      <Scroll className="px-6 py-4">
        <div className="max-w-5xl space-y-5">
          <F id="SBX-002">
            <div className="flex items-stretch gap-2 font-mono text-[12px]">
              {[
                { t: 'Stufe 1 · Harness', b: 'claude', s: 'Bash("pnpm test")' },
                { t: '', b: 'bt-exec', s: 'Unix-Socket, Token' },
                { t: 'Stufe 0 · beton', b: 'Exec-Broker', s: 'prüft cwd und Env' },
                { t: 'Stufe 2 · Tools', b: 'sh -c "pnpm test"', s: 'frische Sandbox je Aufruf' },
              ].map((n, i) => (
                <div key={i} className="flex items-center gap-2">
                  {i > 0 && <ArrowRight className="size-4 shrink-0 text-muted-foreground" />}
                  <div className={cn('rounded-md border px-3 py-2', i === 2 ? 'border-foreground/40 bg-card' : 'border-border bg-card')}>
                    <div className="font-sans text-[11px] text-muted-foreground">{n.t || 'Shim in Stufe 1'}</div>
                    <div className="font-semibold">{n.b}</div>
                    <div className="font-sans text-[11px] text-muted-foreground">{n.s}</div>
                  </div>
                </div>
              ))}
            </div>
            <p className="mt-2 text-[12px] text-muted-foreground">
              Keine verschachtelten Sandboxes: Tools erben die Rechte der CLI nicht. Exit-Code und Signale (Strg+C) laufen durch.
            </p>
          </F>

          <F id={['SBX-002', 'SBX-004', 'PRX-005']}>
            <div className="overflow-hidden rounded-md border border-border bg-card">
              <div className="grid grid-cols-[220px_repeat(3,minmax(0,1fr))] border-b border-border bg-sunken text-[12px]">
                <span className="px-3 py-2 text-muted-foreground">Was darf …</span>
                <span className="border-l border-border px-3 py-2 font-medium">Stufe 0 · beton selbst</span>
                <span className="border-l border-border px-3 py-2 font-medium">Stufe 1 · Harness-Prozess</span>
                <span className="border-l border-border px-3 py-2 font-medium">Stufe 2 · Tools</span>
              </div>
              {matrix.map((r) => (
                <div key={r.res} className="grid grid-cols-[220px_repeat(3,minmax(0,1fr))] border-b border-border/70 text-[12px] last:border-b-0">
                  <span className="px-3 py-1.5">{r.res}</span>
                  {[r.s0, r.s1, r.s2].map(([v, t], i) => (
                    <span key={i} className="border-l border-border px-3 py-1.5">
                      <Mark v={v}>{t ?? (v === 'yes' ? 'ja' : v === 'no' ? 'nein' : 'teilweise')}</Mark>
                    </span>
                  ))}
                </div>
              ))}
            </div>
          </F>

          <F id={['SBX-002', 'PRX-005', 'SBX-017']}>
            <Section title="Was die Adapter für Stufe 1 anmelden" hint="Nur diese Pfade und Hosts erhält die CLI">
              <div className="divide-y divide-border">
                {adapters.map((a) => (
                  <div key={a.h} className="grid grid-cols-[160px_1fr_1fr_220px] gap-4 py-2 text-[12px]">
                    <HarnessBadge id={a.h} />
                    <div>
                      <div className="text-muted-foreground">Anmeldung lesen</div>
                      <div className="font-mono text-[11px]">{a.creds}</div>
                    </div>
                    <div>
                      <div className="text-muted-foreground">Anbieter-Hosts (Tunnel, nicht entschlüsselt)</div>
                      <div className="font-mono text-[11px]">{a.hosts}</div>
                    </div>
                    <div>
                      <div className="text-muted-foreground">Tools in Stufe 2</div>
                      <div>{a.iso}</div>
                    </div>
                  </div>
                ))}
                {inherited && (
                  <div className="grid grid-cols-[160px_1fr] gap-4 py-2 text-[12px]">
                    <span className="font-medium">Goose (ACP-Bridge)</span>
                    <Callout tone="deny" title="Kann Tools nicht in Stufe 2 schicken">
                      <p>
                        Der Adapter meldet <C>tool_isolation: inherited</C>: Tools liefen mit den Rechten der CLI. Die Session startet nur, wenn du für dieses
                        Projekt <C>require_tool_isolation: false</C> setzt – sie ist dann als „eingeschränkt“ markiert.
                      </p>
                    </Callout>
                  </div>
                )}
              </div>
              <p className="mt-2 text-[12px] text-muted-foreground">
                Auch im Original-TUI der Anbieter (PTY-Modus) läuft die CLI in Stufe 1; Shell-Befehle und das Terminal der Session laufen in Stufe 2.
              </p>
            </Section>
          </F>
        </div>
      </Scroll>
    </SettingsLayout>
  )
}

/* ------------------------------------------------------------------ Probe & Escape-Suite */

const providers = [
  { id: 'landlock', ok: true, caps: 'fs_read fs_write masks net env proc syscall resources', note: 'Kernel 6.12 · Landlock-ABI 6 · User-NS erlaubt' },
  { id: 'bwrap', ok: true, caps: 'wie landlock', note: 'bubblewrap 0.11 · nur als Ausweichlösung' },
  { id: 'docker', ok: false, caps: '–', note: 'Docker-Daemon nicht erreichbar (/var/run/docker.sock)' },
  { id: 'podman', ok: true, caps: 'fs_read fs_write masks net env proc resources', note: 'rootless 5.2 · ab M5' },
  { id: 'seatbelt', ok: false, caps: '–', note: 'nur macOS' },
  { id: 'windows', ok: false, caps: '–', note: 'nur Windows' },
]

const escapeCases = [
  'Lesen ~/.ssh/id_ed25519',
  'Lesen ~/.aws/credentials',
  'Anmeldung der CLI aus Stufe 2 lesen',
  'Schreiben nach /etc',
  'Schreiben außerhalb des Workspace',
  'Schreiben in .git/hooks',
  'Symlink im Workspace auf ~/.ssh',
  '/proc/<runner>/environ lesen',
  'Secrets im Environment',
  'curl https://evil.test (keine Regel)',
  'curl http://169.254.169.254/',
  'Direktverbindung ins LAN ohne Proxy',
  'DNS-Rebinding auf 127.0.0.1',
  'Platzhalter an fremden Host',
  'Schlüsselbund/Secret Service aus Stufe 2',
  'ptrace/kill auf den Runner',
  'Fork-Bombe (Limit greift)',
  'Docker-Socket',
  'mount/unshare',
]

export function SandboxProbe({ state }: { state: string }) {
  const suite = state === 'escape-suite'
  return (
    <SettingsLayout active="sandbox">
      <PageHead
        title={suite ? 'Ausbruchstests auf diesem Rechner' : 'Was kann dieser Rechner isolieren?'}
        sub={suite ? 'Dieselbe Testsuite wie in CI, mit Test-Secrets in einem gefälschten Home-Ordner. Jeder Angriff muss scheitern.' : 'Ergebnis der Fähigkeitsprüfung aller Backends. Auch in beton doctor enthalten.'}
        actions={<Btn>{suite ? 'Erneut ausführen' : 'Ausbruchstests ausführen'}</Btn>}
      />
      <Scroll className="px-6 py-4">
        {suite ? (
          <F id={['SBX-011', 'SBX-010']} className="max-w-3xl">
            <div className="mb-2 flex items-center gap-3 text-[13px]">
              <span className="font-semibold">19 von 19 abgewehrt</span>
              <span className="text-muted-foreground">Stufe 1 und 2 · landlock · 14,2 s</span>
              <span className="ml-auto font-mono text-[11px] text-muted-foreground">beton sandbox test</span>
            </div>
            <div className="columns-2 gap-6 rounded-md border border-border bg-card p-3">
              {escapeCases.map((c) => (
                <div key={c} className="flex items-center gap-2 py-0.5 text-[12px]">
                  <Lock className="size-3 text-ok" aria-label="abgewehrt" />
                  <span>{c}</span>
                </div>
              ))}
            </div>
            <p className="mt-2 text-[12px] text-muted-foreground">Ein neues Backend gilt erst als verfügbar, wenn es diese Suite besteht.</p>
          </F>
        ) : (
          <F id={['SBX-010', 'SBX-001', 'SBX-008', 'SBX-009', 'SBX-015']} className="max-w-4xl">
            <div className="mb-2 text-[13px]">
              Gewählt: <b>landlock</b> <span className="text-muted-foreground">(automatisch auf Linux)</span>
            </div>
            <div className="overflow-hidden rounded-md border border-border bg-card">
              {providers.map((p) => (
                <div key={p.id} className="grid grid-cols-[110px_120px_minmax(0,1fr)_minmax(0,1fr)] gap-3 border-b border-border px-3 py-2 text-[12px] last:border-b-0">
                  <span className="font-mono font-medium">{p.id}</span>
                  <Mark v={p.ok ? 'yes' : 'no'}>{p.ok ? 'verfügbar' : 'nicht verfügbar'}</Mark>
                  <span className="font-mono text-[11px] text-muted-foreground">{p.caps}</span>
                  <span className="text-muted-foreground">{p.note}</span>
                </div>
              ))}
            </div>
            <div className="mt-4 grid grid-cols-2 gap-6">
              <div>
                <div className="mb-1 text-[13px] font-semibold">Befehl in einer Stufe ausprobieren</div>
                <TerminalOutput>{`$ beton sandbox exec --stage tools -- cat ~/.ssh/config
cat: /home/ingo/.ssh/config: No such file or directory
beton: ~/.ssh ist in Stufe 2 maskiert (Default-Maske). Exit 1`}</TerminalOutput>
              </div>
              <div>
                <div className="mb-1 text-[13px] font-semibold">Aufgelöste Sandbox (Stufe 2, shop-frontend)</div>
                <div className="rounded-md border border-border bg-card p-2 font-mono text-[11px] leading-relaxed">
                  <div>read: ~/code/shop-frontend (Workspace) · ~/.npm/_cacache [dev] · /opt/homebrew</div>
                  <div>write: ~/code/shop-frontend · ~/.cache/sccache · $TMPDIR (Scratch)</div>
                  <div>masks: 17 Default · .env .env.* secrets/ [Projekt]</div>
                  <div>env: 14 Namen (ohne Werte) · net: Proxy 127.0.0.1:53124</div>
                </div>
              </div>
            </div>
          </F>
        )}
      </Scroll>
    </SettingsLayout>
  )
}
