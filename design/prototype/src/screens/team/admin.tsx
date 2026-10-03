import { Bot, Check, Copy, Eye, Keyboard, MessageSquare, Plus } from 'lucide-react'
import { SessionHeader } from '@/app/session-chrome'
import { AgentMessage, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, C, Callout, InlineDialog, KV, PageHead, Premise, PrimaryButton, Scroll, Section, SettingsLayout, Shell, Tag } from '../policies/kit'

/* ------------------------------------------------------------------ Tokens & Service-Accounts */

const scopes = [
  ['sessions:read', 'Sessions und Verläufe lesen'],
  ['sessions:write', 'Sessions anlegen, Eingaben, Queue, Unterbrechen'],
  ['sessions:approve', 'Freigaben erteilen'],
  ['agents:read', 'Agents lesen'],
  ['agents:write', 'Agents ändern'],
  ['policies:read', 'Policies lesen'],
  ['usage:read', 'Verbrauch lesen'],
  ['secrets:read_meta', 'Secret-Metadaten lesen (nie Werte)'],
  ['schedules:write', 'Zeitpläne ändern'],
  ['webhooks:trigger', 'Webhooks auslösen'],
  ['audit:read', 'Audit-Log lesen'],
  ['admin', 'Verwaltung (nur mit Admin-Rolle wirksam)'],
] as const

