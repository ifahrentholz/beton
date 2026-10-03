import { Apple, Box, Monitor, Terminal as TerminalIcon } from 'lucide-react'
import type { ReactNode } from 'react'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { Ask, Comment, Cursor, ExitCode, L, Prompt, S, Term, TermBlock } from '../cli/term'
import { Btn, C, KV, Mark, Premise, Section, SettingsShell } from '../hosts/settings-shell'

/* ───────────────────────── Installationswege ───────────────────────── */

type Way = { id: string; name: string; cmd: string; note: string; since: string; features: string[]; update: string }

const ways: Record<string, Way[]> = {
  macos: [
    { id: 'brew', name: 'Homebrew', cmd: 'brew install ifahrentholz/tap/beton', note: 'CLI, vorgebaut für arm64 und x64', since: 'M3', features: ['DIST-004'], update: 'brew upgrade beton' },
    { id: 'brew-cask', name: 'Desktop-App (Homebrew)', cmd: 'brew install --cask ifahrentholz/tap/beton-desktop', note: 'Universal-App, signiert und notarisiert', since: 'M3', features: ['DIST-004', 'DIST-008'], update: 'brew upgrade --cask beton-desktop' },
    { id: 'script', name: 'Installer-Skript', cmd: 'curl -fsSL https://github.com/ifahrentholz/beton/releases/latest/download/install.sh | sh', note: 'nach ~/.beton/bin, prüft SHA-256 und – falls vorhanden – cosign; PATH nur nach Rückfrage', since: 'M3', features: ['DIST-003'], update: 'beton upgrade' },
    { id: 'binstall', name: 'cargo binstall', cmd: 'cargo binstall beton-cli', note: 'lädt das Release-Binary, ohne zu kompilieren; cargo install beton-cli baut aus dem Quellcode', since: 'M3', features: ['DIST-005'], update: 'cargo binstall beton-cli' },
    { id: 'dmg', name: 'Desktop-App (.dmg)', cmd: 'beton_1.4.0_universal.dmg', note: 'aus den GitHub-Releases, in den Programme-Ordner ziehen', since: 'M3', features: ['DIST-008'], update: 'in der App: Einstellungen → Updates' },
  ],
  linux: [
    { id: 'script', name: 'Installer-Skript', cmd: 'curl -fsSL https://github.com/ifahrentholz/beton/releases/latest/download/install.sh | sh', note: 'statisches Binary (musl) für amd64 und arm64', since: 'M3', features: ['DIST-003'], update: 'beton upgrade' },
    { id: 'brew', name: 'Homebrew', cmd: 'brew install ifahrentholz/tap/beton', note: 'auch unter Linux', since: 'M3', features: ['DIST-004'], update: 'brew upgrade beton' },
    { id: 'deb', name: 'Debian/Ubuntu (.deb)', cmd: 'sudo apt install ./beton_1.4.0_amd64.deb', note: 'mit systemd-User-Units für serve und host (nicht aktiviert), Completions, Manpage', since: 'M5', features: ['DIST-007'], update: 'neues .deb installieren' },
    { id: 'rpm', name: 'Fedora/RHEL (.rpm)', cmd: 'sudo dnf install ./beton-1.4.0-1.x86_64.rpm', note: 'wie .deb', since: 'M5', features: ['DIST-007'], update: 'neues .rpm installieren' },
    { id: 'binstall', name: 'cargo binstall', cmd: 'cargo binstall beton-cli', note: 'ohne Kompilierung', since: 'M3', features: ['DIST-005'], update: 'cargo binstall beton-cli' },
    { id: 'appimage', name: 'Desktop-App', cmd: 'beton_1.4.0_amd64.AppImage  ·  .deb  ·  .rpm', note: 'Flatpak folgt in v2', since: 'M3', features: ['DIST-008'], update: 'in der App: Einstellungen → Updates' },
  ],
  windows: [
    { id: 'ps', name: 'PowerShell-Skript', cmd: 'irm https://github.com/ifahrentholz/beton/releases/latest/download/install.ps1 | iex', note: 'nach %LOCALAPPDATA%\\beton\\bin', since: 'M3', features: ['DIST-003'], update: 'beton upgrade' },
    { id: 'winget', name: 'winget', cmd: 'winget install ifahrentholz.beton', note: 'CLI (portable); Desktop: ifahrentholz.beton.Desktop', since: 'M5', features: ['DIST-006'], update: 'winget upgrade ifahrentholz.beton' },
    { id: 'scoop', name: 'Scoop', cmd: 'scoop bucket add ifahrentholz https://github.com/ifahrentholz/scoop-bucket; scoop install beton', note: 'eigener Bucket', since: 'M5', features: ['DIST-006'], update: 'scoop update beton' },
    { id: 'msi', name: 'Desktop-App', cmd: 'beton_1.4.0_x64_de-DE.msi  ·  beton_1.4.0_x64-setup.exe', note: 'Windows ist Beta; signiert mit Azure Trusted Signing', since: 'M3', features: ['DIST-008'], update: 'in der App: Einstellungen → Updates' },
  ],
}

