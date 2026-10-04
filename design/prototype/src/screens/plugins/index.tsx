import { FolderOpen, GitBranch, Package, Search } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { Blank, Comment, Cursor, ExitCode, L, Prompt, S, Term, TermBlock } from '@/app/kit/term'
import { Btn, C, KV, Mark, Premise, Section, SettingsShell, type MarkKind } from '@/app/kit/hosts'

/* ───────────────────────── Beispieldaten ───────────────────────── */

type Kind = 'harness' | 'runner_provider' | 'git_provider'
type Source = 'eingebaut' | 'Registry' | 'Git' | 'lokaler Pfad'
type Status = 'ok' | 'disabled' | 'incompatible' | 'crashlooping' | 'unsigned'

type Plugin = {
  name: string
  desc: string
  kind: Kind
  source: Source
  sourceRef?: string
  version: string
  signed: 'ja' | 'nein' | '–'
  status: Status
}

const builtins: Plugin[] = [
  { name: 'beton-harness-claude', desc: 'Claude Code über stream-json', kind: 'harness', source: 'eingebaut', version: '1.3.0', signed: '–', status: 'ok' },
  { name: 'beton-harness-codex', desc: 'Codex über app-server', kind: 'harness', source: 'eingebaut', version: '1.3.0', signed: '–', status: 'ok' },
  { name: 'beton-harness-acp', desc: 'Gemini CLI und andere ACP-Agents', kind: 'harness', source: 'eingebaut', version: '1.3.0', signed: '–', status: 'ok' },
  { name: 'beton-harness-direct', desc: 'Direkte Modell-APIs, z. B. Ollama lokal', kind: 'harness', source: 'eingebaut', version: '1.3.0', signed: '–', status: 'ok' },
  { name: 'provider-docker', desc: 'Container über Docker oder Podman', kind: 'runner_provider', source: 'eingebaut', version: '1.3.0', signed: '–', status: 'ok' },
  { name: 'provider-kubernetes', desc: 'Pods und Jobs in einem Cluster', kind: 'runner_provider', source: 'eingebaut', version: '1.3.0', signed: '–', status: 'ok' },
]

const installed: Plugin[] = [
  { name: 'beton-runner-hetzner', desc: 'Runner auf Hetzner-Cloud-VMs', kind: 'runner_provider', source: 'Registry', sourceRef: 'beton-plugins', version: '0.3.1', signed: 'ja', status: 'ok' },
  { name: 'acme-git-gitea', desc: 'Gitea und Forgejo als Git-Provider', kind: 'git_provider', source: 'Git', sourceRef: 'codeberg.org/acme/beton-git-gitea#v0.2.0', version: '0.2.0', signed: 'nein', status: 'unsigned' },
  { name: 'echo-harness', desc: 'Beispiel-Harness aus dem SDK', kind: 'harness', source: 'lokaler Pfad', sourceRef: '~/code/echo-harness', version: '0.1.0-dev', signed: 'nein', status: 'unsigned' },
  { name: 'beton-runner-proxmox', desc: 'Runner als Proxmox-LXC', kind: 'runner_provider', source: 'Registry', sourceRef: 'beton-plugins', version: '0.9.2', signed: 'ja', status: 'incompatible' },
  { name: 'beton-harness-aider', desc: 'Aider im PTY', kind: 'harness', source: 'Registry', sourceRef: 'beton-plugins', version: '0.5.0', signed: 'ja', status: 'disabled' },
]

const statusMark: Record<Status, { kind: MarkKind; label: string }> = {
  ok: { kind: 'ok', label: 'ok' },
  disabled: { kind: 'off', label: 'deaktiviert' },
  incompatible: { kind: 'fail', label: 'inkompatibel' },
  crashlooping: { kind: 'fail', label: 'stürzt wiederholt ab' },
  unsigned: { kind: 'warn', label: 'ok · unsigniert' },
}

const kindLabel: Record<Kind, string> = { harness: 'Harness', runner_provider: 'Runner-Provider', git_provider: 'Git-Provider' }
const sourceIcon: Record<Source, typeof Package> = { eingebaut: Package, Registry: Search, Git: GitBranch, 'lokaler Pfad': FolderOpen }

function PluginTable({ rows, selected }: { rows: Plugin[]; selected?: string }) {
  return (
    <table className="w-full text-[12.5px]">
      <thead>
        <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
          <th className="py-1.5 pr-3 font-normal">Name</th>
          <th className="py-1.5 pr-3 font-normal">Art</th>
          <th className="py-1.5 pr-3 font-normal">Quelle</th>
          <th className="py-1.5 pr-3 font-normal">Version</th>
          <th className="py-1.5 font-normal">Status</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((p) => {
          const I = sourceIcon[p.source]
          const st = statusMark[p.status]
          return (
            <tr key={p.name} className={cn('border-b border-border', p.name === selected && 'bg-accent', p.status === 'disabled' && 'text-muted-foreground')}>
              <td className="py-1.5 pr-3">
                <div className="font-medium">{p.name}</div>
                <div className="text-[11px] text-muted-foreground">{p.desc}</div>
              </td>
              <td className="py-1.5 pr-3">{kindLabel[p.kind]}</td>
              <td className="py-1.5 pr-3">
                <span className="inline-flex items-center gap-1.5">
                  <I className="size-3.5 text-muted-foreground" />
                  {p.source}
                </span>
              </td>
              <td className="py-1.5 pr-3 font-mono text-[12px]">{p.version}</td>
              <td className="py-1.5">
                <Mark kind={st.kind} label={st.label} />
              </td>
            </tr>
          )
        })}
      </tbody>
    </table>
  )
}

