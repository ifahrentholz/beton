import { CheckCircle2, KeyRound, Link2, Lock, Plus, ShieldCheck } from 'lucide-react'
import { TerminalOutput } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, C, Callout, InlineDialog, KV, Mark, PageHead, Premise, PrimaryButton, Scroll, Section, SettingsLayout, Tag } from '../policies/kit'

/* ------------------------------------------------------------------ Secrets */

type SecretRow = { ref: string; type: string; provider: string; account: string; used: string; scope?: string }

const local: SecretRow[] = [
  { ref: 'secret://user/github.com/ifahrentholz', type: 'token', provider: 'github.com', account: 'ifahrentholz', used: 'vor 3 Min.' },
  { ref: 'secret://user/gitlab.example.com/ingo', type: 'oauth', provider: 'gitlab.example.com', account: 'ingo', used: 'gestern' },
  { ref: 'secret://user/npm/ifahrentholz', type: 'token', provider: 'npm', account: 'ifahrentholz', used: 'vor 2 Tagen' },
  { ref: 'secret://user/custom:sentry/ci#token', type: 'token', provider: 'custom:sentry', account: 'ci', used: 'nie' },
]

const team: SecretRow[] = [
  { ref: 'secret://user/github.com/ifahrentholz', type: 'oauth', provider: 'github.com', account: 'ifahrentholz', used: 'vor 3 Min.', scope: 'nur du' },
  { ref: 'secret://team:plattform/npm/acme-bot', type: 'token', provider: 'npm', account: 'acme-bot', used: 'vor 1 Std.', scope: 'Team plattform' },
  { ref: 'secret://org/custom:sentry/acme#ci', type: 'token', provider: 'custom:sentry', account: 'acme', used: 'heute', scope: 'ganze Org' },
]