function WayRow({ w }: { w: Way }) {
  return (
    <F id={w.features} className="grid grid-cols-[11rem_minmax(0,1fr)_3rem] gap-4 border-b border-border py-3">
      <div>
        <div className="text-[13px] font-semibold">{w.name}</div>
        <div className="text-[11px] text-muted-foreground">Update: {w.update}</div>
      </div>
      <div className="min-w-0">
        <div className="dark flex items-center gap-2 rounded-md bg-sunken px-3 py-1.5 font-mono text-[12px] text-foreground">
          <span className="text-muted-foreground select-none">$</span>
          <span className="min-w-0 flex-1 truncate">{w.cmd}</span>
          <button className="shrink-0 text-[11px] text-muted-foreground hover:text-foreground">Kopieren</button>
        </div>
        <div className="mt-1 text-[12px] text-muted-foreground">{w.note}</div>
      </div>
      <span className="pt-1.5 text-right font-mono text-[11px] text-muted-foreground">ab {w.since}</span>
    </F>
  )
}

const assets: [string, string][] = [
  ['beton-1.4.0-aarch64-apple-darwin.tar.gz', '18,2 MB'],
  ['beton-1.4.0-x86_64-apple-darwin.tar.gz', '19,0 MB'],
  ['beton-1.4.0-x86_64-unknown-linux-musl.tar.gz', '20,4 MB'],
  ['beton-1.4.0-aarch64-unknown-linux-musl.tar.gz', '19,7 MB'],
  ['beton-1.4.0-x86_64-pc-windows-msvc.zip', '17,9 MB'],
  ['beton_1.4.0_universal.dmg', '41,3 MB'],
  ['SHA256SUMS', '2 KB'],
  ['*.sigstore.json', 'je 6 KB'],
  ['beton-1.4.0.cdx.json (SBOM)', '310 KB'],
  ['install.sh · install.ps1', '9 KB'],
  ['latest-stable.json (Desktop-Updater)', '1 KB'],
]

function Page({ children }: { children: ReactNode }) {
  return <div className="h-full min-h-[640px] overflow-y-auto rounded-lg border border-border bg-background">{children}</div>
}