export function Tokens({ state }: { state: string }) {
  const sa = state === 'service-accounts'
  return (
    <SettingsLayout
      active="tokens"
      connection={sa ? 'server' : 'local'}
      overlay={
        state === 'create' ? (
          <InlineDialog
            title="Zugangstoken erstellen"
            width="w-[600px]"
            footer={
              <>
                <Btn tone="quiet">Abbrechen</Btn>
                <PrimaryButton>Token erstellen</PrimaryButton>
              </>
            }
          >
            <div className="grid grid-cols-[110px_1fr] items-center gap-2">
              <span className="text-muted-foreground">Name</span>
              <span className="rounded-md border border-input bg-card px-2 py-1">release-script</span>
              <span className="text-muted-foreground">Läuft ab</span>
              <span className="flex items-center gap-2">
                <span className="rounded-md border border-input bg-card px-2 py-1">in 90 Tagen (02.01.2027)</span>
                <span className="text-[12px] text-muted-foreground">Pflicht, höchstens 365 Tage</span>
              </span>
            </div>
            <div>
              <div className="mb-1 text-muted-foreground">Rechte</div>
              <div className="grid grid-cols-2 gap-x-4 gap-y-1">
                {scopes.map(([s, d]) => {
                  const on = s === 'sessions:read' || s === 'usage:read'
                  return (
                    <label key={s} className="flex items-start gap-2 text-[12px]">
                      <span className={cn('mt-0.5 flex size-3.5 shrink-0 items-center justify-center rounded-[3px] border', on ? 'border-foreground bg-foreground text-background' : 'border-input')}>
                        {on && <Check className="size-2.5" />}
                      </span>
                      <span>
                        <span className="font-mono">{s}</span>
                        <span className="block text-[11px] text-muted-foreground">{d}</span>
                      </span>
                    </label>
                  )
                })}
              </div>
            </div>
            <p className="text-[12px] text-muted-foreground">Wirksam ist immer nur, was du selbst gerade darfst. Verlierst du Rechte, verliert sie auch das Token.</p>
          </InlineDialog>
        ) : undefined
      }
    >
      <PageHead
        title={sa ? 'Service-Accounts' : 'Zugangstokens'}
        sub={sa ? 'Nicht-menschliche Konten der Org für API- und Webhook-getriggerte Agents. Keine Anmeldung, keine Geräte.' : 'Für Skripte und das SDK, auch gegen deinen lokalen beton auf localhost.'}
        actions={
          <PrimaryButton>
            <Plus className="size-3.5" /> {sa ? 'Service-Account anlegen' : 'Token erstellen'}
          </PrimaryButton>
        }
      >
        <div className="mt-2 flex gap-1.5 text-[12px]">
          {['Persönliche Tokens', 'Service-Accounts (Admin)'].map((t, i) => (
            <span key={t} className={cn('rounded-md border px-2 py-0.5', (sa ? i === 1 : i === 0) ? 'border-foreground bg-foreground text-background' : 'border-border')}>
              {t}
            </span>
          ))}
        </div>
      </PageHead>
      <Scroll className="px-6 py-3">
        {state === 'created' && (
          <F id="AUTH-012" className="mb-4">
            <Callout tone="ok" title="Token „release-script“ erstellt – jetzt kopieren">
              <div className="flex items-center gap-2">
                <span className="flex-1 rounded-md border border-border bg-background px-2 py-1 font-mono text-[12px]">bt_pat_8Qm2vXr7…Kp41aZ</span>
                <Btn>
                  <Copy className="size-3.5" /> Kopieren
                </Btn>
              </div>
              <p className="text-muted-foreground">Es wird nur dieses eine Mal angezeigt. beton speichert nur einen Hash.</p>
            </Callout>
          </F>
        )}
        {sa ? (
          <F id={['AUTH-013', 'AUTH-014']}>
            {[
              { n: 'release-bot', d: 'Erstellt Release-Notes nach jedem Tag', role: 'member · Team plattform', tokens: '1 Token, läuft in 211 Tagen ab', sessions: '3 Sessions, per Team-Freigabe sichtbar' },
              { n: 'dependabot-fixer', d: 'Webhook: behebt Dependabot-PRs', role: 'member · Team web', tokens: '2 Tokens', sessions: '1 läuft' },
            ].map((s) => (
              <div key={s.n} className="flex items-center gap-4 border-b border-border/60 py-2 text-[12px]">
                <Bot className="size-4 text-muted-foreground" />
                <div className="min-w-0 flex-1">
                  <div className="font-mono text-[13px]">{s.n}</div>
                  <div className="text-muted-foreground">{s.d}</div>
                </div>
                <span className="w-44">{s.role}</span>
                <span className="w-48 text-muted-foreground">{s.tokens}</span>
                <span className="w-56 text-muted-foreground">{s.sessions}</span>
                <Btn tone="quiet">Token erstellen</Btn>
                <Btn tone="deny">Deaktivieren</Btn>
              </div>
            ))}
            <p className="mt-2 text-[12px] text-muted-foreground">
              Deaktivieren macht alle Tokens sofort ungültig. Laufende Sessions stoppt beton nur, wenn du es im nächsten Schritt bestätigst. Service-Accounts nutzen Secrets ihrer
              Team- bzw. Org-Bindung.
            </p>
          </F>
        ) : (
          <F id="AUTH-012">
            <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_120px_120px_90px] gap-3 border-b border-border pb-1 text-[11px] text-muted-foreground">
              <span>Name</span>
              <span>Rechte</span>
              <span>Läuft ab</span>
              <span>Zuletzt genutzt</span>
              <span />
            </div>
            {[
              { n: 'release-script', s: 'sessions:read usage:read', e: '02.01.2027', u: state === 'created' ? 'nie' : 'heute' },
              { n: 'raycast', s: 'sessions:read sessions:write sessions:approve', e: '14.11.2026', u: 'vor 2 Std.' },
              { n: 'grafana-export', s: 'usage:read', e: 'in 6 Tagen', u: 'gestern' },
            ].map((t) => (
              <div key={t.n} className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_120px_120px_90px] items-center gap-3 border-b border-border/60 py-1.5 text-[12px]">
                <span className="font-medium">{t.n}</span>
                <span className="truncate font-mono text-[11px]">{t.s}</span>
                <span className={cn(t.e.startsWith('in') ? 'text-foreground' : 'text-muted-foreground')}>{t.e}</span>
                <span className="text-muted-foreground">{t.u}</span>
                <Btn tone="deny">Widerrufen</Btn>
              </div>
            ))}
            <p className="mt-2 text-[12px] text-muted-foreground">Secret-Werte sind mit keinem Token lesbar. Gerätespezifische Rechte (Host-Tunnel, Sync) gibt es nur über das Koppeln.</p>
          </F>
        )}
      </Scroll>
    </SettingsLayout>
  )
}