export function Secrets({ state }: { state: string }) {
  const isTeam = state === 'team'
  const rows = isTeam ? team : local
  const locked = state === 'locked'
  return (
    <SettingsLayout
      active="secrets"
      connection={isTeam ? 'server' : 'local'}
      overlay={
        state === 'add' ? (
          <InlineDialog
            title="Secret hinzufügen"
            footer={
              <>
                <Btn tone="quiet">Abbrechen</Btn>
                <Btn>Testen</Btn>
                <PrimaryButton>Im Schlüsselbund speichern</PrimaryButton>
              </>
            }
          >
            <div className="grid grid-cols-[110px_1fr] items-center gap-2">
              <span className="text-muted-foreground">Anbieter</span>
              <span className="rounded-md border border-input bg-card px-2 py-1">GitHub · github.com</span>
              <span className="text-muted-foreground">Konto</span>
              <span className="rounded-md border border-input bg-card px-2 py-1">ifahrentholz</span>
              <span className="text-muted-foreground">Token</span>
              <span className="flex items-center rounded-md border border-ring bg-card px-2 py-1 font-mono tracking-widest outline-2 outline-ring">
                ••••••••••••••••••••
              </span>
            </div>
            <p className="text-[12px] text-muted-foreground">
              Wird verdeckt eingegeben und nie wieder angezeigt – auch nicht gekürzt. Auf der Kommandozeile geht das nur per Prompt oder stdin:{' '}
              <C>beton secrets add github --host github.com</C>
            </p>
          </InlineDialog>
        ) : state === 'import-gh' ? (
          <InlineDialog
            title="Vorhandene GitHub-Anmeldung übernehmen?"
            waiting
            footer={
              <>
                <Btn tone="quiet">Nein, nichts speichern</Btn>
                <PrimaryButton waiting>Token übernehmen</PrimaryButton>
              </>
            }
          >
            <p>
              <C>gh</C> ist auf diesem Rechner als <b>ifahrentholz</b> angemeldet (Scopes: repo, read:org, workflow). beton kann diesen Token einmalig in den
              Schlüsselbund kopieren und Agents über den Platzhalter <C>GH_TOKEN=bt_cred_…</C> nutzen lassen.
            </p>
            <p className="text-muted-foreground">Ohne deine Bestätigung wird nichts gespeichert. Später änderst du das unter „Git-Verbindungen“.</p>
          </InlineDialog>
        ) : undefined
      }
    >
      <PageHead
        title="Secrets"
        sub="Tokens für Git, Registries und eigene APIs. Werte gibst du nur ein – angezeigt werden sie nie."
        actions={
          <PrimaryButton>
            <Plus className="size-3.5" /> Secret hinzufügen
          </PrimaryButton>
        }
      >
        <div className="mt-2 flex flex-wrap gap-x-5 gap-y-1">
          {locked ? (
            <Premise kind="lock">Speicher: verschlüsselte Datei ~/.beton/secrets/store.v1 (kein Schlüsselbund auf diesem Server)</Premise>
          ) : isTeam ? (
            <Premise kind="team">Persönliche Secrets lokal im Schlüsselbund · Team- und Org-Secrets verschlüsselt auf dem Team-Server</Premise>
          ) : (
            <Premise kind="lock">Speicher: macOS-Schlüsselbund, Dienst „beton“</Premise>
          )}
          <Premise>Kein API-Key für Modelle nötig – die CLIs nutzen ihre eigene Anmeldung</Premise>
        </div>
      </PageHead>
      <Scroll className="px-6 py-4">
        <div className="max-w-5xl space-y-4">
          {locked && (
            <F id="SEC-003">
              <Callout
                tone="wait"
                title="Secrets sind gesperrt – 2 Sessions warten"
                actions={
                  <>
                    <span className="flex h-7 w-56 items-center rounded-md border border-input bg-card px-2 font-mono text-[12px] tracking-widest">••••••••••</span>
                    <PrimaryButton waiting>Entsperren</PrimaryButton>
                  </>
                }
              >
                <p>
                  Auf diesem Linux-Server gibt es keinen Schlüsselbund. beton nutzt eine mit deiner Passphrase verschlüsselte Datei (AES-256-GCM, Argon2id).
                  Sessions, die ein Secret brauchen, starten erst nach dem Entsperren (<C>secrets_locked</C>).
                </p>
                <p className="text-muted-foreground">Alternativ: BETON_SECRETS_PASSPHRASE_FILE für unbeaufsichtigte Starts.</p>
              </Callout>
            </F>
          )}
          {isTeam && (
            <F id={['SEC-007', 'SEC-008']}>
              <Callout tone="deny" title="Secret für eine fremde Session abgelehnt">
                <p>
                  Bobs Session „Nightly e2e“ nutzt deine Agent-Datei <C>agents/pr-fixer/agent.yaml</C> mit <C>secret://user/github.com/ifahrentholz</C>. Dein
                  persönliches Secret gilt nur in deinen Sessions – Start mit <C>secret_binding_mismatch</C> abgebrochen.
                </p>
              </Callout>
            </F>
          )}
          <F id={['SEC-001', 'SEC-002', 'SEC-004', 'SEC-011']}>
            <div className="overflow-hidden rounded-md border border-border bg-card">
              <div className={cn('grid gap-3 border-b border-border bg-sunken px-3 py-1.5 text-[11px] text-muted-foreground', isTeam ? 'grid-cols-[minmax(0,1.6fr)_70px_120px_minmax(0,1fr)_110px_100px_150px]' : 'grid-cols-[minmax(0,1.6fr)_70px_minmax(0,1fr)_110px_100px_200px]')}>
                <span>Referenz</span>
                <span>Typ</span>
                {isTeam && <span>Gilt für</span>}
                <span>Anbieter</span>
                <span>Konto</span>
                <span>Zuletzt genutzt</span>
                <span />
              </div>
              {rows.map((r) => (
                <div
                  key={r.ref}
                  className={cn('grid items-center gap-3 border-b border-border/70 px-3 py-1.5 text-[12px] last:border-b-0', isTeam ? 'grid-cols-[minmax(0,1.6fr)_70px_120px_minmax(0,1fr)_110px_100px_150px]' : 'grid-cols-[minmax(0,1.6fr)_70px_minmax(0,1fr)_110px_100px_200px]', locked && 'opacity-60')}
                >
                  <span className="flex min-w-0 items-center gap-1.5 font-mono text-[11px]">
                    {locked ? <Lock className="size-3 shrink-0" /> : <KeyRound className="size-3 shrink-0 text-muted-foreground" />}
                    <span className="truncate">{r.ref}</span>
                  </span>
                  <span className="font-mono text-muted-foreground">{r.type}</span>
                  {isTeam && <span>{r.scope}</span>}
                  <span className="truncate">{r.provider}</span>
                  <span className="truncate">{r.account}</span>
                  <span className="text-muted-foreground">{r.used}</span>
                  <span className="flex justify-end gap-1">
                    <Btn tone="quiet">Testen</Btn>
                    <Btn tone="quiet">Ersetzen</Btn>
                    {!isTeam && <Btn tone="quiet">Löschen</Btn>}
                  </span>
                </div>
              ))}
            </div>
            <p className="mt-1.5 text-[12px] text-muted-foreground">
              „Testen“ prüft über den Proxy-Pfad beim Anbieter (z. B. GitHub <C>/user</C>) und meldet fehlende Scopes – ohne den Wert zu zeigen. „Ersetzen“
              rotiert: neuer Wert, alte Platzhalter werden ungültig.
            </p>
          </F>
          {isTeam && (
            <F id={['SEC-008', 'SEC-005']}>
              <Section title="An Runner ausgeliefert" hint="nur im Speicher des Proxys, 1 Std. gültig, erneuerbar">
                <div className="space-y-1 text-[12px]">
                  {[
                    ['lse_01JBA…', 'secret://team:plattform/npm/acme-bot', 'Runner k8s-runner-3 · „Release-Notes generieren“', 'noch 42 Min.'],
                    ['lse_01JB9…', 'secret://user/github.com/ifahrentholz', 'Host ingo-mbp · „Rate-Limiter für die Login-API“', 'noch 18 Min.'],
                  ].map(([id, ref, where, exp]) => (
                    <div key={id} className="grid grid-cols-[100px_minmax(0,1.2fr)_minmax(0,1.4fr)_100px_90px] items-center gap-3">
                      <span className="font-mono text-[11px] text-muted-foreground">{id}</span>
                      <span className="truncate font-mono text-[11px]">{ref}</span>
                      <span className="truncate">{where}</span>
                      <span className="text-muted-foreground">{exp}</span>
                      <Btn tone="quiet">Widerrufen</Btn>
                    </div>
                  ))}
                </div>
                <p className="mt-1.5 text-[12px] text-muted-foreground">
                  Auf dem Team-Server ist jeder Wert mit eigenem Schlüssel verschlüsselt (Envelope-Encryption). Ändern oder Löschen widerruft laufende
                  Auslieferungen binnen Sekunden.
                </p>
              </Section>
            </F>
          )}
        </div>
      </Scroll>
    </SettingsLayout>
  )
}

