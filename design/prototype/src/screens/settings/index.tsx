import type { ReactNode } from 'react'
import { Check, FileJson, FolderOpen, KeyRound, Lock, Plus, RotateCcw, Server, TerminalSquare, Trash2, Upload } from 'lucide-react'
import { Checkbox } from '@/components/ui/checkbox'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { Btn, NetNote, Overlay, PageHeader, ProblemBox, Row, SectionTitle, Segmented, SettingsShell } from './shell'

/* ───────────────────────── Darstellung (UX-005, WEB-016, WEB-017) ───────────────────────── */

const appearanceText = {
  de: {
    title: 'Darstellung',
    intro: 'Gilt nur für dieses Gerät. Themes wirken sofort – auch auf Editor, Terminal und Code-Blöcke.',
    theme: 'Theme',
    language: 'Sprache',
    languageHint: 'Wechsel wirkt sofort, ohne neu zu laden. Die Kommandozeile (beton, TUI) bleibt in v1 auf Englisch.',
    auto: 'Automatisch',
    formats: 'Formate',
    formatsHint: 'Datum, Zahlen und Währung folgen der Sprache.',
    a11y: 'Barrierefreiheit',
    motion: 'Bewegung reduzieren',
    motionHint: 'Schaltet Animationen wie pulsierende Statuspunkte und Übergänge ab.',
    system: 'Wie System',
    on: 'An',
    off: 'Aus',
    contrast: 'Hoher Kontrast',
    contrastHint: 'Wechselt auf das Theme „Hoher Kontrast“ (WCAG AAA für Fließtext).',
    focus: 'Fokus immer deutlich zeigen',
    focusHint: 'Gelber Fokusrahmen auch bei Mausbedienung.',
    live: 'Streaming für Screenreader ansagen',
    liveHint: 'Liest nur abgeschlossene Absätze vor, höchstens alle 2 Sekunden.',
    keyboard: 'Tastaturbedienung',
    keyboardHint: 'Alle Kernabläufe – Session öffnen, Nachricht senden, Freigabe entscheiden, Diff lesen – gehen ohne Maus.',
    showKeys: 'Tastenkürzel anzeigen',
    size: 'Schriftgröße',
  },
  en: {
    title: 'Appearance',
    intro: 'Applies to this device only. Themes take effect immediately – including editor, terminal and code blocks.',
    theme: 'Theme',
    language: 'Language',
    languageHint: 'Switching takes effect immediately, without a reload. The command line (beton, TUI) stays English in v1.',
    auto: 'Automatic',
    formats: 'Formats',
    formatsHint: 'Dates, numbers and currency follow the language.',
    a11y: 'Accessibility',
    motion: 'Reduce motion',
    motionHint: 'Turns off animations such as pulsing status dots and transitions.',
    system: 'Like system',
    on: 'On',
    off: 'Off',
    contrast: 'High contrast',
    contrastHint: 'Switches to the “High contrast” theme (WCAG AAA for body text).',
    focus: 'Always show focus clearly',
    focusHint: 'Yellow focus ring even when using the mouse.',
    live: 'Announce streaming to screen readers',
    liveHint: 'Reads finished paragraphs only, at most every 2 seconds.',
    keyboard: 'Keyboard',
    keyboardHint: 'All core flows – open a session, send a message, decide an approval, read a diff – work without a mouse.',
    showKeys: 'Show shortcuts',
    size: 'Font size',
  },
}

function ThemeTile({ name, note, selected, className, split }: { name: string; note: string; selected?: boolean; className?: string; split?: boolean }) {
  const mini = (
    <div className="flex h-full bg-background">
      <div className="w-3 border-r border-border bg-sidebar" />
      <div className="flex flex-1 flex-col gap-1 p-1.5">
        <div className="h-1 w-3/5 rounded-full bg-foreground/70" />
        <div className="h-1 w-4/5 rounded-full bg-muted-foreground/50" />
        <div className="mt-auto flex gap-1">
          <span className="h-2 w-5 rounded-[2px] bg-voice-claude" />
          <span className="h-2 w-3 rounded-[2px] bg-voice-codex" />
          <span className="chamfer-sm ml-auto h-2 w-4 bg-signal" />
        </div>
      </div>
    </div>
  )
  return (
    <button
      role="radio"
      aria-checked={!!selected}
      className={cn('flex w-[112px] flex-col gap-1.5 rounded-md border p-1.5 text-left', selected ? 'border-foreground ring-1 ring-foreground' : 'border-border hover:bg-accent')}
    >
      <div className="relative h-16 overflow-hidden rounded-[3px] border border-border">
        {split ? (
          <div className="flex h-full">
            <div className="w-1/2 overflow-hidden">{mini}</div>
            <div className="dark w-1/2 overflow-hidden">{mini}</div>
          </div>
        ) : (
          <div className={cn('h-full', className)}>{mini}</div>
        )}
      </div>
      <div className="flex items-center gap-1 px-0.5 text-[12px] font-medium">
        {selected && <Check className="size-3" />}
        {name}
      </div>
      <div className="px-0.5 text-[11px] leading-tight text-muted-foreground">{note}</div>
    </button>
  )
}