/* ------------------------------------------------------------------ Mitglieder & Rollen */

const members = [
  { n: 'Ingo Fahrentholz', e: 'ingo@acme.example', role: 'owner', teams: 'plattform (owner), web (member)', via: 'Entra ID · Passkey', seen: 'jetzt' },
  { n: 'Mara Klein', e: 'mara@acme.example', role: 'admin', teams: 'web (owner)', via: 'Entra ID', seen: 'vor 5 Min.' },
  { n: 'Bob Weber', e: 'bob@acme.example', role: 'member', teams: 'plattform (member)', via: 'GitHub', seen: 'vor 2 Tagen' },
  { n: 'Lea Sommer', e: 'lea@acme.example', role: 'viewer', teams: 'web (viewer)', via: 'Google', seen: 'vor 3 Wochen' },
  { n: 'release-bot', e: 'Service-Account', role: 'member', teams: 'plattform (member)', via: '–', seen: 'vor 1 Std.' },
]

const roleMatrix: [string, string, string, string, string][] = [
  ['Org löschen, Owner übertragen', '✔', '–', '–', '–'],
  ['Anmeldeanbieter, Master-Keys, Server', '✔', '✔', '–', '–'],
  ['Mitglieder, Teams, Service-Accounts', '✔', '✔', '–', '–'],
  ['Org-Policies und Org-Secrets', '✔', '✔', '–', '–'],
  ['Audit-Log der Org', '✔', '✔', 'eigene', '–'],
  ['Sessions, Hosts, eigene Tokens', '✔', '✔', '✔', '–'],
  ['Inhalte fremder, nicht geteilter Sessions', '–', '–', '–', '–'],
  ['Metadaten aller Sessions, stoppen', '✔', '✔', '–', '–'],
]