/* ------------------------------------------------------------------ Audit-Log */

const audit = [
  { n: 18241, t: '14:22:39', who: 'Proxy (für ses_7f3k)', a: 'secret.used', target: 'secret://user/github.com/ifahrentholz', d: 'api.github.com · 14× in 5 Min.', h: '9f2c…' },
  { n: 18240, t: '14:21:40', who: 'Proxy (für ses_7f3k)', a: 'credential.misuse', target: 'Binding github', d: 'Platzhalter an evil-cdn.test · 403', h: '41ab…', bad: true },
  { n: 18239, t: '14:02:11', who: 'Ingo', a: 'policy.changed', target: '.beton/policies/git-and-budget.yaml', d: 'sha256:5aa0… → 9c1e…', h: 'c07e…' },
  { n: 18238, t: '13:55:02', who: 'Ingo', a: 'sandbox.none_confirmed', target: 'Session ses_6h2f', d: 'backend: none, einmalig', h: '7d11…' },
  { n: 18237, t: '11:40:18', who: 'Ingo', a: 'device.paired', target: 'Pixel 9 (PWA)', d: 'Scopes sessions:read, sessions:write, sessions:approve', h: '2b90…' },
  { n: 18236, t: '09:12:44', who: 'Ingo', a: 'proxy.ca_rotated', target: 'beton egress CA inst_7QK2M4', d: 'neue CA für neue Sessions', h: 'e4d3…' },
  { n: 18235, t: '09:02:01', who: 'Ingo', a: 'secret.created', target: 'secret://user/npm/ifahrentholz', d: 'Schlüsselbund', h: '5c6f…' },
  { n: 18234, t: 'gestern', who: 'Ingo', a: 'token.created', target: 'PAT „release-script“', d: 'sessions:read, usage:read · 90 Tage', h: '0aa8…' },
]