function SettingsAppearance({ state }: { state: string }) {
  const en = state === 'english'
  const t = en ? appearanceText.en : appearanceText.de
  const theme = state === 'high-contrast' ? 'hc' : 'system'
  const now = new Date('2026-10-03T14:05:00')
  const loc = en ? 'en-US' : 'de-DE'
  return (
    <SettingsShell section="appearance">
      <PageHeader title={t.title}>{t.intro}</PageHeader>

      <F id="UX-005">
        <SectionTitle>{t.theme}</SectionTitle>
        <div role="radiogroup" aria-label={t.theme} className="flex flex-wrap gap-2">
          <ThemeTile name={en ? 'System' : 'System'} note={en ? 'Follows the OS, switches live' : 'Folgt dem Betriebssystem, wechselt live'} split selected={theme === 'system'} />
          <ThemeTile name={en ? 'Concrete light' : 'Sichtbeton hell'} note={en ? 'Brand theme, by day' : 'Markenthema am Tag'} />
          <ThemeTile name={en ? 'Concrete dark' : 'Sichtbeton dunkel'} note={en ? 'Brand theme, by night' : 'Markenthema bei Nacht'} className="dark" />
          <ThemeTile name="Nord" note={en ? 'Cool blue-grey' : 'Kühles Blaugrau'} className="dark saturate-[.7] hue-rotate-[12deg]" />
          <ThemeTile name={en ? 'High contrast' : 'Hoher Kontrast'} note="WCAG AAA" className="dark contrast-150" selected={theme === 'hc'} />
        </div>
      </F>

      <F id="WEB-017" className="mt-2">
        <SectionTitle>{t.language}</SectionTitle>
        <Row label={t.language} hint={t.languageHint}>
          <Segmented
            label={t.language}
            value={en ? 'en' : 'auto'}
            options={[
              { id: 'auto', label: `${t.auto} (${en ? 'German' : 'Deutsch'})` },
              { id: 'de', label: 'Deutsch' },
              { id: 'en', label: 'English' },
            ]}
          />
        </Row>
        <Row label={t.formats} hint={t.formatsHint}>
          <span className="font-mono text-[12px] text-muted-foreground tabular-nums">
            {now.toLocaleDateString(loc, { day: '2-digit', month: '2-digit', year: 'numeric' })} ·{' '}
            {now.toLocaleTimeString(loc, { hour: '2-digit', minute: '2-digit' })} · {(1234.5).toLocaleString(loc)} ·{' '}
            {(4.82).toLocaleString(loc, { style: 'currency', currency: 'USD' })}
          </span>
        </Row>
      </F>

      <F id="WEB-016" className="mt-2">
        <SectionTitle>{t.a11y}</SectionTitle>
        <Row label={t.motion} hint={t.motionHint}>
          <Segmented label={t.motion} value="system" options={[{ id: 'system', label: t.system }, { id: 'on', label: t.on }, { id: 'off', label: t.off }]} />
        </Row>
        <Row label={t.contrast} hint={t.contrastHint}>
          <Switch checked={theme === 'hc'} aria-label={t.contrast} />
        </Row>
        <Row label={t.focus} hint={t.focusHint}>
          <Switch defaultChecked aria-label={t.focus} />
        </Row>
        <Row label={t.live} hint={t.liveHint}>
          <Switch defaultChecked aria-label={t.live} />
        </Row>
        <Row label={t.size}>
          <Segmented label={t.size} value="14" options={[{ id: '13', label: '13 px' }, { id: '14', label: '14 px' }, { id: '16', label: '16 px' }]} />
        </Row>
        <Row label={t.keyboard} hint={t.keyboardHint}>
          <Btn>
            {t.showKeys} <kbd className="font-mono text-[11px] text-muted-foreground">⌘/</kbd>
          </Btn>
        </Row>
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Benachrichtigungen (UX-006, WEB-014, USE-004) ───────────────────────── */

const NOTIFY_TYPES: { id: string; label: string; hint?: string; feature?: string; defaults: [boolean, boolean, boolean, boolean]; later?: string }[] = [
  { id: 'approval_requested', label: 'Freigabe angefordert', defaults: [true, true, true, true] },
  { id: 'question', label: 'Frage des Agents', defaults: [true, true, false, true] },
  { id: 'turn_completed', label: 'Agent ist fertig', hint: 'Nur wenn das Fenster nicht im Vordergrund ist', defaults: [true, false, false, false] },
  { id: 'async_done', label: 'Hintergrund-Agent fertig oder fehlgeschlagen', defaults: [true, true, false, true], later: 'ab M5' },
  { id: 'mention', label: 'Erwähnung in geteilter Session', defaults: [true, true, false, true], later: 'ab M4' },
  { id: 'runner_lost', label: 'Runner oder Host getrennt', defaults: [true, true, false, false] },
  { id: 'budget_threshold', label: 'Budget-Schwelle erreicht', hint: 'Nur bei API-Key oder Gateway', defaults: [true, true, false, false] },
  { id: 'rate_limit_high', label: 'Subscription-Kontingent fast aufgebraucht', hint: 'Ab 90 % eines Fensters, z. B. des 5-Stunden-Fensters', feature: 'USE-004', defaults: [true, true, false, false] },
]

function SettingsNotifications({ state }: { state: string }) {
  const pushOn = state === 'push-on'
  const quiet = state === 'quiet-hours'
  const channels = ['Inbox', 'Desktop-Mitteilung', 'Ton', 'Dock-Badge', 'Web-Push']
  return (
    <SettingsShell
      section="notifications"
      overlay={
        state === 'push-dialog' ? (
          <Overlay
            title="Web-Push einrichten?"
            footer={
              <>
                <Btn primary>Im Browser erlauben</Btn>
                <Btn>Abbrechen</Btn>
                <span className="ml-auto text-[11px] text-muted-foreground">Du kannst es jederzeit wieder ausschalten.</span>
              </>
            }
          >
            <p>
              Web-Push bringt Benachrichtigungen aufs Handy oder in einen geschlossenen Browser. Dafür schickt beton eine
              verschlüsselte Nachricht an den <strong>Push-Dienst deines Browser-Herstellers</strong> (Apple, Google oder
              Mozilla), der sie zustellt.
            </p>
            <dl className="mt-3 grid grid-cols-[170px_1fr] gap-x-3 gap-y-1.5">
              <dt>
                <NetNote kind="network">Verlässt den Rechner</NetNote>
              </dt>
              <dd>Ereignistyp und Projekt, z. B. „Freigabe wartet · shop-frontend“.</dd>
              <dt>
                <NetNote kind="local">Bleibt hier</NetNote>
              </dt>
              <dd>Nachrichtentext, Befehle, Dateinamen – außer du schaltest „Vorschau in Push“ ein.</dd>
            </dl>
            <p className="mt-3 text-muted-foreground">
              Freigaben entscheidest du danach in der App, nicht auf dem Sperrbildschirm. Voraussetzung: das Handy erreicht
              diesen Rechner per LAN oder Tailscale und ist gekoppelt.
            </p>
          </Overlay>
        ) : null
      }
    >
      <PageHeader title="Benachrichtigungen">
        Standard sind lokale Mitteilungen über das Betriebssystem – nichts verlässt dafür den Rechner. Gilt für alle deine
        Geräte.
      </PageHeader>

      <F id="UX-006">
        <SectionTitle aside={<NetNote kind="local">Inbox, Desktop, Ton und Badge bleiben lokal</NetNote>}>Was dich wie erreicht</SectionTitle>
        <div className="overflow-hidden rounded-md border border-border">
          <table className="w-full text-[13px]">
            <thead className="bg-sunken text-[12px] text-muted-foreground">
              <tr>
                <th className="py-2 pl-3 text-left font-medium">Ereignis</th>
                {channels.map((c, i) => (
                  <th key={c} className={cn('w-[86px] px-1 py-2 text-center font-medium', i === 4 && 'border-l border-dashed border-border')}>
                    {c}
                    {i === 4 && <div className="text-[10px] font-normal">optional</div>}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {NOTIFY_TYPES.map((n) => {
                const label = (
                  <>
                    <div className="flex items-baseline gap-2">
                      <span>{n.label}</span>
                      {n.later && <span className="text-[11px] text-muted-foreground">{n.later}</span>}
                    </div>
                    {n.hint && <div className="text-[11px] text-muted-foreground">{n.hint}</div>}
                  </>
                )
                return (
                  <tr key={n.id} className="border-t border-border">
                    <td className="py-2 pl-3">{n.feature ? <F id={n.feature}>{label}</F> : label}</td>
                    <td className="text-center">
                      <Checkbox checked disabled aria-label={`${n.label}: Inbox (immer an)`} title="Die Inbox zeigt immer alles" />
                    </td>
                    {n.defaults.slice(1).map((d, i) => (
                      <td key={i} className="text-center">
                        <Checkbox defaultChecked={quiet && i === 1 ? false : d} aria-label={`${n.label}: ${channels[i + 1]}`} />
                      </td>
                    ))}
                    <td className="border-l border-dashed border-border text-center">
                      <Checkbox disabled={!pushOn} defaultChecked={pushOn && n.defaults[3]} aria-label={`${n.label}: Web-Push`} />
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>

        <SectionTitle>Ruhezeiten</SectionTitle>
        <Row label="Ruhezeiten" hint="In dieser Zeit bleiben Desktop-Mitteilung und Ton stumm; die Inbox sammelt weiter.">
          <Switch checked={quiet} aria-label="Ruhezeiten" />
        </Row>
        {quiet && (
          <>
            <Row label="Zeitraum" hint="Jeden Tag, in deiner Zeitzone (Europe/Berlin)">
              <span className="rounded-md border border-input bg-card px-2 py-1 font-mono text-[12px]">22:00</span>
              <span className="text-muted-foreground">bis</span>
              <span className="rounded-md border border-input bg-card px-2 py-1 font-mono text-[12px]">07:00</span>
            </Row>
            <Row label="Freigaben trotzdem melden" hint="Ein wartender Agent soll dich auch nachts erreichen.">
              <Switch defaultChecked aria-label="Freigaben trotzdem melden" />
            </Row>
          </>
        )}
      </F>

      <F id="WEB-014" className="mt-2">
        <SectionTitle aside={<NetNote kind="network">Nutzt den Push-Dienst des Browser-Herstellers</NetNote>}>Web-Push (optional)</SectionTitle>
        {!pushOn ? (
          <Row
            label="Web-Push ist aus"
            hint="Für Benachrichtigungen auf dem Handy, wenn die beton-App dort geschlossen ist. Läuft über Apple, Google oder Mozilla – deshalb nur auf Wunsch."
          >
            <Btn>Web-Push einrichten …</Btn>
          </Row>
        ) : (
          <>
            <div className="divide-y divide-border rounded-md border border-border">
              {[
                { d: 'iPhone von Ingo', b: 'Safari · Home-Bildschirm-App', s: 'Apple Push', when: 'zuletzt zugestellt vor 12 Min.' },
                { d: 'Pixel 8 (Arbeit)', b: 'Chrome · installierte App', s: 'Google FCM', when: 'zuletzt zugestellt gestern' },
              ].map((x) => (
                <div key={x.d} className="flex items-center gap-3 px-3 py-2 text-[13px]">
                  <span className="size-2 rounded-full bg-ok" aria-hidden />
                  <div className="min-w-0 flex-1">
                    <div className="font-medium">{x.d}</div>
                    <div className="text-[11px] text-muted-foreground">
                      {x.b} · über {x.s} · {x.when}
                    </div>
                  </div>
                  <Btn>Entfernen</Btn>
                </div>
              ))}
            </div>
            <Row label="Vorschau in Push" hint="Zeigt Befehl oder Frage im Push-Text. Aus: nur Ereignistyp und Projekt." className="mt-2">
              <Switch aria-label="Vorschau in Push" />
            </Row>
            <Row label="Abgelaufene Geräte" hint="Meldet der Push-Dienst ein Gerät als ungültig, entfernt beton es automatisch." />
          </>
        )}
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── MCP-Server (UX-010) ───────────────────────── */

type Mcp = { name: string; scope: string; transport: 'stdio' | 'HTTP'; target: string; status: 'ok' | 'failed' | 'off' | 'untested'; tools?: number; secret?: string }

const MCP: Mcp[] = [
  { name: 'github', scope: 'Benutzer', transport: 'stdio', target: 'github-mcp-server stdio', status: 'ok', tools: 26, secret: 'GITHUB_PERSONAL_ACCESS_TOKEN → secret://user/github.com/ifahrentholz' },
  { name: 'postgres-dev', scope: 'Projekt shop-frontend', transport: 'stdio', target: 'uvx mcp-server-postgres', status: 'ok', tools: 4, secret: 'DATABASE_URL → secret://user/custom:shop-db/dev' },
  { name: 'docs-intern', scope: 'Benutzer', transport: 'HTTP', target: 'http://localhost:8931/mcp', status: 'off' },
  { name: 'sentry', scope: 'Projekt shop-frontend', transport: 'HTTP', target: 'https://mcp.sentry.dev/mcp', status: 'failed', secret: 'Authorization → Bearer secret://user/custom:sentry/token' },
]

function McpStatus({ s }: { s: Mcp['status'] }) {
  if (s === 'ok') return <span className="inline-flex items-center gap-1.5 text-[12px]"><span className="size-2 rounded-full bg-ok" />erreichbar</span>
  if (s === 'failed') return <span className="inline-flex items-center gap-1.5 text-[12px] text-deny"><span className="size-2 rotate-45 bg-deny" />Fehler</span>
  if (s === 'off') return <span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground"><span className="size-2 rounded-[1px] bg-muted-foreground/50" />deaktiviert</span>
  return <span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground"><span className="size-2 rounded-full border border-muted-foreground" />nicht getestet</span>
}

function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: ReactNode }) {
  return (
    <label className="grid grid-cols-[150px_minmax(0,1fr)] items-start gap-3 py-1.5">
      <span className="pt-1.5 text-[13px] text-muted-foreground">{label}</span>
      <span>
        {children}
        {hint && <span className="mt-1 block text-[11px] text-muted-foreground">{hint}</span>}
      </span>
    </label>
  )
}

function FakeInput({ value, mono, placeholder, className }: { value?: string; mono?: boolean; placeholder?: string; className?: string }) {
  return (
    <span className={cn('flex h-8 items-center rounded-md border border-input bg-card px-2 text-[13px]', mono && 'font-mono text-[12px]', !value && 'text-muted-foreground', className)}>
      {value ?? placeholder}
    </span>
  )
}

function McpAddForm({ state }: { state: string }) {
  return (
    <div className="mt-2 rounded-md border border-border bg-card p-4">
      <div className="mb-2 flex items-center gap-2">
        <h3 className="text-[14px] font-semibold">MCP-Server hinzufügen</h3>
        <span className="ml-auto text-[11px] text-muted-foreground">
          wird gespeichert in <code className="font-mono">~/.beton/mcp.yaml</code>
        </span>
      </div>
      <Field label="Name">
        <FakeInput value="github" mono />
      </Field>
      <Field label="Ebene" hint="Projekt-Server ersetzen gleichnamige Benutzer-Server. Agent-eigene Server stehen im Agent-YAML.">
        <Segmented label="Ebene" value="user" options={[{ id: 'user', label: 'Benutzer (alle Projekte)' }, { id: 'project', label: 'Projekt shop-frontend' }]} />
      </Field>
      <Field label="Verbindung">
        <Segmented label="Transport" value="stdio" options={[{ id: 'stdio', label: 'Lokales Programm (stdio)' }, { id: 'http', label: 'HTTP-URL' }]} />
      </Field>
      <Field label="Kommando" hint="Läuft in der Sandbox der Session, nicht mit deinen vollen Rechten.">
        <FakeInput value={state === 'test-failed' ? 'npx -y @modelcontextprotocol/server-gihub' : 'github-mcp-server'} mono />
      </Field>
      <Field label="Argumente">
        <FakeInput value={state === 'test-failed' ? '' : 'stdio'} mono placeholder="keine" />
      </Field>
      <Field label="Umgebung">
        <div className="space-y-1.5">
          <div className="flex items-center gap-2">
            <FakeInput value="GITHUB_PERSONAL_ACCESS_TOKEN" mono className="w-60 shrink-0" />
            <span className="flex h-8 min-w-0 flex-1 items-center gap-1.5 rounded-md border border-input bg-sunken px-2 font-mono text-[12px]" title="Wert liegt im Schlüsselbund">
              <Lock className="size-3 shrink-0 text-muted-foreground" />
              <span className="truncate">secret://user/github.com/ifahrentholz</span>
            </span>
          </div>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[12px]">
            <Checkbox checked aria-label="Als Secret speichern" />
            <span>Als Secret im Schlüsselbund speichern</span>
            <span className="text-muted-foreground">– in der Datei steht nur der Verweis, nie der Wert.</span>
          </div>
          <button className="inline-flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground">
            <Plus className="size-3" /> Variable hinzufügen
          </button>
        </div>
      </Field>
      <Field label="Erlaubte Tools" hint="Leer = alle Tools des Servers.">
        <FakeInput placeholder="z. B. get_pull_request, create_pull_request" mono />
      </Field>

      {state === 'test-ok' && (
        <div className="mt-3 rounded-md border border-ok/40 bg-ok-soft p-3 text-[13px]">
          <div className="flex items-center gap-2 font-semibold">
            <Check className="size-4 text-ok" /> Verbunden – 26 Tools in 1,4 s
          </div>
          <div className="mt-2 flex flex-wrap gap-1 font-mono text-[11.5px]">
            {['get_pull_request', 'create_pull_request', 'list_check_runs', 'get_file_contents', 'search_code', 'create_issue', 'list_issues', 'add_issue_comment', 'merge_pull_request', 'list_commits'].map((t) => (
              <span key={t} className="rounded-sm border border-border bg-card px-1.5 py-0.5">
                {t}
              </span>
            ))}
            <span className="px-1.5 py-0.5 text-muted-foreground">+ 16 weitere</span>
          </div>
        </div>
      )}
      {state === 'test-failed' && (
        <div className="mt-3 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
          <div className="flex items-center gap-2 font-semibold">
            <span className="size-2 rotate-45 bg-deny" aria-hidden /> Server ist nicht gestartet (Exit-Code 1)
          </div>
          <p className="mt-1">Das Paket wurde nicht gefunden. Prüfe den Namen im Kommando – vermutlich ein Tippfehler („gihub“).</p>
          <pre className="mt-2 overflow-x-auto rounded-sm bg-card px-2 py-1.5 font-mono text-[11.5px] leading-relaxed">{`stderr:
npm error code E404
npm error 404 Not Found - GET https://registry.npmjs.org/@modelcontextprotocol%2fserver-gihub
npm error 404  '@modelcontextprotocol/server-gihub@*' is not in this registry.
env: GITHUB_PERSONAL_ACCESS_TOKEN=[REDACTED:secret]`}</pre>
        </div>
      )}

      <div className="mt-4 flex items-center gap-2">
        <Btn>
          <TerminalSquare className="size-3.5" /> Verbindung testen
        </Btn>
        <Btn primary disabled={state === 'test-failed'}>Server hinzufügen</Btn>
        <Btn className="ml-auto">Abbrechen</Btn>
      </div>
    </div>
  )
}

function SettingsMcp({ state }: { state: string }) {
  const form = state !== 'list'
  return (
    <SettingsShell section="mcp">
      <PageHeader
        title="MCP-Server"
        actions={
          !form && (
            <Btn>
              <Plus className="size-3.5" /> Server hinzufügen
            </Btn>
          )
        }
      >
        Werkzeuge, die deine Agents nutzen dürfen. beton reicht sie an Claude Code, Codex und ACP-Agents weiter. Zugangsdaten
        liegen im Schlüsselbund, in den Dateien stehen nur Verweise.
      </PageHeader>
      <F id="UX-010">
        {form ? (
          <McpAddForm state={state} />
        ) : (
          <>
            <div className="overflow-hidden rounded-md border border-border">
              <table className="w-full text-[13px]">
                <thead className="bg-sunken text-[12px] text-muted-foreground">
                  <tr>
                    <th className="py-2 pl-3 text-left font-medium">Server</th>
                    <th className="text-left font-medium">Ebene</th>
                    <th className="text-left font-medium">Status</th>
                    <th className="pr-3 text-right font-medium">Aktiv</th>
                  </tr>
                </thead>
                <tbody>
                  {MCP.map((m) => (
                    <tr key={m.name} className="border-t border-border align-top">
                      <td className="py-2 pl-3">
                        <div className="flex items-center gap-2">
                          <Server className="size-3.5 text-muted-foreground" />
                          <span className="font-medium">{m.name}</span>
                          <span className="rounded-sm border border-border px-1 text-[10px] text-muted-foreground">{m.transport}</span>
                        </div>
                        <div className="mt-0.5 font-mono text-[11px] text-muted-foreground">{m.target}</div>
                        {m.secret && (
                          <div className="mt-0.5 flex items-center gap-1 font-mono text-[11px] text-muted-foreground">
                            <KeyRound className="size-3" /> {m.secret}
                          </div>
                        )}
                        {m.status === 'failed' && (
                          <div className="mt-1 text-[12px] text-deny">401 vom Server – das Token ist abgelaufen. Neues Token im Schlüsselbund hinterlegen.</div>
                        )}
                      </td>
                      <td className="py-2 text-[12px] text-muted-foreground">{m.scope}</td>
                      <td className="py-2">
                        <McpStatus s={m.status} />
                        {m.tools !== undefined && <div className="text-[11px] text-muted-foreground">{m.tools} Tools</div>}
                      </td>
                      <td className="py-2 pr-3 text-right">
                        <Switch checked={m.status !== 'off'} aria-label={`${m.name} aktiv`} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="mt-3 text-[12px] text-muted-foreground">
              Auch per Kommandozeile: <code className="font-mono">beton mcp list | add | remove | test</code>
            </p>
          </>
        )}
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Datenschutz (OBS-007, OBS-008) ───────────────────────── */

const TELEMETRY_SENT = [
  'Zufällige Installations-ID (jederzeit neu würfelbar)',
  'beton-Version, Update-Kanal, Installationsart',
  'Betriebssystem mit Hauptversion, CPU-Architektur',
  'Modus (lokal oder Team-Server) und Client (Desktop, Web, CLI)',
  'Pro Tag: gestartete Sessions je Harness-Art (claude, codex, acp, direct, plugin)',
  'Pro Tag: genutzte Funktionen ja/nein (Sprache, Browser, Fork, Worktree, Policies, Sandbox-Art)',
  'Pro Tag: Anzahl Freigaben (nur die Zahl)',
]
const TELEMETRY_NEVER = ['Prompts und Antworten', 'Tool-Argumente, Befehle', 'Datei- und Repo-Namen, Pfade', 'Modellnamen von Gateways', 'Hostname, E-Mail, IP-Adresse']

function TelemetryConsent() {
  return (
    <Overlay
      title="Anonyme Nutzungsstatistik senden?"
      width="max-w-2xl"
      footer={
        <>
          <Btn>Nicht senden</Btn>
          <Btn>Anonym senden</Btn>
          <span className="ml-auto text-[11px] text-muted-foreground">Keine Vorauswahl. Schließen heißt: bleibt aus.</span>
        </>
      }
    >
      <p>
        Hilft dem beton-Projekt zu sehen, welche Harnesses und Funktionen genutzt werden. Gesendet wird höchstens einmal am Tag
        per HTTPS – und nur genau diese Liste:
      </p>
      <div className="mt-3 grid grid-cols-[1fr_200px] gap-6">
        <ul className="space-y-1">
          {TELEMETRY_SENT.map((x) => (
            <li key={x} className="flex gap-2">
              <Check className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
              {x}
            </li>
          ))}
        </ul>
        <div>
          <div className="text-[12px] font-medium">Nie dabei</div>
          <ul className="mt-1 space-y-1 text-muted-foreground">
            {TELEMETRY_NEVER.map((x) => (
              <li key={x}>– {x}</li>
            ))}
          </ul>
        </div>
      </div>
      <p className="mt-3 text-muted-foreground">
        Das nächste Paket kannst du vor dem Versand vollständig ansehen. <code className="font-mono">DO_NOT_TRACK=1</code> und
        CI-Umgebungen schalten die Statistik immer ab.
      </p>
    </Overlay>
  )
}

function SettingsPrivacy({ state }: { state: string }) {
  const dnt = state === 'dnt'
  return (
    <SettingsShell section="privacy" overlay={state === 'consent' ? <TelemetryConsent /> : null}>
      <PageHeader title="Datenschutz">
        beton läuft vollständig auf diesem Rechner. Ins Netz gehen nur die Anfragen der Vendor-CLIs an ihre Modell-Anbieter – alles
        andere nur, wenn du es einschaltest.
      </PageHeader>

      {state === 'crash' && (
        <F id="OBS-008" className="chamfer mb-5 border-l-4 border-signal bg-signal-soft p-3 text-[13px]">
          <div className="font-semibold">Der beton-Daemon ist gestern um 18:42 abgestürzt.</div>
          <p className="mt-0.5">
            Ein Bericht liegt lokal in <code className="font-mono">~/.beton/crashes/</code>. Er enthält keine Prompts, Umgebungsvariablen,
            Argumente oder Pfade. Senden hilft, den Fehler zu beheben.
          </p>
          <pre className="mt-2 max-h-28 overflow-auto rounded-sm bg-card px-2 py-1.5 font-mono text-[11.5px] leading-relaxed">{`beton-daemon 0.9.2 · macos 15 aarch64 · component=policy
panicked at crates/beton-policy/src/eval.rs:212:18:
called \`Option::unwrap()\` on a \`None\` value
   0: beton_policy::eval::Evaluator::match_rule
   1: beton_policy::eval::Evaluator::evaluate
   2: beton_server::session::turn::on_tool_requested`}</pre>
          <div className="mt-2 flex gap-2">
            <Btn primary>Diesen Bericht senden</Btn>
            <Btn>Verwerfen</Btn>
            <span className="ml-auto self-center text-[11px] text-muted-foreground">Ohne deine Bestätigung wird nichts gesendet.</span>
          </div>
        </F>
      )}

      {dnt && (
        <div className="mb-5 flex items-start gap-2 rounded-md border border-border bg-card p-3 text-[13px]">
          <Lock className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          <div>
            <div className="font-medium">
              <code className="font-mono">DO_NOT_TRACK=1</code> ist gesetzt.
            </div>
            <div className="text-muted-foreground">Nutzungsstatistik und Crash-Reports bleiben aus – unabhängig von den Schaltern unten.</div>
          </div>
        </div>
      )}

      <SectionTitle>Was diesen Rechner verlassen darf</SectionTitle>
      <div className="divide-y divide-border rounded-md border border-border text-[13px]">
        {[
          { what: 'Anfragen der Vendor-CLIs an Anthropic, OpenAI, Google', how: 'Durch die CLIs selbst, mit deinem Abo-Login', on: true, fixed: true },
          { what: 'Nach Updates suchen', how: 'Abfrage der Release-Liste, einmal am Tag', on: false },
          { what: 'Whisper-Modell herunterladen', how: 'Nur wenn du Spracheingabe einrichtest', on: false },
          { what: 'Nutzungsstatistik', how: 'Anonym, einmal am Tag', on: false, f: 'OBS-007' },
          { what: 'Crash-Reports', how: 'Nur nach Zustimmung', on: false, f: 'OBS-008' },
          { what: 'Web-Push', how: 'Über den Push-Dienst des Browser-Herstellers', on: false },
        ].map((x) => (
          <div key={x.what} className="flex items-center gap-3 px-3 py-2">
            {x.on ? <NetNote kind="network">an</NetNote> : <NetNote kind="local">aus</NetNote>}
            <div className="min-w-0 flex-1">
              <div>{x.what}</div>
              <div className="text-[11px] text-muted-foreground">{x.how}</div>
            </div>
            {x.fixed && <span className="text-[11px] text-muted-foreground">nötig für Agents</span>}
          </div>
        ))}
      </div>

      <F id="OBS-007" className="mt-2">
        <SectionTitle aside="Standard: aus">Nutzungsstatistik</SectionTitle>
        <Row
          label="Anonyme Nutzungsstatistik senden"
          hint={dnt ? 'Durch DO_NOT_TRACK abgeschaltet.' : 'Bei der Einrichtung übersprungen – deshalb aus. Keine Inhalte, nur Zähler.'}
        >
          <Switch checked={false} disabled={dnt} aria-label="Nutzungsstatistik senden" />
        </Row>
        <Row label="Was gesendet würde" hint="Zeigt das nächste Paket vollständig, bevor irgendetwas rausgeht.">
          <Btn>Datenliste ansehen</Btn>
          <Btn>Nächstes Paket zeigen</Btn>
        </Row>
        <Row label="Installations-ID" hint="Zufällig erzeugt, ohne Bezug zu dir. Ausschalten löscht sie und alle ungesendeten Pakete.">
          <Btn>
            <RotateCcw className="size-3.5" /> Neu erzeugen
          </Btn>
        </Row>
      </F>

      <F id="OBS-008" className="mt-2">
        <SectionTitle aside="Standard: aus">Crash-Reports</SectionTitle>
        <Row label="Crash-Reports automatisch senden" hint="Berichte werden immer lokal gespeichert. Ohne diese Zustimmung fragt beton bei jedem Bericht einzeln.">
          <Switch checked={false} disabled={dnt} aria-label="Crash-Reports automatisch senden" />
        </Row>
        <Row label="Lokale Berichte" hint="1 Bericht in ~/.beton/crashes/ (4 KB)">
          <Btn>
            <FolderOpen className="size-3.5" /> Ordner öffnen
          </Btn>
        </Row>
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Sessions & Daten (DATA-008, DATA-009, DATA-010, PROTO-011) ───────────────────────── */

const ARCHIVED = [
  { title: 'Terraform-Plan erklären', project: 'infra', when: 'archiviert vor 3 Tagen', size: '8,2 MB', events: 1284 },
  { title: 'Spike: Bun statt Node', project: 'shop-frontend', when: 'archiviert vor 5 Wochen', size: '21,4 MB', events: 4410 },
  { title: 'Lokales Modell testen', project: 'beton', when: 'archiviert vor 2 Monaten', size: '0,9 MB', events: 212 },
]

function SettingsData({ state }: { state: string }) {
  return (
    <SettingsShell
      section="data"
      overlay={
        state === 'delete-confirm' ? (
          <Overlay
            title="„Terraform-Plan erklären“ endgültig löschen?"
            footer={
              <>
                <Btn danger>
                  <Trash2 className="size-3.5" /> Endgültig löschen
                </Btn>
                <Btn>Stattdessen archiviert lassen</Btn>
              </>
            }
          >
            <p>Das lässt sich nicht rückgängig machen. Gelöscht werden:</p>
            <ul className="mt-2 space-y-1">
              <li>– der Verlauf mit 1 284 Ereignissen und die Roh-Ausgaben des Harness</li>
              <li>– 2 Side-Chats und 1 Sub-Session</li>
              <li>– 14 Anhänge und Snapshots (8,2 MB) – sie verschwinden beim nächsten Aufräumen</li>
            </ul>
            <p className="mt-3 text-muted-foreground">
              Ein Löschvermerk bleibt zurück, damit andere Geräte die Session nicht wieder herstellen. Der Git-Branch im Repo bleibt
              unangetastet.
            </p>
          </Overlay>
        ) : null
      }
    >
      <PageHeader title="Sessions & Daten">Alle Daten liegen in <code className="font-mono">~/.beton/</code> auf diesem Rechner.</PageHeader>

      <F id="DATA-008">
        <SectionTitle aside="Archivieren ist umkehrbar, Löschen nicht">Archivierte Sessions</SectionTitle>
        <div className="divide-y divide-border rounded-md border border-border">
          {ARCHIVED.map((a) => (
            <div key={a.title} className="flex items-center gap-3 px-3 py-2 text-[13px]">
              <div className="min-w-0 flex-1">
                <div className="font-medium">{a.title}</div>
                <div className="text-[11px] text-muted-foreground">
                  {a.project} · {a.when} · {a.events.toLocaleString('de-DE')} Ereignisse · {a.size}
                </div>
              </div>
              <Btn>Wiederherstellen</Btn>
              <Btn danger>Löschen …</Btn>
            </div>
          ))}
        </div>
      </F>

      <F id="DATA-009" className="mt-2">
        <SectionTitle aside="Täglich um 03:00 · zuletzt: 412 Roh-Ausgaben, 37 Snapshots entfernt">Aufbewahrung</SectionTitle>
        <Row label="Sessions" hint="Aktive Sessions bleiben, bis du sie löschst.">
          <Segmented label="Sessions" value="inf" options={[{ id: 'inf', label: 'Unbegrenzt' }, { id: '365', label: '1 Jahr' }]} />
        </Row>
        <Row label="Archivierte Sessions" hint="Danach endgültig gelöscht, wie beim manuellen Löschen.">
          <Segmented label="Archivierte Sessions" value="180" options={[{ id: 'inf', label: 'Unbegrenzt' }, { id: '180', label: '180 Tage' }, { id: '30', label: '30 Tage' }]} />
        </Row>
        <Row label="Roh-Ausgaben der Harnesses" hint="Originalereignisse der Vendor-CLIs, für Fehlersuche. Der Verlauf selbst bleibt erhalten.">
          <Segmented label="Roh-Ausgaben" value="30" options={[{ id: '7', label: '7 Tage' }, { id: '30', label: '30 Tage' }, { id: 'off', label: 'Nicht speichern' }]} />
        </Row>
        <Row label="Terminal- und Browser-Snapshots" hint="Bildschirmstände und Screenshots.">
          <Segmented label="Snapshots" value="90" options={[{ id: '30', label: '30 Tage' }, { id: '90', label: '90 Tage' }]} />
        </Row>
        <Row label="Abweichend für Projekte" hint="Projekte dürfen nur kürzer aufbewahren.">
          <span className="text-[12px] text-muted-foreground">shop-frontend: Roh-Ausgaben 7 Tage</span>
        </Row>
      </F>

      <F id="DATA-010" className="mt-2">
        <SectionTitle>Export & Import</SectionTitle>
        <Row label="Session exportieren" hint="Als JSONL (ein Ereignis pro Zeile) oder mit Anhängen als .tar.zst. Inhalte sind redigiert.">
          <Btn>
            <FileJson className="size-3.5" /> Session wählen …
          </Btn>
        </Row>
        <Row label="Roh-Ausgaben mitexportieren" hint="Ebenfalls redigiert. Nur für Fehlersuche nötig.">
          <Switch aria-label="Roh-Ausgaben mitexportieren" />
        </Row>
        <div className="mt-2 flex items-center justify-center gap-2 rounded-md border border-dashed border-input py-5 text-[13px] text-muted-foreground">
          <Upload className="size-4" /> Export-Datei hierher ziehen (.jsonl oder .tar.zst), um sie als neue Session zu importieren
        </div>
        {state === 'import-error' && (
          <F id="PROTO-011" className="mt-3">
            <ProblemBox
              open
              title="Import abgelehnt: Lücke im Verlauf"
              detail={
                <>
                  In <code className="font-mono">rate-limiter-7f3k.jsonl</code> folgt in Zeile 419 auf Ereignis 417 direkt 419 – Ereignis 418 fehlt.
                  Es wurde nichts importiert.
                </>
              }
              next="Exportiere die Session erneut aus der Quelle; bearbeitete Export-Dateien lassen sich nicht importieren."
              problem={{
                type: 'urn:beton:problem:import_seq_gap',
                status: 422,
                code: 'import_seq_gap',
                instance: '/v1/sessions/import',
                trace_id: '4bf92f3577b34da6a3ce929d0e0e4736',
                errors: [{ pointer: '/lines/419/seq', detail: 'expected 418, got 419' }],
              }}
            />
          </F>
        )}
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Experimentelle Funktionen (UX-007) ───────────────────────── */

const FLAGS = [
  { id: 'voice', label: 'Spracheingabe', status: 'beta', on: true, src: 'config' },
  { id: 'browser', label: 'Eingebauter Browser', status: 'beta', on: true, src: 'config' },
  { id: 'async_agents', label: 'Hintergrund-Agents', status: 'experimental', on: false, src: '' },
  { id: 'team_sync', label: 'Mit Team-Server synchronisieren', status: 'experimental', on: false, src: '' },
  { id: 'score_view', label: 'Partitur-Ansicht für Multi-Agent-Läufe', status: 'stable', on: true, src: 'immer an' },
  { id: 'legacy_pty', label: 'Altes PTY-Backend', status: 'removed', on: false, src: '' },
]
const flagStatus: Record<string, string> = { experimental: 'experimentell', beta: 'Beta', stable: 'stabil', removed: 'entfernt' }

function SettingsFlags({ state }: { state: string }) {
  return (
    <SettingsShell section="flags">
      <PageHeader title="Experimentelle Funktionen">
        Unfertige Funktionen sind ausgeblendet, bis du sie einschaltest. Sie können sich ändern oder wieder verschwinden.
      </PageHeader>
      <F id="UX-007">
        {state === 'unknown' && (
          <div className="mb-4 flex items-start gap-2 rounded-md border border-border bg-card p-3 text-[13px]">
            <span className="mt-1 size-2 shrink-0 rotate-45 border border-foreground" aria-hidden />
            <div>
              <div className="font-medium">
                Unbekannte Funktion <code className="font-mono">brwoser</code> in <code className="font-mono">BETON_FEATURES</code> ignoriert
              </div>
              <div className="text-muted-foreground">Meintest du „browser“? beton ist trotzdem normal gestartet.</div>
            </div>
          </div>
        )}
        <div className="overflow-hidden rounded-md border border-border">
          <table className="w-full text-[13px]">
            <thead className="bg-sunken text-[12px] text-muted-foreground">
              <tr>
                <th className="py-2 pl-3 text-left font-medium">Funktion</th>
                <th className="text-left font-medium">Reife</th>
                <th className="text-left font-medium">Gesetzt über</th>
                <th className="pr-3 text-right font-medium">An</th>
              </tr>
            </thead>
            <tbody>
              {FLAGS.map((f) => (
                <tr key={f.id} className={cn('border-t border-border', f.status === 'removed' && 'text-muted-foreground')}>
                  <td className="py-2 pl-3">
                    <div>{f.label}</div>
                    <div className="font-mono text-[11px] text-muted-foreground">{f.id}</div>
                  </td>
                  <td className="text-[12px]">{flagStatus[f.status]}</td>
                  <td className="text-[12px] text-muted-foreground">
                    {state === 'unknown' && f.id === 'browser' ? 'BETON_FEATURES (Umgebung)' : f.src === 'config' ? '~/.beton/config.toml' : f.src || '—'}
                  </td>
                  <td className="pr-3 text-right">
                    {f.status === 'removed' ? (
                      <span className="text-[11px]">wird ignoriert</span>
                    ) : (
                      <Switch checked={f.on} disabled={f.status === 'stable' || (state === 'unknown' && f.id === 'browser')} aria-label={f.label} />
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="mt-3 text-[12px] text-muted-foreground">
          Auch per <code className="font-mono">features: [voice, browser]</code> in der Config oder <code className="font-mono">BETON_FEATURES=voice,browser</code>.
          Mit Team-Server entscheidet der Server.
        </p>
      </F>
    </SettingsShell>
  )
}

export const group: ScreenGroup = {
  id: 'settings',
  title: 'Einstellungen',
  order: 130,
  screens: [
    {
      id: 'settings-appearance',
      title: 'Darstellung',
      description: 'Theme, Sprache und Barrierefreiheit. Themes und Sprache wirken sofort, ohne Neuladen.',
      features: ['UX-005', 'WEB-016', 'WEB-017'],
      states: [
        { id: 'default', title: 'Standard (System, Deutsch)' },
        { id: 'high-contrast', title: 'Hoher Kontrast' },
        { id: 'english', title: 'Englisch' },
      ],
      component: SettingsAppearance,
    },
    {
      id: 'settings-notifications',
      title: 'Benachrichtigungen',
      description:
        'Matrix Ereignis × Kanal. Standard sind lokale Desktop-Mitteilungen; Web-Push ist optional und als Netzwerkzugriff über den Push-Dienst des Browser-Herstellers beschriftet.',
      features: ['UX-006', 'WEB-014', 'USE-004'],
      states: [
        { id: 'default', title: 'Nur lokal (Standard)' },
        { id: 'push-dialog', title: 'Web-Push einrichten' },
        { id: 'push-on', title: 'Web-Push aktiv' },
        { id: 'quiet-hours', title: 'Ruhezeiten' },
      ],
      component: SettingsNotifications,
    },
    {
      id: 'settings-mcp',
      title: 'MCP-Server',
      description: 'MCP-Server für Benutzer und Projekt verwalten: hinzufügen (stdio oder HTTP), Secrets als Verweis, Verbindung testen mit Tool-Liste.',
      features: ['UX-010'],
      states: [
        { id: 'list', title: 'Übersicht' },
        { id: 'add', title: 'Hinzufügen' },
        { id: 'test-ok', title: 'Test erfolgreich' },
        { id: 'test-failed', title: 'Test fehlgeschlagen' },
      ],
      component: SettingsMcp,
    },
    {
      id: 'settings-privacy',
      title: 'Datenschutz',
      description:
        'Übersicht, was den Rechner verlassen darf; Nutzungsstatistik und Crash-Reports sind opt-in mit genauer Datenliste und ohne Vorauswahl.',
      features: ['OBS-007', 'OBS-008'],
      states: [
        { id: 'default', title: 'Alles aus (Standard)' },
        { id: 'consent', title: 'Zustimmung Statistik' },
        { id: 'crash', title: 'Crash-Bericht liegt vor' },
        { id: 'dnt', title: 'DO_NOT_TRACK gesetzt' },
      ],
      component: SettingsPrivacy,
    },
    {
      id: 'settings-data',
      title: 'Sessions & Daten',
      description: 'Archivierte Sessions wiederherstellen oder endgültig löschen, Aufbewahrungsfristen, Export und Import als JSONL.',
      features: ['DATA-008', 'DATA-009', 'DATA-010', 'PROTO-011'],
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'delete-confirm', title: 'Löschen bestätigen' },
        { id: 'import-error', title: 'Import abgelehnt' },
      ],
      component: SettingsData,
    },
    {
      id: 'settings-flags',
      title: 'Experimentelle Funktionen',
      description: 'Interne Feature-Flags mit Reifegrad und Herkunft (Config oder Umgebung). Unbekannte Flags erzeugen eine Warnung, keinen Abbruch.',
      features: ['UX-007'],
      states: [
        { id: 'default', title: 'Standard' },
        { id: 'unknown', title: 'Unbekanntes Flag' },
      ],
      component: SettingsFlags,
    },
  ],
}