function Permissions({ diff }: { diff?: boolean }) {
  const rows: [string, string, string, boolean?][] = [
    ['network', 'api.hetzner.cloud:443', 'nur über den Egress-Proxy, sonst kein Netz'],
    ...(diff ? ([['network', 'objects.hetzner.cloud:443', 'neu', true]] as [string, string, string, boolean][]) : []),
    ['fs_read', '~/.config/hcloud', 'nur dieser Ordner ist lesbar'],
    ...(diff ? ([['fs_write', '~/.cache/beton-hetzner', 'neu', true]] as [string, string, string, boolean][]) : []),
    ['secrets', 'hetzner/api-token', 'nur als Platzhalter bt_cred_…, der Proxy setzt ihn ein'],
    ['exec', 'ssh', 'darf genau dieses Programm starten'],
    ['env', '–', 'keine Umgebungsvariablen'],
  ]
  return (
    <table className="w-full text-[12.5px]">
      <tbody>
        {rows.map(([k, v, note, isNew], i) => (
          <tr key={i} className={cn('border-b border-border/70', isNew && 'font-semibold')}>
            <td className="w-20 py-1 pr-3 font-mono text-[12px]">
              {isNew ? '+ ' : ''}
              {k}
            </td>
            <td className="py-1 pr-3 font-mono text-[12px]">{v}</td>
            <td className={cn('py-1 text-[12px]', isNew ? 'text-foreground' : 'text-muted-foreground')}>{note}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

/* ───────────────────────── Plugin-Liste ───────────────────────── */

function PluginsList({ state }: { state: string }) {
  const rows =
    state === 'crashloop' ? installed.map((p) => (p.name === 'beton-runner-hetzner' ? { ...p, status: 'crashlooping' as Status } : p)) : installed
  const sel = state === 'incompatible' ? rows[3] : rows[0]
  return (
    <SettingsShell
      active="plugins"
      title="Plugins"
      description="Plugins fügen Harnesses, Runner-Provider und Git-Provider hinzu. Sie laufen als eigene Prozesse in der Sandbox – mit genau den Rechten, die du gewährt hast."
      actions={<Btn variant="primary">Plugin installieren…</Btn>}
      bodyClassName="p-0"
    >
      <div className="flex h-full min-h-0">
        <div className="min-w-0 flex-1 overflow-y-auto px-6 py-4">
          {state === 'empty' ? (
            <F id="PLG-005" className="concrete-grain mb-5 rounded-md border border-dashed border-border px-6 py-8 text-center">
              <p className="type-wide text-[15px] font-[650]">Noch keine Plugins</p>
              <p className="mx-auto mt-1 max-w-md text-[13px] text-muted-foreground">
                Installiere aus einem lokalen Ordner, einer Git-URL oder – optional – aus der Plugin-Registry. Ein lokaler Pfad geht immer, auch ohne Netz.
              </p>
              <div className="mt-3 flex justify-center gap-2">
                <Btn variant="primary">Aus Ordner installieren…</Btn>
                <Btn>Git-URL…</Btn>
              </div>
            </F>
          ) : (
            <F id={['PLG-011', 'PLG-003', 'PLG-010']}>
              <Section title="Installiert" aside={`${rows.length} Plugins · ~/.beton/plugins`}>
                <PluginTable rows={rows} selected={sel.name} />
              </Section>
            </F>
          )}
          <F id="PLG-001">
            <Section title="Eingebaut" aside="Teil von beton 1.3.0 · dieselben Schnittstellen wie Plugins">
              <PluginTable rows={builtins} />
            </Section>
          </F>
        </div>

        {state !== 'empty' && (
          <aside className="w-[420px] shrink-0 overflow-y-auto border-l border-border px-5 py-4">
            <div className="flex items-baseline gap-2">
              <h2 className="text-[15px] font-semibold">{sel.name}</h2>
              <span className="font-mono text-[12px] text-muted-foreground">{sel.version}</span>
            </div>
            <p className="text-[12px] text-muted-foreground">{sel.desc}</p>

            {state === 'crashloop' && (
              <F id="PLG-003" className="mt-3 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
                <p className="font-semibold">Stürzt wiederholt ab – angehalten</p>
                <p className="mt-1 text-muted-foreground">
                  5 Abstürze in 10 Min. beton startet das Plugin nicht mehr neu; Sessions auf Hetzner-Runnern bekommen sofort einen Fehler statt zu warten.
                </p>
                <TermBlock className="mt-2" title="stderr (letzte Zeilen)">
                  <L tone="dim">14:31:02 initialize ok, plugin_api 1</L>
                  <L tone="deny">14:31:03 error: HCLOUD_TOKEN not set (secret hetzner/api-token fehlt)</L>
                  <L tone="dim">14:31:03 exit 1 · Neustart in 16 s</L>
                </TermBlock>
                <p className="mt-2">Ursache: Das Secret <C>hetzner/api-token</C> ist nicht angelegt.</p>
                <div className="mt-2 flex gap-2">
                  <Btn variant="primary">Secret anlegen</Btn>
                  <Btn>Erneut starten</Btn>
                </div>
              </F>
            )}
            {state === 'incompatible' && (
              <F id="PLG-010" className="mt-3 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
                <p className="font-semibold">Passt nicht zu dieser beton-Version</p>
                <p className="mt-1 text-muted-foreground">
                  0.9.2 spricht <C>plugin_api 2</C> und verlangt <C>beton &gt;=1.4</C>. Installiert ist beton 1.3.0 mit plugin_api 1. Das Plugin wird nicht gestartet.
                </p>
                <div className="mt-2 flex gap-2">
                  <Btn variant="primary">0.8.4 installieren (kompatibel)</Btn>
                  <Btn>beton aktualisieren…</Btn>
                </div>
              </F>
            )}

            <F id="PLG-004" className="mt-4">
              <Section title="Manifest" aside={<C>beton-plugin.toml</C>}>
                <KV
                  rows={[
                    ['Art', kindLabel[sel.kind]],
                    ['Quelle', <>{sel.source} · <span className="font-mono text-[12px]">{sel.sourceRef}</span></>],
                    ['Protokoll', <>JSON-RPC 2.0 über stdio · <C>plugin_api {sel.status === 'incompatible' ? 2 : 1}</C></>],
                    ['Kompatibel mit', <C>{sel.status === 'incompatible' ? 'beton >=1.4, <2.0' : 'beton >=1.0, <2.0'}</C>],
                    ['Signatur', sel.signed === 'ja' ? <Mark kind="ok" label="gültig · github.com/acme/beton-runner-hetzner" /> : <Mark kind="warn" label="unsigniert" />],
                    ['Prüfsumme', <Mark kind="ok" label="sha256 stimmt (aarch64-apple-darwin)" />],
                    ['Lizenz', 'Apache-2.0'],
                  ]}
                />
              </Section>
            </F>
            <F id={['PLG-008', 'PLG-009']}>
              <Section title="Gewährte Berechtigungen" aside="am 28.09. bestätigt">
                <Permissions />
                <p className="mt-2 text-[12px] text-muted-foreground">
                  Durchgesetzt von der Sandbox: nur diese Pfade, Netz nur über den Egress-Proxy. Ohne verfügbare Sandbox startet das Plugin nicht.
                </p>
              </Section>
            </F>
            <F id="PLG-003">
              <Section title="Prozess">
                <KV
                  rows={[
                    ['Start', 'bei Bedarf (zuletzt vor 12 Min.)'],
                    ['Lebenszeichen', 'Ping alle 30 s · zuletzt vor 9 s'],
                    ['Neustarts', state === 'crashloop' ? '5 in 10 Min. – angehalten' : '0 in 10 Min.'],
                  ]}
                />
              </Section>
            </F>
            <div className="flex gap-2">
              <Btn>Aktualisieren</Btn>
              <Btn>Deaktivieren</Btn>
              <Btn variant="danger">Entfernen</Btn>
            </div>
            <p className="mt-1.5 text-[11px] text-muted-foreground">Entfernen löscht Binaries und Konfiguration. Von ihm erzeugte VMs räumt der Reaper auf.</p>
          </aside>
        )}
      </div>
    </SettingsShell>
  )
}

/* ───────────────────────── Installation ───────────────────────── */

function InstallDialog({ state }: { state: string }) {
  const tab = ['registry', 'permissions', 'sig-fail', 'incompatible'].includes(state) ? 'registry' : state === 'git' ? 'git' : 'path'
  const confirm = ['permissions', 'unsigned', 'update'].includes(state)
  /** Eingabe der Quelle nur im ersten Schritt; danach eine Zeile mit der gewählten Quelle. */
  const pick = ['path', 'git', 'registry'].includes(state)
  return (
    <SettingsShell
      active="plugins"
      title="Plugins"
      description="Plugins fügen Harnesses, Runner-Provider und Git-Provider hinzu."
      actions={<Btn variant="primary">Plugin installieren…</Btn>}
    >
      <div className="pointer-events-none opacity-40">
        <PluginTable rows={installed.slice(0, 3)} />
      </div>
      <div className="absolute inset-0 z-10 flex items-start justify-center overflow-y-auto bg-foreground/20 py-10">
        <div className="w-[640px] rounded-lg border border-border bg-popover shadow-xl">
          <div className="border-b border-border px-5 pt-4">
            <h2 className="type-wide text-[16px] font-[650]">{state === 'update' ? 'beton-runner-hetzner aktualisieren' : 'Plugin installieren'}</h2>
            {state !== 'update' && (
              <F id={['PLG-005', 'PLG-006']} className="mt-3 flex gap-1 text-[13px]">
                {[
                  ['path', 'Lokaler Ordner', FolderOpen],
                  ['git', 'Git-URL', GitBranch],
                  ['registry', 'Registry (optional)', Search],
                ].map(([id, label, Icon]) => {
                  const I = Icon as typeof Search
                  return (
                    <span
                      key={id as string}
                      className={cn('-mb-px inline-flex items-center gap-1.5 border-b-2 px-2 pb-2', id === tab ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground')}
                    >
                      <I className="size-3.5" /> {label as string}
                    </span>
                  )
                })}
              </F>
            )}
          </div>

          <div className="px-5 py-4">
            {tab === 'path' && pick && (
              <F id="PLG-005">
                <label className="text-[12px] text-muted-foreground">Ordner mit beton-plugin.toml</label>
                <div className="mt-1 flex gap-2">
                  <span className="flex h-8 flex-1 items-center rounded-md border border-input bg-card px-2 font-mono text-[12.5px]">~/code/beton-runner-hetzner</span>
                  <Btn>Wählen…</Btn>
                </div>
                <div className="mt-1">
                  <Premise kind="local">Kein Download; Binary aus dem Ordner oder per cargo build --release gebaut.</Premise>
                </div>
              </F>
            )}
            {tab === 'git' && pick && (
              <F id="PLG-005">
                <label className="text-[12px] text-muted-foreground">Git-URL, optional mit Tag</label>
                <div className="mt-1 flex h-8 items-center rounded-md border border-input bg-card px-2 font-mono text-[12.5px] outline-2 outline-ring">
                  git+https://codeberg.org/acme/beton-git-gitea#v0.2.0
                  <Cursor />
                </div>
                <div className="mt-1">
                  <Premise kind="network">klont das Repository; Binaries laut Manifest-URL oder lokal gebaut</Premise>
                </div>
              </F>
            )}
            {tab === 'registry' && pick && (
              <F id={['PLG-006', 'PLG-005']}>
                <div className="flex h-8 items-center gap-2 rounded-md border border-input bg-card px-2 text-[13px] outline-2 outline-ring">
                  <Search className="size-3.5 text-muted-foreground" /> runner
                </div>
                <div className="mt-1">
                  <Premise kind="network">lädt den Index github.com/ifahrentholz/beton-plugins (Git, flach) – zuletzt vor 40 Min., höchstens stündlich</Premise>
                </div>
                <div className="mt-3">
                  {[
                    ['beton-runner-hetzner', '0.4.0', 'Runner auf Hetzner-Cloud-VMs', 'acme'],
                    ['beton-runner-proxmox', '0.9.2', 'Runner als Proxmox-LXC (braucht beton ≥ 1.4)', 'homelab-tools'],
                    ['beton-runner-firecracker', '0.2.1', 'Runner in Firecracker-MicroVMs', 'vmm-labs'],
                  ].map(([n, v, d, who]) => (
                    <div key={n} className="flex items-center gap-3 border-b border-border py-2">
                      <div className="min-w-0 flex-1">
                        <div className="text-[13px] font-medium">
                          {n} <span className="font-mono text-[11px] text-muted-foreground">{v}</span>
                        </div>
                        <div className="text-[12px] text-muted-foreground">
                          {d} · signiert von github.com/{who}
                        </div>
                      </div>
                      <Btn>Auswählen</Btn>
                    </div>
                  ))}
                  <p className="mt-2 text-[12px] text-muted-foreground">
                    Weitere Indizes in <C>plugins.indexes</C>. Die Registry ist optional – Ordner und Git-URL funktionieren ohne sie.
                  </p>
                </div>
              </F>
            )}

            {!pick && state !== 'update' && (
              <p className="mb-3 text-[12px] text-muted-foreground">
                {tab === 'registry' ? (
                  <>Aus der Registry: beton-runner-hetzner 0.3.1 · Index beton-plugins</>
                ) : (
                  <>Aus lokalem Ordner: <span className="font-mono">~/code/beton-runner-hetzner</span></>
                )}
              </p>
            )}
            {(state === 'path' || state === 'permissions' || state === 'unsigned' || state === 'sig-fail' || state === 'incompatible' || state === 'update') && (
              <F id={['PLG-004', 'PLG-007', 'PLG-010']} className="mt-4">
                <Section title="Geprüft">
                  <KV
                    rows={[
                      ['Plugin', <>beton-runner-hetzner {state === 'update' ? '0.3.1 → 0.4.0' : '0.3.1'} · Runner-Provider</>],
                      [
                        'Kompatibel',
                        state === 'incompatible' ? (
                          <Mark kind="fail" label="verlangt beton >=2.0 · du hast 1.3.0" />
                        ) : (
                          <Mark kind="ok" label="beton >=1.0, <2.0 · plugin_api 1" />
                        ),
                      ],
                      [
                        'Prüfsumme',
                        state === 'sig-fail' ? <Mark kind="fail" label="stimmt nicht mit dem Manifest überein" /> : <Mark kind="ok" label="sha256 4f1c9a…e07b stimmt" />,
                      ],
                      [
                        'Signatur',
                        state === 'unsigned' || state === 'path' ? (
                          <Mark kind="warn" label="keine – lokaler Ordner" />
                        ) : state === 'sig-fail' ? (
                          <Mark kind="fail" label="Identität passt nicht zum Index" />
                        ) : (
                          <Mark kind="ok" label="cosign · github.com/acme/beton-runner-hetzner (release.yml)" />
                        ),
                      ],
                    ]}
                  />
                </Section>
              </F>
            )}

            {state === 'sig-fail' && (
              <F id="PLG-007" className="rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
                <p className="font-semibold">Nicht installiert: Der Download ist nicht das, was der Herausgeber signiert hat</p>
                <p className="mt-1 text-muted-foreground">
                  Erwartet war eine Signatur von github.com/acme/beton-runner-hetzner, gefunden wurde github.com/acme-mirror/…. Das Archiv wurde verworfen, an deiner
                  Installation hat sich nichts geändert.
                </p>
              </F>
            )}
            {state === 'incompatible' && (
              <F id="PLG-010" className="rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
                <p className="font-semibold">Diese Version passt nicht zu beton 1.3.0</p>
                <p className="mt-1 text-muted-foreground">Kompatible Versionen laut Index: 0.3.1, 0.3.0. Die neueste davon wird empfohlen.</p>
                <div className="mt-2">
                  <Btn variant="primary">0.3.1 installieren</Btn>
                </div>
              </F>
            )}

            {confirm && (
              <F id={['PLG-008', 'PLG-009']}>
                <div className="chamfer border-l-4 border-signal bg-signal-soft p-3">
                  <p className="text-[13px] font-semibold">
                    {state === 'update' ? 'Das Update verlangt zusätzliche Berechtigungen' : 'Das Plugin verlangt diese Berechtigungen'}
                  </p>
                  <div className="mt-2 rounded-sm bg-card px-2 py-1">
                    <Permissions diff={state === 'update'} />
                  </div>
                  <p className="mt-2 text-[12px] text-muted-foreground">
                    Die Sandbox setzt genau diese Liste durch. Spätere Updates mit mehr Rechten fragen erneut.
                  </p>
                  {state === 'unsigned' && (
                    <label className="mt-2 flex items-start gap-2 text-[13px]">
                      <span className="mt-0.5 size-4 shrink-0 rounded-sm border-2 border-foreground" />
                      <span>
                        Ich installiere ein <span className="font-semibold">unsigniertes</span> Plugin. Es bleibt dauerhaft als unsigniert markiert.
                      </span>
                    </label>
                  )}
                </div>
              </F>
            )}
          </div>

          <div className="flex items-center gap-2 border-t border-border px-5 py-3">
            <span className="text-[11px] text-muted-foreground">Ziel: ~/.beton/plugins/beton-runner-hetzner/</span>
            <div className="ml-auto flex gap-2">
              <Btn variant="ghost">{state === 'sig-fail' ? 'Schließen' : 'Abbrechen'}</Btn>
              {confirm && <Btn variant="signal">{state === 'update' ? 'Aktualisieren und gewähren' : 'Installieren und gewähren'}</Btn>}
              {(state === 'path' || tab === 'git') && !confirm && <Btn variant="primary">Weiter</Btn>}
            </div>
          </div>
        </div>
      </div>
    </SettingsShell>
  )
}

/* ───────────────────────── Plugin-Doktor ───────────────────────── */

function DoctorScreen({ state }: { state: string }) {
  const bad = state === 'problems'
  const rows: [string, MarkKind, string, MarkKind, MarkKind, MarkKind, string][] = [
    ['beton-runner-hetzner', 'ok', 'ok', bad ? 'fail' : 'ok', 'ok', 'ok', bad ? 'Antwortet nicht auf capabilities (Timeout 10 s)' : ''],
    ['acme-git-gitea', 'ok', 'ok', 'ok', 'ok', 'warn', 'Unsigniert (Git ohne Signatur)'],
    ['echo-harness', 'ok', 'ok', bad ? 'warn' : 'ok', bad ? 'fail' : 'ok', 'warn', bad ? 'Sandbox nicht verfügbar – startet nicht' : 'Unsigniert (lokaler Pfad)'],
    ['beton-runner-proxmox', 'fail', 'incompatible', 'off', 'ok', 'ok', 'plugin_api 2 – beton spricht 1 (Fehler -32006)'],
  ]
  return (
    <SettingsShell
      active="plugins"
      title="Plugin-Doktor"
      description="Startet jedes Plugin kurz, prüft Handshake, gemeldete Fähigkeiten, Prüfsumme und Sandbox. Ändert nichts an deiner Installation."
      actions={<Btn variant="primary">Erneut prüfen</Btn>}
    >
      <F id={['PLG-011', 'PLG-003', 'PLG-009']} className="max-w-5xl">
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
              <th className="py-1.5 pr-3 font-normal">Plugin</th>
              <th className="py-1.5 pr-3 font-normal">Handshake</th>
              <th className="py-1.5 pr-3 font-normal">Fähigkeiten</th>
              <th className="py-1.5 pr-3 font-normal">Prüfsumme</th>
              <th className="py-1.5 pr-3 font-normal">Sandbox</th>
              <th className="py-1.5 font-normal">Hinweis</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(([n, hs, hsl, cap, sbx, sum, note]) => (
              <tr key={n} className="border-b border-border">
                <td className="py-1.5 pr-3 font-medium">{n}</td>
                <td className="py-1.5 pr-3">
                  <Mark kind={hs} label={hsl} />
                </td>
                <td className="py-1.5 pr-3">
                  <Mark kind={cap} label={cap === 'ok' ? 'wie deklariert' : cap === 'off' ? '–' : cap === 'warn' ? 'weniger als deklariert' : 'keine Antwort'} />
                </td>
                <td className="py-1.5 pr-3">
                  <Mark kind="ok" label="stimmt" />
                </td>
                <td className="py-1.5 pr-3">
                  <Mark kind={sbx} label={sbx === 'ok' ? 'aktiv' : 'fehlt'} />
                </td>
                <td className={cn('py-1.5 text-[12px]', (cap === 'fail' || sbx === 'fail' || hs === 'fail') && 'text-deny')}>
                  {note || (sum === 'warn' ? 'Unsigniert' : '')}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="mt-3 text-[12px] text-muted-foreground">
          Dieselbe Prüfung im Terminal: <C>beton plugin doctor</C> · Ergebnisse fließen in <C>beton doctor</C>.
        </p>
        {bad && (
          <div className="mt-4 max-w-3xl rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
            <p className="font-semibold">echo-harness startet nicht, weil die Sandbox fehlt</p>
            <p className="mt-1 text-muted-foreground">
              Auf diesem Linux fehlt Landlock (Kernel 5.10). beton startet Plugins nie ohne Sandbox. Kernel ≥ 5.13 verwenden oder das Plugin auf einem anderen Host
              betreiben.
            </p>
          </div>
        )}
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Plugin entwickeln (Terminal) ───────────────────────── */

function DevScreen({ state }: { state: string }) {
  const cwd = '~/code/beton-runner-hetzner'
  return (
    <Term>
      {state === 'new' && (
        <F id="PLG-012">
          <Prompt cwd="~/code">beton plugin new --kind runner_provider beton-runner-hetzner</Prompt>
          <L>Angelegt aus dem Template runner_provider:</L>
          <L tone="dim">{'  beton-runner-hetzner/Cargo.toml          (beton-plugin-sdk = "1.3")'}</L>
          <L tone="dim">{'  beton-runner-hetzner/beton-plugin.toml   (Manifest, plugin_api 1)'}</L>
          <L tone="dim">{'  beton-runner-hetzner/src/main.rs         (impl RunnerProviderPlugin, serve_stdio)'}</L>
          <L tone="dim">{'  beton-runner-hetzner/tests/contract.rs   (testing::PluginTestHost)'}</L>
          <L>Weiter: cd beton-runner-hetzner && cargo build && beton plugin test .</L>
          <Blank />
          <Comment>Beispiel zum Abschauen: examples/plugins/echo-harness im beton-Repo</Comment>
        </F>
      )}
      {state === 'validate' && (
        <F id={['PLG-004', 'PLG-012']}>
          <Prompt cwd={cwd}>beton plugin validate .</Prompt>
          <L>
            <S tone="deny">✗</S> beton-plugin.toml:4:14  <S tone="bold">kind</S>: „runner“ ist nicht erlaubt (harness, runner_provider, git_provider)
          </L>
          <L>
            <S tone="deny">✗</S> beton-plugin.toml:22:1  <S tone="bold">binaries."x86_64-unknown-linux-musl".sha256</S> fehlt
          </L>
          <L>
            <S tone="bold">△</S> beton-plugin.toml        <S tone="bold">signature</S> fehlt – Installation nur mit --allow-unsigned
          </L>
          <L>2 Fehler, 1 Hinweis · Schema: beton-plugin.schema.json</L>
          <ExitCode code={1} cwd={cwd} />
        </F>
      )}
      {state === 'test' && (
        <F id={['PLG-013', 'PLG-012']}>
          <Prompt cwd={cwd}>beton plugin test . --junit target/contract.xml</Prompt>
          <L tone="dim">Contract-Suite runner_provider (dieselbe wie für docker und kubernetes)</L>
          {[
            ['handshake: initialize → initialized → capabilities', 'ok', '41 ms'],
            ['capabilities: deklariert = beobachtet', 'ok', '1,2 s'],
            ['provision ist idempotent über runner_id', 'ok', '3,4 s'],
            ['exec mit pty=true liefert interaktiven Stream', 'ok', '2,1 s'],
            ['$/cancelRequest bricht provision ab', 'fail', '10,0 s'],
            ['terminate räumt alle list_managed-Ressourcen ab', 'ok', '4,8 s'],
            ['Fehlercodes: -32001 Unsupported für snapshot', 'ok', '12 ms'],
            ['shutdown → exit binnen 5 s', 'ok', '0,3 s'],
          ].map(([t, r, d]) => (
            <L key={t}>
              {'  '}
              {r === 'ok' ? <S tone="ok">✓</S> : <S tone="deny">✗</S>} {t.padEnd(52)}
              <S tone="dim">{d}</S>
            </L>
          ))}
          <L tone="deny">{'    cancel: provision lief nach $/cancelRequest weiter (Timeout 10 s)'}</L>
          <L>
            7 bestanden, <S tone="deny">1 fehlgeschlagen</S> · JUnit: target/contract.xml
          </L>
          <ExitCode code={1} cwd={cwd} />
        </F>
      )}
      {state === 'trace' && (
        <F id={['PLG-002', 'PLG-003']}>
          <Comment>Protokoll mitlesen: newline-delimited JSON-RPC 2.0 über stdin/stdout</Comment>
          <Prompt cwd={cwd}>BETON_PLUGIN_TRACE=1 beton plugin doctor beton-runner-hetzner</Prompt>
          {[
            ['→', '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"beton_version":"1.3.0","plugin_api":1,"platform":"aarch64-apple-darwin"}}'],
            ['←', '{"jsonrpc":"2.0","id":1,"result":{"plugin_api":1,"name":"beton-runner-hetzner","version":"0.3.1"}}'],
            ['→', '{"jsonrpc":"2.0","method":"initialized"}'],
            ['→', '{"jsonrpc":"2.0","id":2,"method":"capabilities"}'],
            ['←', '{"jsonrpc":"2.0","id":2,"result":{"isolation":"vm","workspace_modes":["clone"],"interactive_exec":true,"snapshot":false}}'],
            ['←', '{"jsonrpc":"2.0","id":7,"method":"host/secret.placeholder","params":{"name":"hetzner/api-token"}}'],
            ['→', '{"jsonrpc":"2.0","id":7,"result":{"placeholder":"bt_cred_8f2a…"}}'],
            ['→', '{"jsonrpc":"2.0","id":3,"method":"$/ping"}'],
            ['←', '{"jsonrpc":"2.0","id":3,"result":{}}'],
            ['→', '{"jsonrpc":"2.0","id":4,"method":"shutdown"}'],
            ['→', '{"jsonrpc":"2.0","method":"exit"}'],
          ].map(([dir, msg], i) => (
            <L key={i}>
              <S tone={dir === '→' ? 'codex' : 'acp'}>{dir}</S> {msg}
            </L>
          ))}
          <L tone="dim">stderr: [info] hcloud client ready (region fsn1)</L>
          <L>
            <S tone="ok">✓</S> beton-runner-hetzner ok
          </L>
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── WASM-Policy-Regeln ───────────────────────── */

function WasmScreen({ state }: { state: string }) {
  const limit = state === 'limit'
  return (
    <SettingsShell
      active="policies-wasm"
      title="WASM-Regeln"
      description="Für Regeln, die CEL nicht ausdrücken kann: eine WebAssembly-Komponente bekommt Hook und Kontext und liefert optional eine Entscheidung. Ohne Dateisystem, ohne Netz."
      actions={<Btn variant="primary">Testfall ausführen</Btn>}
    >
      <div className="grid max-w-6xl grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-10">
        <F id="PLG-014">
          <Section title="Regel risk-score" aside="Projekt shop-frontend · .beton/policies/risk.yaml">
            <TermBlock>
              <L>
                <S tone="codex">rules</S>:
              </L>
              <L>
                {'  - '}
                <S tone="codex">id</S>: risk-score
              </L>
              <L>
                {'    '}
                <S tone="codex">on</S>: pre_tool_use
              </L>
              <L>
                {'    '}
                <S tone="codex">wasm</S>:
              </L>
              <L>
                {'      '}
                <S tone="codex">module</S>: <S tone="ok">./risk.wasm</S>
              </L>
              <L>
                {'      '}
                <S tone="codex">sha256</S>: <S tone="ok">"9c0e…a71f"</S>
              </L>
              <L>
                {'      '}
                <S tone="codex">params</S>: {'{ '}max_files: 20, protected: [<S tone="ok">"infra/"</S>, <S tone="ok">".github/"</S>]{' }'}
              </L>
            </TermBlock>
            <KV
              className="mt-3"
              rows={[
                ['Schnittstelle', <C>beton:policy@0.1.0 · world policy-extension</C>],
                ['Isolation', 'kein Dateisystem, kein Netz, nur log()'],
                ['Grenzen', '64 MiB Speicher · 50 ms pro Aufruf · Ergebnis ≤ 64 KiB'],
                ['Bei Fehler', 'wie deny (fail closed)'],
              ]}
            />
          </Section>
        </F>
        <F id="PLG-014">
          <Section title="Testfall" aside="git push mit Änderungen in infra/">
            <TermBlock>
              <L tone="dim">evaluate("pre_tool_use", context, params)</L>
              {limit ? (
                <>
                  <L tone="deny">✗ Zeitlimit: 50 ms überschritten (Aufruf nach 50 ms abgebrochen)</L>
                  <L>
                    → Entscheidung <S tone="deny">deny</S> · Grund: „Policy-Modul risk.wasm antwortet nicht rechtzeitig“
                  </L>
                </>
              ) : (
                <>
                  <L>
                    → <S tone="bold">ask</S> · „3 Dateien in infra/ betroffen – bitte bestätigen“
                  </L>
                  <L tone="dim">4,1 ms · 2,3 MiB Speicher</L>
                </>
              )}
            </TermBlock>
            {limit && (
              <p className="mt-2 text-[13px] text-deny">
                Das Modul hat sein Zeitlimit überschritten. beton lehnt ab, statt ungeprüft durchzulassen; im Verlauf erscheint die Regel als Grund.
              </p>
            )}
          </Section>
        </F>
      </div>
    </SettingsShell>
  )
}

/* ───────────────────────── Gruppe ───────────────────────── */

export const group: ScreenGroup = {
  id: 'plugins',
  title: 'Plugins',
  order: 170,
  screens: [
    {
      id: 'plugins-list',
      title: 'Plugins',
      description:
        'Eingebaute Adapter und installierte Plugins mit Quelle (eingebaut, Registry, Git, lokaler Pfad), Signatur und Status. Rechts Manifest, gewährte Rechte und Prozesszustand.',
      features: ['PLG-001', 'PLG-003', 'PLG-004', 'PLG-005', 'PLG-008', 'PLG-009', 'PLG-010', 'PLG-011'],
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'crashloop', title: 'Stürzt ab' },
        { id: 'incompatible', title: 'Inkompatibel' },
        { id: 'empty', title: 'Keine Plugins' },
      ],
      component: PluginsList,
    },
    {
      id: 'plugins-install',
      title: 'Plugin installieren',
      description:
        'Aus lokalem Ordner (geht immer, ohne Netz), Git-URL oder optionaler Registry. Vor der Installation: Prüfsumme, Signatur, Kompatibilität – dann bestätigst du die Berechtigungen.',
      features: ['PLG-004', 'PLG-005', 'PLG-006', 'PLG-007', 'PLG-008', 'PLG-009', 'PLG-010'],
      states: [
        { id: 'path', title: 'Lokaler Ordner' },
        { id: 'git', title: 'Git-URL' },
        { id: 'registry', title: 'Registry' },
        { id: 'permissions', title: 'Berechtigungen' },
        { id: 'unsigned', title: 'Unsigniert' },
        { id: 'sig-fail', title: 'Signatur ungültig' },
        { id: 'incompatible', title: 'Inkompatibel' },
        { id: 'update', title: 'Update mit mehr Rechten' },
      ],
      component: InstallDialog,
    },
    {
      id: 'plugins-doctor',
      title: 'Plugin-Doktor',
      description: 'Handshake, Fähigkeiten, Prüfsumme und Sandbox je Plugin. Ohne Sandbox startet ein Plugin nicht.',
      features: ['PLG-011', 'PLG-003', 'PLG-009'],
      states: [
        { id: 'ok', title: 'Alles in Ordnung' },
        { id: 'problems', title: 'Probleme' },
      ],
      component: DoctorScreen,
    },
    {
      id: 'plugins-dev',
      title: 'Plugin entwickeln',
      description: 'Für Plugin-Autoren: Gerüst aus dem SDK, Manifest prüfen, Contract-Suite (dieselbe wie für eingebaute Adapter) mit JUnit-Ausgabe, Protokoll mitlesen.',
      features: ['PLG-002', 'PLG-003', 'PLG-004', 'PLG-012', 'PLG-013'],
      frame: 'terminal',
      states: [
        { id: 'new', title: 'plugin new' },
        { id: 'validate', title: 'plugin validate' },
        { id: 'test', title: 'plugin test' },
        { id: 'trace', title: 'JSON-RPC mitlesen' },
      ],
      component: DevScreen,
    },
    {
      id: 'plugins-wasm',
      title: 'WASM-Regeln',
      description: 'Policy-Regel als WebAssembly-Komponente mit WIT-Schnittstelle, strikten Grenzen und Testfall. Zeitüberschreitung führt zu deny.',
      features: ['PLG-014'],
      states: [
        { id: 'default', title: 'Testfall ok' },
        { id: 'limit', title: 'Zeitlimit' },
      ],
      component: WasmScreen,
    },
  ],
}