export function AuditLog({ state }: { state: string }) {
  const broken = state === 'verify-failed'
  return (
    <SettingsLayout active="audit">
      <PageHead
        title="Audit-Log"
        sub="Sicherheitsrelevante Aktionen, nur anhängbar und als Hash-Kette verknüpft. Enthält nie Secret-Werte."
        actions={
          <>
            <Btn>Exportieren</Btn>
            <PrimaryButton>
              <ShieldCheck className="size-3.5" /> Kette prüfen
            </PrimaryButton>
          </>
        }
      >
        <div className="mt-2">
          <Premise>Lokal 90 Tage aufbewahrt · beton audit list|export|verify</Premise>
        </div>
      </PageHead>
      <Scroll className="px-6 py-3">
        <F id={['SEC-012', 'PRX-009', 'PRX-002']}>
          {broken ? (
            <Callout tone="deny" title="Kette unterbrochen bei Eintrag 18 236" className="mb-3">
              <p>
                Der gespeicherte Hash passt nicht zum Inhalt – der Eintrag wurde nachträglich geändert oder ein Eintrag davor gelöscht. Alle Einträge ab 18 236 sind
                nicht mehr vertrauenswürdig.
              </p>
            </Callout>
          ) : (
            <div className="mb-3 inline-flex items-center gap-1.5 text-[12px]">
              <CheckCircle2 className="size-3.5 text-ok" /> Kette vollständig geprüft: 18 241 Einträge, heute 14:23
            </div>
          )}
          <div className="grid grid-cols-[60px_70px_150px_160px_minmax(0,1.2fr)_minmax(0,1.3fr)_64px] gap-3 border-b border-border pb-1 text-[11px] text-muted-foreground">
            <span>Nr.</span>
            <span>Zeit</span>
            <span>Wer</span>
            <span>Aktion</span>
            <span>Ziel</span>
            <span>Details</span>
            <span>Hash</span>
          </div>
          {audit.map((e) => (
            <div
              key={e.n}
              className={cn(
                'grid grid-cols-[60px_70px_150px_160px_minmax(0,1.2fr)_minmax(0,1.3fr)_64px] items-center gap-3 border-b border-border/60 py-1.5 text-[12px]',
                broken && e.n >= 18236 && 'bg-deny-soft/60',
              )}
            >
              <span className="font-mono text-[11px] text-muted-foreground">{e.n}</span>
              <span className="text-muted-foreground">{e.t}</span>
              <span className="truncate">{e.who}</span>
              <span className={cn('font-mono text-[11px]', e.bad && 'text-deny')}>{e.a}</span>
              <span className="truncate font-mono text-[11px]">{e.target}</span>
              <span className="truncate text-muted-foreground">{e.d}</span>
              <span className="flex items-center gap-1 font-mono text-[10px] text-muted-foreground">
                <Link2 className="size-3" />
                {e.h}
              </span>
            </div>
          ))}
          <p className="mt-2 text-[12px] text-muted-foreground">Häufige Nutzung eines Secrets wird pro Session und Host in 5-Minuten-Fenstern gezählt statt einzeln geschrieben.</p>
        </F>
      </Scroll>
    </SettingsLayout>
  )
}

/* ------------------------------------------------------------------ Redaction */