function InstallWays({ state }: { state: string }) {
  const tabs = [
    { id: 'macos', label: 'macOS', icon: Apple },
    { id: 'linux', label: 'Linux', icon: TerminalIcon },
    { id: 'windows', label: 'Windows', icon: Monitor },
    { id: 'container', label: 'Server & Container', icon: Box },
  ]
  return (
    <Page>
      <div className="mx-auto max-w-6xl px-8 py-8">
        <h1 className="type-wide text-[26px] font-[750]">beton installieren</h1>
        <p className="mt-1 max-w-2xl text-[14px] text-muted-foreground">
          Ein Binary für CLI, lokalen Server und Host; dazu die Desktop-App. Alles läuft auf deinem Rechner. Du brauchst keinen API-Schlüssel – beton nutzt die
          Anmeldung von claude, codex und gemini.
        </p>
        <div className="mt-5 flex gap-1 border-b border-border text-[13px]">
          {tabs.map((t) => (
            <span
              key={t.id}
              className={cn('-mb-px inline-flex items-center gap-1.5 border-b-2 px-3 pb-2', t.id === state ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground')}
            >
              <t.icon className="size-3.5" /> {t.label}
            </span>
          ))}
        </div>

        <div className="mt-2 grid grid-cols-[minmax(0,1fr)_19rem] gap-10">
          <div>
            {state !== 'container' ? (
              ways[state].map((w) => <WayRow key={w.id} w={w} />)
            ) : (
              <>
                <p className="mt-3 text-[13px] text-muted-foreground">
                  Nur nötig, wenn du für ein Team einen zentralen Server betreibst. Für dich allein reicht <C>beton serve</C> auf deinem Rechner.
                </p>
                <F id="DIST-009" className="border-b border-border py-3">
                  <div className="text-[13px] font-semibold">Server-Image</div>
                  <TermBlock className="mt-1">
                    <Prompt>docker run -d -p 7420:7420 -v beton-data:/data ghcr.io/ifahrentholz/beton-server:1.4</Prompt>
                  </TermBlock>
                  <p className="mt-1 text-[12px] text-muted-foreground">
                    Nur das beton-Binary und die Web-UI, ohne Harnesses. Non-root, distroless. Tags <C>1.4.0</C> <C>1.4</C> <C>latest</C> <C>beta</C> <C>nightly</C>.
                  </p>
                </F>
                <F id={['DIST-010', 'RUN-014']} className="border-b border-border py-3">
                  <div className="text-[13px] font-semibold">Runner-Image</div>
                  <TermBlock className="mt-1">
                    <Prompt>docker pull ghcr.io/ifahrentholz/beton-runner:1.4.0</Prompt>
                  </TermBlock>
                  <p className="mt-1 text-[12px] text-muted-foreground">
                    Mit claude, codex und gemini; wöchentlich neu gebaut (<C>1.4.0-r2</C>). Oder selbst bauen aus <C>deploy/runner-image</C>.
                  </p>
                </F>
                <F id="DIST-011" className="border-b border-border py-3">
                  <div className="text-[13px] font-semibold">docker compose</div>
                  <TermBlock className="mt-1">
                    <Prompt>cd deploy/docker-compose && cp .env.example .env && docker compose up -d</Prompt>
                    <L tone="dim">✔ Container beton-postgres  Healthy</L>
                    <L tone="dim">✔ Container beton-server    Healthy (/readyz)</L>
                    <L tone="dim">✔ Container beton-caddy     Started (TLS für beton.team.example)</L>
                  </TermBlock>
                  <p className="mt-1 text-[12px] text-muted-foreground">Server, Postgres, optional MinIO und Caddy. OIDC, Datenbank und Master-Key in .env.</p>
                </F>
                <F id="DIST-012" className="border-b border-border py-3">
                  <div className="text-[13px] font-semibold">Helm (Kubernetes)</div>
                  <TermBlock className="mt-1">
                    <Prompt>{'helm install beton oci://ghcr.io/ifahrentholz/charts/beton --version 1.4.0 \\'}</Prompt>
                    <L>{'    --set host.enabled=true --set host.kubernetes.namespace=beton-runners'}</L>
                  </TermBlock>
                  <p className="mt-1 text-[12px] text-muted-foreground">
                    Server mit Probes und Ingress; optional ein Host mit Kubernetes-Provider, RBAC nur im Runner-Namespace, Egress-NetworkPolicy.
                  </p>
                </F>
              </>
            )}

            <F id="DIST-020" className="mt-6">
              <h2 className="text-[13px] font-semibold">Deinstallieren</h2>
              <p className="mt-1 text-[13px] text-muted-foreground">
                <C>beton uninstall</C> entfernt Binary und Dienste und lässt deine Daten in <C>~/.beton</C>. Mit <C>--purge</C> – nach Rückfrage – auch Daten, Logs,
                Modelle und Schlüsselbund-Einträge. Bei Paketmanagern nennt beton den passenden Befehl; die Desktop-App entfernst du wie jede andere App.
              </p>
            </F>
          </div>

          <aside className="pt-3">
            <F id={['DIST-002', 'DIST-013']}>
              <Section title="Release 1.4.0" aside="30.09.2026">
                <ul className="flex flex-col text-[12px]">
                  {assets.map(([n, size]) => (
                    <li key={n} className="flex justify-between gap-2 border-b border-border/70 py-1">
                      <span className="truncate font-mono text-[11.5px]">{n}</span>
                      <span className="shrink-0 text-muted-foreground">{size}</span>
                    </li>
                  ))}
                </ul>
                <p className="mt-2 text-[12px] text-muted-foreground">
                  Alles signiert (cosign, macOS notarisiert, Windows Authenticode) und mit SLSA-Provenance. <span className="underline">So prüfst du die Signatur</span>.
                </p>
                <p className="mt-1 text-[12px] text-muted-foreground">
                  Stabiler Link ohne Version: <C>…/releases/latest/download/beton-&lt;target&gt;.tar.gz</C>
                </p>
              </Section>
            </F>
            <F id="DIST-001">
              <Section title="Plattformen">
                <table className="w-full text-[12px]">
                  <tbody>
                    {[
                      ['macOS', 'arm64, x64', 'CLI + App'],
                      ['Linux', 'amd64, arm64', 'CLI + App'],
                      ['Windows', 'x64, arm64', 'Beta'],
                      ['Container', 'amd64, arm64', 'Server, Runner'],
                    ].map(([os, arch, what]) => (
                      <tr key={os} className="border-b border-border/70">
                        <td className="py-1 pr-2 font-medium">{os}</td>
                        <td className="py-1 pr-2 font-mono text-[11px]">{arch}</td>
                        <td className="py-1 text-muted-foreground">{what}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <p className="mt-1 text-[11px] text-muted-foreground">Reproduzierbar in CI gebaut (--locked, gepinnte Toolchain).</p>
              </Section>
            </F>
            <Premise kind="network">Updates kommen nie von selbst. Du prüfst mit beton upgrade oder in der App.</Premise>
          </aside>
        </div>
      </div>
    </Page>
  )
}

/* ───────────────────────── Installer & Verifikation (Terminal) ───────────────────────── */

const TGZ = 'beton-1.4.0-aarch64-apple-darwin.tar.gz'

function InstallVerify({ state }: { state: string }) {
  return (
    <Term>
      {state === 'installer' && (
        <F id={['DIST-003', 'DIST-002']}>
          <Prompt cwd="~">curl -fsSL https://github.com/ifahrentholz/beton/releases/latest/download/install.sh | sh</Prompt>
          <L>
            <S tone="bold">beton-Installer</S> · macOS arm64 · Kanal stable
          </L>
          <L tone="dim">Lade {TGZ} (18,2 MB) …</L>
          <L>
            {'  '}
            <S tone="ok">✓</S> SHA-256 stimmt mit SHA256SUMS überein
          </L>
          <L>
            {'  '}
            <S tone="ok">✓</S> cosign gefunden – Signatur gültig (release.yml@refs/tags/v1.4.0)
          </L>
          <L>
            {'  '}
            <S tone="ok">✓</S> installiert nach ~/.beton/bin/beton
          </L>
          <Ask title="~/.beton/bin ist nicht in deinem PATH" question="Zeile an ~/.zshrc anhängen?">
            <L>{'  '}export PATH="$HOME/.beton/bin:$PATH"</L>
          </Ask>
        </F>
      )}
      {state === 'verify' && (
        <F id={['DIST-013', 'DIST-002']}>
          <Comment>Selbst prüfen, ohne dem Installer zu vertrauen</Comment>
          <Prompt cwd="~/Downloads">shasum -a 256 -c SHA256SUMS --ignore-missing</Prompt>
          <L>
            {TGZ}: <S tone="ok">OK</S>
          </L>
          <Prompt cwd="~/Downloads">{`cosign verify-blob ${TGZ} --bundle ${TGZ}.sigstore.json \\`}</Prompt>
          <L>{'    --certificate-identity "https://github.com/ifahrentholz/beton/.github/workflows/release.yml@refs/tags/v1.4.0" \\'}</L>
          <L>{'    --certificate-oidc-issuer https://token.actions.githubusercontent.com'}</L>
          <L tone="ok">Verified OK</L>
          <Prompt cwd="~/Downloads">gh attestation verify {TGZ} --repo ifahrentholz/beton</Prompt>
          <L>
            <S tone="ok">✓</S> Verification succeeded! · SLSA Build-Provenance, gebaut von release.yml auf GitHub Actions
          </L>
          <Prompt cwd="~/Downloads">spctl -a -vv /Applications/beton.app</Prompt>
          <L>
            /Applications/beton.app: <S tone="ok">accepted</S>
          </L>
          <L>source=Notarized Developer ID</L>
        </F>
      )}
      {state === 'fail' && (
        <F id={['DIST-013', 'DIST-003']}>
          <Prompt cwd="~">curl -fsSL https://github.com/ifahrentholz/beton/releases/latest/download/install.sh | sh</Prompt>
          <L>
            <S tone="bold">beton-Installer</S> · macOS arm64 · Kanal stable
          </L>
          <L tone="dim">Lade {TGZ} (18,2 MB) …</L>
          <L tone="deny">✗ SHA-256 stimmt nicht mit SHA256SUMS überein.</L>
          <L>{'  '}erwartet 4f1c9a…e07b, erhalten 0d77b2…91c4</L>
          <L>Abgebrochen. Es wurde nichts installiert. Lade erneut oder prüfe, ob ein Proxy die Datei verändert.</L>
          <ExitCode code={1} cwd="~" />
          <Prompt cwd="~/Downloads">cosign verify-blob {TGZ} --bundle {TGZ}.sigstore.json --certificate-identity … </Prompt>
          <L tone="deny">Error: none of the expected identities matched what was in the certificate</L>
          <ExitCode code={1} cwd="~/Downloads" />
          <Prompt cwd="~/Downloads">
            <Cursor />
          </Prompt>
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── Updates (Desktop) ───────────────────────── */

function Radio({ on, title, desc }: { on?: boolean; title: string; desc: string }) {
  return (
    <div className="flex gap-3 border-b border-border py-2">
      <span className={cn('mt-0.5 size-4 shrink-0 rounded-full border', on ? 'border-[5px] border-foreground' : 'border-input')} />
      <div>
        <div className="text-[13px] font-medium">{title}</div>
        <div className="text-[12px] text-muted-foreground">{desc}</div>
      </div>
    </div>
  )
}

function Toggle({ on, label, children }: { on?: boolean; label: string; children?: ReactNode }) {
  return (
    <div className="flex items-start gap-3 py-2">
      <span className={cn('mt-0.5 flex h-[18px] w-8 shrink-0 items-center rounded-full p-px', on ? 'justify-end bg-foreground' : 'bg-input')}>
        <span className="size-4 rounded-full bg-background" />
      </span>
      <div>
        <div className="text-[13px] font-medium">{label}</div>
        {children}
      </div>
    </div>
  )
}

const changelog: { sec: string; items: [string, string][] }[] = [
  {
    sec: 'Features',
    items: [
      ['CLI-014', 'Shell-Vervollständigung für Session-IDs mit Titel'],
      ['RUN-015', 'Codex-Login im Container per Device-Flow'],
      ['PLG-013', 'beton plugin test schreibt JUnit-XML'],
    ],
  },
  {
    sec: 'Fehlerbehebungen',
    items: [
      ['SES-005', 'Unterbrechen während eines Tool-Calls hinterlässt keine Zombie-Prozesse'],
      ['DESK-005', 'Keine doppelten Benachrichtigungen nach Standby'],
    ],
  },
  {
    sec: 'Breaking Changes',
    items: [['CLI-010', 'beton usage --json: Feld cost wird zu cost_eur. Skripte bitte anpassen.']],
  },
]

function Changelog() {
  return (
    <F id="DIST-019" className="mt-3">
      {changelog.map((c) => (
        <div key={c.sec} className="mb-2">
          <div className={cn('text-[12px] font-semibold', c.sec === 'Breaking Changes' && 'text-deny')}>{c.sec}</div>
          <ul className="mt-0.5 flex flex-col gap-0.5 text-[12.5px]">
            {c.items.map(([id, t]) => (
              <li key={id + t} className="flex gap-2">
                <span className="w-16 shrink-0 font-mono text-[11px] text-muted-foreground">{id}</span>
                <span>{t}</span>
              </li>
            ))}
          </ul>
        </div>
      ))}
      <p className="text-[11px] text-muted-foreground">Vollständig: CHANGELOG.md im Release</p>
    </F>
  )
}

function UpdateScreen({ state }: { state: string }) {
  const brew = state === 'brew'
  return (
    <SettingsShell
      active="updates"
      title="Updates"
      description="beton aktualisiert sich nur, wenn du es willst. Laufende Sessions werden nie unterbrochen, ohne dass du es bestätigst."
    >
      <div className="grid max-w-6xl grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-10">
        <div>
          <F id={['DIST-016', 'DIST-004']}>
            <Section title="Installiert">
              <KV
                rows={[
                  ['Version', <>beton 1.3.0 <span className="text-muted-foreground">· Kanal stable</span></>],
                  ['Installiert über', brew ? 'Homebrew (Formel beton, Cask beton-desktop)' : 'Desktop-App (.dmg) · CLI per Installer-Skript'],
                  ['Zuletzt gesucht', state === 'idle' ? 'noch nie' : 'heute, 09:14 (von dir)'],
                ]}
              />
            </Section>
          </F>

          {brew ? (
            <F id={['DIST-016', 'DIST-004']}>
              <Section title="Updates kommen über Homebrew">
                <p className="text-[13px] text-muted-foreground">
                  Damit Homebrew den Überblick behält, aktualisiert sich beton hier nicht selbst. 1.4.0 ist verfügbar:
                </p>
                <TermBlock className="mt-2">
                  <Prompt>brew upgrade beton && brew upgrade --cask beton-desktop</Prompt>
                </TermBlock>
              </Section>
            </F>
          ) : (
            <F id={['DIST-014', 'DIST-016']}>
              <Section title="Nach Updates suchen">
                <div className="flex items-center gap-2">
                  <Btn variant="primary">Jetzt nach Updates suchen</Btn>
                  <Btn>Update aus Datei…</Btn>
                </div>
                <div className="mt-2 flex flex-col gap-1">
                  <Premise kind="network">fragt github.com nach latest-stable.json; sendet nichts über dich</Premise>
                  <Premise kind="local">Aus Datei: Update-Paket mit Signatur, z. B. per USB-Stick – ohne Netz</Premise>
                </div>
                <Toggle label="Automatisch suchen">
                  <p className="text-[12px] text-muted-foreground">
                    Aus. Wenn an: beim Start und alle 6 Std. Installiert wird trotzdem erst nach deiner Bestätigung.
                  </p>
                </Toggle>
              </Section>
            </F>
          )}

          <F id="DIST-017">
            <Section title="Kanal">
              <Radio on title="stable" desc="Getaggte Releases. Empfohlen." />
              <Radio title="beta" desc="Vorabversionen 1.5.0-beta.N, etwa alle zwei Wochen." />
              <Radio title="nightly" desc="Täglicher Build von main, nur bei Änderungen; die letzten 14 bleiben verfügbar." />
              <p className="mt-2 text-[12px] text-muted-foreground">
                Gilt für App und CLI (<C>update.channel</C>). Zurück zu stable heißt kein Downgrade: Du bleibst, bis stable dich überholt.
              </p>
            </Section>
          </F>
        </div>

        <div>
          {state === 'available' && (
            <F id={['DIST-014', 'DIST-019']}>
              <div className="chamfer border-l-4 border-signal bg-signal-soft p-4">
                <p className="text-[14px] font-semibold">beton 1.4.0 ist bereit</p>
                <p className="mt-0.5 text-[12px] text-muted-foreground">stable · 30.09.2026 · Signatur des Update-Manifests geprüft</p>
                <Changelog />
                <p className="mt-2 text-[12.5px]">Der Daemon startet neu, sobald keine Turns laufen. Gerade sind alle Sessions untätig.</p>
                <div className="mt-3 flex gap-2">
                  <Btn variant="signal">Installieren und neu starten</Btn>
                  <Btn>Beim nächsten Start</Btn>
                  <Btn variant="ghost">Überspringen</Btn>
                </div>
              </div>
            </F>
          )}
          {state === 'draining' && (
            <F id="DIST-014">
              <div className="chamfer border-l-4 border-signal bg-signal-soft p-4">
                <p className="text-[14px] font-semibold">1.4.0 ist installiert – Neustart wartet auf 2 Turns</p>
                <ul className="mt-2 flex flex-col gap-1 text-[13px]">
                  <li className="flex items-center gap-2">
                    <Mark kind="busy" /> Review: Rate-Limiter <span className="text-[12px] text-muted-foreground">Codex · seit 3 Min.</span>
                  </li>
                  <li className="flex items-center gap-2">
                    <Mark kind="busy" /> Nächtliches Dependency-Update <span className="text-[12px] text-muted-foreground">Claude Code · im Hintergrund</span>
                  </li>
                </ul>
                <p className="mt-2 text-[12.5px] text-muted-foreground">Danach startet beton automatisch neu. Die App bleibt bis dahin auf 1.3.0.</p>
                <div className="mt-3 flex gap-2">
                  <Btn variant="signal">Jetzt neu starten (unterbricht 2 Turns)</Btn>
                  <Btn variant="ghost">Warten</Btn>
                </div>
              </div>
            </F>
          )}
          {state === 'file' && (
            <F id={['DIST-014', 'DIST-016']}>
              <Section title="Update aus Datei">
                <div className="rounded-md border border-dashed border-border px-4 py-5 text-center text-[13px]">
                  <p className="font-medium">beton_1.4.0_aarch64.app.tar.gz</p>
                  <p className="text-[12px] text-muted-foreground">mit beton_1.4.0_aarch64.app.tar.gz.sig aus demselben Ordner</p>
                </div>
                <KV
                  className="mt-3"
                  rows={[
                    ['Signatur', <Mark kind="ok" label="gültig (Update-Schlüssel von beton)" />],
                    ['Version', <Mark kind="ok" label="1.4.0 – neuer als 1.3.0" />],
                    ['Kanal', 'stable'],
                  ]}
                />
                <div className="mt-3">
                  <Btn variant="primary">Installieren</Btn>
                </div>
              </Section>
            </F>
          )}
          {state === 'invalid' && (
            <F id="DIST-014">
              <div className="rounded-md border border-deny/40 bg-deny-soft p-4 text-[13px]">
                <p className="font-semibold">Update verworfen: Signatur ungültig</p>
                <p className="mt-1 text-muted-foreground">
                  Das Update-Manifest für 1.4.0 ist nicht mit dem Update-Schlüssel von beton signiert. Es wurde nichts installiert; der Vorfall steht im Log. Versuche es
                  später erneut oder lade das Update selbst von den GitHub-Releases.
                </p>
                <div className="mt-2 flex gap-2">
                  <Btn>Log öffnen</Btn>
                  <Btn variant="ghost">Releases öffnen</Btn>
                </div>
              </div>
            </F>
          )}
          {state === 'idle' && (
            <div className="concrete-grain rounded-md border border-dashed border-border px-6 py-10 text-center">
              <p className="text-[14px] font-medium">Noch nicht gesucht</p>
              <p className="mx-auto mt-1 max-w-xs text-[12.5px] text-muted-foreground">beton fragt erst, wenn du auf „Jetzt nach Updates suchen“ klickst.</p>
            </div>
          )}
        </div>
      </div>
    </SettingsShell>
  )
}

/* ───────────────────────── Versionen & Kompatibilität ───────────────────────── */

function CompatScreen({ state }: { state: string }) {
  const refused = state === 'refused'
  const rows: [string, string, string, 'ok' | 'fail' | 'warn', string][] = [
    ['Diese App', 'Desktop', '1.4.0', 'ok', ''],
    ['Lokaler Daemon', 'Server + Host local', '1.4.0', 'ok', ''],
    ['beton.team.example', 'zentraler Server', refused ? '1.2.3' : '1.3.2', refused ? 'fail' : 'ok', refused ? 'zwei Minor-Versionen älter' : ''],
    ['build-box-01', 'Host', '1.3.0', 'ok', ''],
    ['old-box', 'Host', '1.1.0', 'fail', 'außerhalb des Fensters'],
  ]
  return (
    <SettingsShell
      active="updates"
      connection={refused ? 'offline' : 'server'}
      title="Versionen & Kompatibilität"
      description="App, Server, Hosts und Runner dürfen höchstens eine Minor-Version auseinanderliegen – in beide Richtungen. So musst du nicht alles gleichzeitig aktualisieren."
    >
      <F id="DIST-018" className="max-w-4xl">
        {refused && (
          <div className="mb-4 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
            <p className="font-semibold">Keine Verbindung zu beton.team.example</p>
            <p className="mt-1 text-muted-foreground">
              Der Server läuft mit 1.2.3, diese App ist 1.4.0. Erlaubt sind Server von 1.3 bis 1.5. Bitte die Betreiberin, den Server zu aktualisieren – deine lokalen
              Sessions laufen weiter.
            </p>
            <p className="mt-1 font-mono text-[11px] text-muted-foreground">WS-Close 4400 · version_incompatible</p>
          </div>
        )}
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
              <th className="py-1.5 pr-3 font-normal">Komponente</th>
              <th className="py-1.5 pr-3 font-normal">Rolle</th>
              <th className="py-1.5 pr-3 font-normal">Version</th>
              <th className="py-1.5 font-normal">Verbindung</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(([n, role, v, st, note]) => (
              <tr key={n} className="border-b border-border">
                <td className="py-1.5 pr-3 font-medium">{n}</td>
                <td className="py-1.5 pr-3 text-muted-foreground">{role}</td>
                <td className="py-1.5 pr-3 font-mono text-[12px]">{v}</td>
                <td className="py-1.5">
                  <Mark kind={st} label={st === 'ok' ? 'kompatibel' : `abgelehnt – ${note}`} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="mt-3 flex items-center gap-1 text-[12px]">
          <span className="text-muted-foreground">Fenster für 1.4:</span>
          {['1.2', '1.3', '1.4', '1.5', '1.6'].map((v) => (
            <span
              key={v}
              className={cn(
                'rounded-sm border px-1.5 py-0.5 font-mono',
                ['1.3', '1.4', '1.5'].includes(v) ? 'border-foreground' : 'border-dashed border-border text-muted-foreground',
                v === '1.4' && 'bg-foreground text-background',
              )}
            >
              {v}
            </span>
          ))}
          <span className="ml-2 text-muted-foreground">Protokoll v1 · Plugin-API 1</span>
        </div>
        {state === 'downgrade' && (
          <div className="mt-4 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
            <p className="font-semibold">beton 1.3.0 startet nicht mit dieser Datenbank</p>
            <p className="mt-1 text-muted-foreground">
              Die Daten wurden schon von 1.4.0 migriert. Ältere Versionen können sie nicht lesen. Installiere 1.4.0 wieder – oder stelle die Sicherung von vor der
              Migration her: <C>~/.beton/backups/beton-2026-09-30T14-02.db</C>.
            </p>
          </div>
        )}
        {state !== 'downgrade' && (
          <p className="mt-4 text-[12px] text-muted-foreground">
            Vor jeder Datenbank-Migration legt beton eine Sicherung an (<C>~/.beton/backups/</C>). Zurück auf eine ältere Version geht nur mit dieser Sicherung.
          </p>
        )}
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Gruppe ───────────────────────── */

export const group: ScreenGroup = {
  id: 'install',
  title: 'Installation & Updates',
  order: 220,
  screens: [
    {
      id: 'install-ways',
      title: 'Installationswege',
      description:
        'Alle Wege je Plattform mit exaktem Befehl: Homebrew, Installer-Skript, PowerShell, winget, Scoop, deb/rpm, cargo binstall, Desktop-Bundles und – für Team-Server – Container, compose und Helm. Rechts das Release mit Prüfsummen und Signaturen.',
      features: [
        'DIST-001', 'DIST-002', 'DIST-003', 'DIST-004', 'DIST-005', 'DIST-006', 'DIST-007', 'DIST-008',
        'DIST-009', 'DIST-010', 'DIST-011', 'DIST-012', 'DIST-013', 'DIST-020', 'RUN-014',
      ],
      frame: 'none',
      states: [
        { id: 'macos', title: 'macOS' },
        { id: 'linux', title: 'Linux' },
        { id: 'windows', title: 'Windows' },
        { id: 'container', title: 'Server & Container' },
      ],
      component: InstallWays,
    },
    {
      id: 'install-verify',
      title: 'Installer & Signaturprüfung',
      description: 'Das Installer-Skript prüft Prüfsumme und Signatur und fragt vor PATH-Änderungen. Wer selbst prüfen will: SHA256SUMS, cosign, SLSA-Provenance, Notarisierung.',
      features: ['DIST-002', 'DIST-003', 'DIST-013'],
      frame: 'terminal',
      states: [
        { id: 'installer', title: 'install.sh' },
        { id: 'verify', title: 'Selbst prüfen' },
        { id: 'fail', title: 'Prüfung schlägt fehl' },
      ],
      component: InstallVerify,
    },
    {
      id: 'install-update',
      title: 'Updates',
      description:
        'Nur auf Klick: Suche, Kanal stable/beta/nightly, Update aus Datei ohne Netz. Ein bereitstehendes Update zeigt den Changelog mit Feature-IDs und wartet auf laufende Turns.',
      features: ['DIST-004', 'DIST-014', 'DIST-016', 'DIST-017', 'DIST-019'],
      states: [
        { id: 'idle', title: 'Noch nicht gesucht' },
        { id: 'available', title: 'Update bereit' },
        { id: 'draining', title: 'Wartet auf Turns' },
        { id: 'file', title: 'Aus Datei' },
        { id: 'invalid', title: 'Signatur ungültig' },
        { id: 'brew', title: 'Über Homebrew' },
      ],
      component: UpdateScreen,
    },
    {
      id: 'install-compat',
      title: 'Versionen & Kompatibilität',
      description: 'Welche Komponenten miteinander sprechen dürfen (Minor-Differenz ≤ 1), was bei zu großem Abstand passiert und warum ein Downgrade nach einer Migration nicht geht.',
      features: ['DIST-018'],
      states: [
        { id: 'ok', title: 'Alles kompatibel' },
        { id: 'refused', title: 'Server zu alt' },
        { id: 'downgrade', title: 'Downgrade verweigert' },
      ],
      component: CompatScreen,
    },
  ],
}