export function Members({ state }: { state: string }) {
  return (
    <SettingsLayout
      active="members"
      connection="server"
      overlay={
        state === 'offboard' ? (
          <InlineDialog
            title="Bob Weber aus der Org entfernen?"
            width="w-[560px]"
            footer={
              <>
                <Btn tone="quiet">Abbrechen</Btn>
                <Btn tone="deny">Entfernen und alles widerrufen</Btn>
              </>
            }
          >
            <p>In einem Schritt werden widerrufen:</p>
            <ul className="ml-4 list-disc space-y-0.5 text-[13px]">
              <li>2 Web-Sitzungen, 1 Zugangstoken, 3 gekoppelte Geräte</li>
              <li>Runner-Tokens von 1 laufenden Session</li>
              <li>4 Freigaben, die Bob erteilt wurden</li>
            </ul>
            <div className="grid grid-cols-[150px_1fr] items-center gap-2">
              <span className="text-muted-foreground">Bobs 12 Sessions</span>
              <span className="rounded-md border border-input bg-card px-2 py-1">an Mara Klein übertragen</span>
            </div>
            <p className="rounded-md border border-deny/40 bg-deny-soft px-2 py-1.5 text-[12px]">
              Bobs 2 persönliche Secrets werden unlesbar gemacht (Schlüssel gelöscht). Das lässt sich nicht rückgängig machen.
            </p>
          </InlineDialog>
        ) : undefined
      }
    >
      <PageHead
        title="Mitglieder & Rollen"
        sub="Org acme · Rollen gelten für die Org und je Team; Änderungen wirken sofort, ohne neue Anmeldung."
        actions={<PrimaryButton>Einladen</PrimaryButton>}
      >
        <div className="mt-2">
          <Premise kind="team">Nur im Team-Betrieb · Konten entstehen bei der ersten Anmeldung (kein SCIM)</Premise>
        </div>
      </PageHead>
      <Scroll className="px-6 py-3">
        {state === 'last-owner' && (
          <F id={['AUTH-006', 'AUTH-014']} className="mb-3">
            <Callout tone="deny" title="Ingo ist der letzte Owner">
              <p>Eine Org braucht immer mindestens einen Owner. Ernenne zuerst jemand anderen zum Owner, dann kannst du deine Rolle ändern.</p>
            </Callout>
          </F>
        )}
        <F id={['AUTH-014', 'AUTH-016']}>
          <div className="grid grid-cols-[minmax(0,1.2fr)_110px_minmax(0,1.2fr)_130px_100px_90px] gap-3 border-b border-border pb-1 text-[11px] text-muted-foreground">
            <span>Person</span>
            <span>Org-Rolle</span>
            <span>Teams</span>
            <span>Anmeldung über</span>
            <span>Zuletzt aktiv</span>
            <span />
          </div>
          {members.map((m) => (
            <div key={m.n} className="grid grid-cols-[minmax(0,1.2fr)_110px_minmax(0,1.2fr)_130px_100px_90px] items-center gap-3 border-b border-border/60 py-1.5 text-[12px]">
              <span className="min-w-0">
                <span className="block truncate font-medium">{m.n}</span>
                <span className="block truncate text-[11px] text-muted-foreground">{m.e}</span>
              </span>
              <span>
                <span className={cn('rounded-md border px-1.5 py-0.5', state === 'last-owner' && m.role === 'owner' ? 'border-deny' : 'border-input')}>{m.role}</span>
              </span>
              <span className="truncate text-muted-foreground">{m.teams}</span>
              <span className="text-muted-foreground">{m.via}</span>
              <span className="text-muted-foreground">{m.seen}</span>
              <span className="text-right">{m.role !== 'owner' && <Btn tone="quiet">Entfernen</Btn>}</span>
            </div>
          ))}
        </F>
        <F id="AUTH-014" className="mt-5">
          <Section title="Was die Rollen dürfen" hint="Viewer sehen geteilte Sessions höchstens lesend">
            <div className="grid grid-cols-[minmax(0,1fr)_repeat(4,80px)] text-[12px]">
              {['', 'Owner', 'Admin', 'Member', 'Viewer'].map((h) => (
                <span key={h} className="border-b border-border pb-1 text-center text-[11px] text-muted-foreground first:text-left">
                  {h}
                </span>
              ))}
              {roleMatrix.map((r) => (
                <div key={r[0]} className="contents">
                  {r.map((c, i) => (
                    <span key={i} className={cn('border-b border-border/50 py-1', i === 0 ? '' : 'text-center', c === '–' && 'text-muted-foreground')}>
                      {c === '✔' ? <Check className="mx-auto size-3.5 text-ok" aria-label="ja" /> : c}
                    </span>
                  ))}
                </div>
              ))}
            </div>
            <p className="mt-1.5 text-[12px] text-muted-foreground">Admins sehen keine Inhalte fremder Sessions – nur Owner, Titel, Kosten und Status.</p>
          </Section>
        </F>
      </Scroll>
    </SettingsLayout>
  )
}

/* ------------------------------------------------------------------ Session-Freigaben */

const shareRoles = [
  { id: 'view', icon: Eye, t: 'Lesen', d: 'Verlauf und Änderungen live sehen' },
  { id: 'comment_approve', icon: MessageSquare, t: 'Kommentieren & freigeben', d: 'Kommentare schreiben, Freigaben erteilen – keine Eingaben' },
  { id: 'drive', icon: Keyboard, t: 'Mitsteuern', d: 'Eingaben senden, Terminal und Browser nutzen' },
]