export function Redaction({ state }: { state: string }) {
  const pii = state === 'pii'
  const diag = state === 'diagnose'
  return (
    <SettingsLayout active="redaction">
      <PageHead title="Redaction" sub="Bevor Tool-Ausgaben, Ereignisse, Logs, Exporte und Diagnose-Bundles gespeichert oder verschickt werden, schwärzt beton bekannte Secrets." />
      <Scroll className="px-6 py-4">
        <div className="max-w-5xl space-y-5">
          <F id={pii ? ['POL-019'] : ['SEC-013']}>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <div className="mb-1 text-[12px] text-muted-foreground">{diag ? 'Rohdaten (nur im Speicher)' : 'So kam es aus dem Tool'}</div>
                <TerminalOutput>
                  {pii
                    ? `$ psql -c "select * from customers limit 2"
 id | email               | iban                   | card
  1 | jana.k@example.org  | DE89370400440532013000 | 4111111111111111
  2 | t.ochs@example.net  | DE02120300000000202051 | 4111111111111112`
                    : diag
                      ? `runner.log: GH_TOKEN=ghp_4uR9…Zx2 (aus env.passthrough)
proxy.log: Authorization: Bearer glpat-x8Wq…3a
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB…
-----END OPENSSH PRIVATE KEY-----`
                      : `$ env | grep -i token
GH_TOKEN=bt_cred_k7m2q9xw4ttyc3fa5hlm2pd8vze61r0n
LEGACY_DEPLOY_TOKEN=ghp_4uR9mKq2…Fj2Zx2
SENTRY_AUTH_TOKEN=sntrys_eyJpYXQiOjE3…`}
                </TerminalOutput>
              </div>
              <div>
                <div className="mb-1 text-[12px] text-muted-foreground">{diag ? 'Im Diagnose-Bundle' : 'So steht es im Verlauf und auf allen Geräten'}</div>
                <TerminalOutput>
                  {pii
                    ? `$ psql -c "select * from customers limit 2"
 id | email               | iban                   | card
  1 | [REDACTED:email]    | [REDACTED:iban]        | [REDACTED:credit_card]
  2 | [REDACTED:email]    | [REDACTED:iban]        | 4111111111111112`
                    : diag
                      ? `runner.log: GH_TOKEN=[REDACTED:github_token]
proxy.log: Authorization: Bearer [REDACTED:gitlab_token]
[REDACTED:private_key]`
                      : `$ env | grep -i token
GH_TOKEN=bt_cred_k7m2q9xw4ttyc3fa5hlm2pd8vze61r0n
LEGACY_DEPLOY_TOKEN=[REDACTED:github_token]
SENTRY_AUTH_TOKEN=[REDACTED:known_secret]`}
                </TerminalOutput>
              </div>
            </div>
            <p className="mt-2 text-[12px] text-muted-foreground">
              {pii
                ? 'Policy pii_redaction (User): E-Mail, IBAN und Kreditkarten mit gültiger Prüfziffer (Luhn) werden geschwärzt, bevor das Ergebnis das Modell erreicht. Die zweite Kartennummer ist ungültig und bleibt stehen.'
                : diag
                  ? 'beton diagnose enthält nach der Schwärzung keine bekannten Werte – auch keine Base64- oder URL-kodierten Varianten.'
                  : 'Platzhalter (bt_cred_…) bleiben stehen – sie sind harmlos. Echte Werte, die z. B. über env.passthrough hereinkamen, werden ersetzt; Live-Ansichten erhalten nur die geschwärzte Fassung.'}
            </p>
          </F>
          <div className="grid grid-cols-2 gap-6">
            <F id="SEC-013">
              <Section title="Erkennung">
                <Mark v="yes">Exakter Abgleich mit allen Secrets dieses Rechners (auch Base64/URL-kodiert)</Mark>
                <div className="mt-1 font-mono text-[11px] leading-relaxed text-muted-foreground">
                  ghp_ gho_ github_pat_ · glpat- · sk-ant- · sk-proj- · AKIA/ASIA · xox[abpr]- · PEM-Schlüssel · bt_pat_ bt_sat_ bt_dev_ bt_run_ bt_loc_ · JWT in
                  Authorization
                </div>
                <div className="mt-2">
                  <Mark v="no">Entropie-Heuristik (aus – zu viele Fehltreffer in Hashes und Lockfiles)</Mark>
                </div>
              </Section>
            </F>
            <F id="POL-019">
              <Section title="Personenbezogene Daten" hint="Policy pii_redaction">
                <KV k="Erkennung">E-Mail, IBAN, Kreditkarte (Luhn), API-Keys, private Schlüssel</KV>
                <KV k="Wirkt auf">Tool-Ergebnisse; beim Direkt-API-Harness auch Modell-Anfragen</KV>
                <KV k="Wo nicht schwärzbar">nur Meldung (z. B. Codex-Ergebnisse) – siehe Durchsetzung</KV>
              </Section>
            </F>
          </div>
        </div>
      </Scroll>
    </SettingsLayout>
  )
}

/* ------------------------------------------------------------------ Server-Härtung */

export function ServerHardening({ state }: { state: string }) {
  const missing = state === 'key-missing'
  return (
    <SettingsLayout active="hardening" connection="server">
      <PageHead title="Server-Härtung" sub="Nur für den optionalen Team-Server. Lokal bindet beton ausschließlich an 127.0.0.1 und ::1.">
        <div className="mt-2">
          <Premise kind="team">beton.example.com · beton serve 1.4.0 · zentraler Modus (OIDC)</Premise>
        </div>
      </PageHead>
      <Scroll className="px-6 py-4">
        <div className="max-w-5xl space-y-5">
          {missing && (
            <F id="SEC-006">
              <Callout tone="deny" title="Server startet nicht: ein Master-Key fehlt">
                <p>
                  Der Schlüssel <C>k2025b</C> wurde aus der Konfiguration entfernt, aber 412 Secrets sind noch mit ihm verschlüsselt. Füge ihn wieder hinzu und
                  führe den Rewrap zu Ende, bevor du ihn entfernst.
                </p>
                <TerminalOutput>{`$ beton serve
error: master_key_missing: k2025b referenced by 412 secrets
hint:  beton admin secrets rewrap --to k2026a`}</TerminalOutput>
              </Callout>
            </F>
          )}
          <div className="grid grid-cols-2 gap-6">
            <F id="SEC-014">
              <Section title="Transport">
                <KV k="TLS">rustls, TLS 1.3 bevorzugt, ab 1.2</KV>
                <KV k="Vertraute Proxys" mono>
                  10.20.0.0/16
                </KV>
                <KV k="HSTS">an, 1 Jahr</KV>
                <KV k="Rate-Limits">Login, Callback, Pairing 10/Min. je IP · Tokens 60/Min.</KV>
                <KV k="Body-Limit">10 MiB (außer Uploads)</KV>
              </Section>
            </F>
            <F id="SEC-014">
              <Section title="Security-Header">
                {[
                  "Content-Security-Policy: default-src 'self'; frame-ancestors 'none'; object-src 'none'",
                  'X-Content-Type-Options: nosniff',
                  'Referrer-Policy: no-referrer',
                  'Cross-Origin-Opener-Policy: same-origin',
                  'Permissions-Policy: microphone=(self)',
                ].map((h) => (
                  <div key={h} className="py-0.5">
                    <Mark v="yes">
                      <span className="font-mono text-[11px]">{h}</span>
                    </Mark>
                  </div>
                ))}
                <p className="mt-1 text-[12px] text-muted-foreground">Anhänge und Workspace-Dateien nur mit isolierender CSP oder als Download.</p>
              </Section>
            </F>
          </div>
          <div className="grid grid-cols-2 gap-6">
            <F id={['AUTH-002', 'AUTH-003', 'AUTH-007']}>
              <Section title="Hosts, Origins, Anmeldungen">
                <KV k="Erlaubte Hosts" mono>
                  beton.example.com
                </KV>
                <KV k="WebSocket-Origins" mono>
                  https://beton.example.com · tauri://localhost
                </KV>
                <KV k="CORS">keine Freigaben</KV>
                <KV k="Web-Sitzung">Cookie __Host-beton_session · 14 Tage inaktiv, 30 Tage max.</KV>
              </Section>
            </F>
            <F id={['SEC-005', 'SEC-006']}>
              <Section title="Master-Keys" hint="Envelope-Encryption der Secrets">
                {[
                  { id: 'k2026a', src: 'Datei /run/secrets/beton_master_k2026a', st: 'aktiv', n: missing ? 2108 : state === 'rewrap' ? 1408 : 2520 },
                  { id: 'k2025b', src: missing ? 'fehlt in der Konfiguration' : 'Env BETON_MASTER_KEY_K2025B', st: 'alt', n: missing ? 412 : state === 'rewrap' ? 1112 : 0 },
                ].map((k) => (
                  <div key={k.id} className={cn('flex items-center gap-3 border-b border-border/60 py-1.5 text-[12px] last:border-b-0', missing && k.id === 'k2025b' && 'text-deny')}>
                    <span className="w-16 font-mono">{k.id}</span>
                    <span className="min-w-0 flex-1 truncate text-muted-foreground">{k.src}</span>
                    <Tag mono={false}>{k.st}</Tag>
                    <span className="w-24 text-right tabular-nums">{k.n.toLocaleString('de-DE')} Secrets</span>
                  </div>
                ))}
                {state === 'rewrap' && (
                  <div className="mt-2">
                    <div className="mb-1 flex justify-between text-[12px]">
                      <span>Rewrap auf k2026a läuft (fortsetzbar)</span>
                      <span className="tabular-nums">56 %</span>
                    </div>
                    <div className="h-1.5 rounded-full bg-muted">
                      <span className="block h-full w-[56%] rounded-full bg-foreground/70" />
                    </div>
                  </div>
                )}
                <p className="mt-1.5 text-[12px] text-muted-foreground">Ein alter Schlüssel lässt sich erst entfernen, wenn kein Secret mehr auf ihn verweist.</p>
              </Section>
            </F>
          </div>
        </div>
      </Scroll>
    </SettingsLayout>
  )
}