export function SessionSharing({ state }: { state: string }) {
  const viewer = state === 'viewer'
  return (
    <Shell
      nav="sessions"
      connection="server"
      activeSession="ses_6q2a"
      overlay={
        !viewer ? (
          <InlineDialog
            title="„Checkout-Formular barrierefrei machen“ teilen"
            width="w-[560px]"
            footer={
              <>
                <Btn tone="quiet">Schließen</Btn>
                <PrimaryButton>Freigaben speichern</PrimaryButton>
              </>
            }
          >
            <div className="flex items-center gap-2">
              <span className="flex-1 rounded-md border border-input bg-card px-2 py-1 text-muted-foreground">Person oder Team hinzufügen</span>
            </div>
            <div className="divide-y divide-border/70">
              {[
                { who: 'Mara Klein', role: state === 'drive' ? 'drive' : 'comment_approve' },
                { who: 'Team web (6 Personen)', role: 'view' },
                { who: 'Lea Sommer (Viewer)', role: 'view', capped: true },
              ].map((p) => (
                <div key={p.who} className="flex items-center gap-3 py-1.5">
                  <span className="flex-1">{p.who}</span>
                  <span className="rounded-md border border-input bg-card px-2 py-0.5 text-[12px]">{shareRoles.find((r) => r.id === p.role)!.t}</span>
                  {p.capped && <span className="text-[11px] text-muted-foreground">höchstens Lesen</span>}
                </div>
              ))}
            </div>
            <div className="grid grid-cols-3 gap-2">
              {shareRoles.map((r) => (
                <div key={r.id} className="rounded-md border border-border p-2">
                  <div className="flex items-center gap-1.5 text-[12px] font-medium">
                    <r.icon className="size-3.5" />
                    {r.t}
                  </div>
                  <div className="mt-0.5 text-[11px] text-muted-foreground">{r.d}</div>
                </div>
              ))}
            </div>
            {state === 'drive' && (
              <F id="AUTH-015">
                <p className="rounded-md border border-foreground/30 bg-card px-2 py-1.5 text-[12px]">
                  <b>Mitsteuern heißt: Mara führt Code auf deinem Rechner aus</b> – mit deinen Credentials, in deiner Sandbox und unter deinen Policies. Gib das nur
                  Personen, denen du so weit vertraust.
                </p>
              </F>
            )}
          </InlineDialog>
        ) : undefined
      }
    >
      <SessionHeader
        title="Checkout-Formular barrierefrei machen"
        harness="claude"
        status="idle"
        branch="beton/checkout-a11y-6q2a"
        extra={
          viewer ? (
            <Tag mono={false}>
              <Eye className="mr-1 size-3" /> du: Lesen
            </Tag>
          ) : undefined
        }
      />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <F id="AUTH-015" className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
          <UserMessage>Prüf das Formular mit axe und behebe alle Fehler der Stufe A.</UserMessage>
          <ToolCall kind="shell" name="Shell" target="pnpm exec axe http://localhost:3000/checkout" duration="6,1 s" />
          <AgentMessage harness="claude">
            <p>7 Verstöße gefunden, 5 behoben. Zwei betreffen Farbkontraste im Designsystem – soll ich die Tokens anpassen?</p>
          </AgentMessage>
          {viewer && (
            <Callout title="Terminal nicht verfügbar">
              <p>Mit Leserecht kannst du kein Terminal und keinen Browser der Session öffnen und keine Freigaben erteilen. Bitte Ingo um „Mitsteuern“, wenn du eingreifen willst.</p>
            </Callout>
          )}
        </F>
      </div>
      {viewer && <div className="shrink-0 border-t border-border px-6 py-3 text-[12px] text-muted-foreground">Nur lesen – Eingaben sind für dich gesperrt.</div>}
    </Shell>
  )
}

/* ------------------------------------------------------------------ Git-Verbindungen */

export function GitConnections({ state }: { state: string }) {
  const local = state === 'local-pat'
  const device = state === 'device-flow'
  const failed = state === 'refresh-failed'
  return (
    <SettingsLayout active="git" connection={local ? 'local' : 'server'}>
      <PageHead
        title="Git-Verbindungen"
        sub="Damit Agents pushen und PRs öffnen können – immer über Platzhalter in der Sandbox, nie mit dem echten Token."
        actions={<PrimaryButton>Verbindung hinzufügen</PrimaryButton>}
      >
        <div className="mt-2">
          {local ? <Premise>Lokal ohne OAuth-App: Personal Access Token als Secret im Schlüsselbund</Premise> : <Premise kind="team">OAuth-Apps vom Betreiber des Team-Servers registriert</Premise>}
        </div>
      </PageHead>
      <Scroll className="px-6 py-4">
        <div className="max-w-4xl space-y-4">
          {failed && (
            <F id="SEC-010">
              <Callout tone="wait" title="GitLab neu verbinden" actions={<PrimaryButton waiting>Bei gitlab.example.com anmelden</PrimaryButton>}>
                <p>
                  Der Zugang zu gitlab.example.com ist abgelaufen und ließ sich nicht automatisch erneuern. Pushes in 2 Sessions scheitern gerade mit{' '}
                  <C>credential_expired</C>.
                </p>
              </Callout>
            </F>
          )}
          {device && (
            <F id="SEC-009">
              <Callout tone="wait" title="Bei GitHub bestätigen">
                <p>
                  Öffne <b>github.com/login/device</b> und gib diesen Code ein: <span className="ml-1 font-mono text-[15px] tracking-[0.2em]">9F2C-71KD</span>
                </p>
                <p className="text-muted-foreground">beton wartet auf die Bestätigung (noch 13:40). Angefragte Rechte: repo, read:org, read:user, user:email.</p>
              </Callout>
            </F>
          )}
          <F id={local ? ['SEC-011'] : ['SEC-009', 'SEC-010']}>
            <div className="overflow-hidden rounded-md border border-border bg-card">
              {(local
                ? [{ host: 'github.com', kind: 'GitHub', acc: 'ifahrentholz', how: 'Personal Access Token', scopes: 'repo, read:org', st: 'gültig · zuletzt geprüft vor 3 Min.' }]
                : [
                    { host: 'github.com', kind: 'GitHub', acc: 'ifahrentholz', how: 'OAuth', scopes: 'repo, read:org, read:user, user:email', st: 'verbunden' },
                    { host: 'ghe.acme.example', kind: 'GitHub Enterprise', acc: 'ifahrentholz', how: 'OAuth · API /api/v3', scopes: 'repo, read:org', st: 'verbunden' },
                    { host: 'gitlab.example.com', kind: 'GitLab (self-hosted)', acc: 'ingo', how: 'OAuth mit PKCE · eigene CA', scopes: 'api, read_user', st: failed ? 'abgelaufen' : 'erneuert sich automatisch (läuft in 1 Std. ab)' },
                  ]
              ).map((c) => (
                <div key={c.host} className="grid grid-cols-[170px_minmax(0,1fr)_minmax(0,1fr)_170px_150px] items-center gap-3 border-b border-border/70 px-3 py-2 text-[12px] last:border-b-0">
                  <span>
                    <span className="block font-medium">{c.kind}</span>
                    <span className="font-mono text-[11px] text-muted-foreground">{c.host}</span>
                  </span>
                  <span>
                    {c.acc} <span className="text-muted-foreground">· {c.how}</span>
                  </span>
                  <span className="font-mono text-[11px] text-muted-foreground">{c.scopes}</span>
                  <span className={cn(c.st === 'abgelaufen' ? 'text-deny' : 'text-muted-foreground')}>
                    {c.st === 'abgelaufen' && <span className="mr-1.5 inline-block size-2 rotate-45 bg-deny" />}
                    {c.st}
                  </span>
                  <span className="flex justify-end gap-1">
                    <Btn tone="quiet">Testen</Btn>
                    <Btn tone="quiet">Trennen</Btn>
                  </span>
                </div>
              ))}
            </div>
            <p className="mt-1.5 text-[12px] text-muted-foreground">
              {local
                ? 'Ohne registrierte OAuth-App ist das PAT der Weg. beton setup erkennt gh- und glab-Logins und bietet die Übernahme an – nie still.'
                : '„Trennen“ widerruft den Zugang beim Anbieter und löscht das Secret. Mehrere Konten pro Host sind möglich.'}
            </p>
          </F>
          {local && (
            <F id="SEC-011">
              <Section title="Token hinterlegen">
                <KV k="Anbieter">GitHub · github.com</KV>
                <KV k="Token">
                  <span className="font-mono tracking-widest text-muted-foreground">••••••••••••</span> verdeckt eingeben
                </KV>
                <p className="mt-1 text-[12px] text-muted-foreground">
                  „Testen“ meldet fehlende Rechte anhand der Antwort von GitHub, z. B. „repo fehlt – private Repos lassen sich nicht klonen“.
                </p>
              </Section>
            </F>
          )}
        </div>
      </Scroll>
    </SettingsLayout>
  )
}
