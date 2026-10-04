import type { ReactNode } from 'react'
import { Fingerprint, KeyRound, RotateCw } from 'lucide-react'
import { TerminalOutput } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { BrowserChrome } from './parts'
import { Btn, C, Callout, KV, Mark, PageHead, Premise, PrimaryButton, Scroll, Section, SettingsLayout } from '@/app/kit/policies'

/* ------------------------------------------------------------------ Lokaler Zugang */

function Split({ left, right }: { left: ReactNode; right: ReactNode }) {
  return (
    <div className="grid h-full grid-cols-[minmax(0,1fr)_minmax(0,1.3fr)]">
      <div className="flex min-h-0 flex-col border-r border-border bg-sunken p-4">{left}</div>
      <div className="min-h-0">{right}</div>
    </div>
  )
}

function Label({ children }: { children: ReactNode }) {
  return <div className="mb-2 text-[12px] text-muted-foreground">{children}</div>
}

export function LocalAccess({ state }: { state: string }) {
  if (state === 'settings') {
    return (
      <SettingsLayout active="local">
        <PageHead title="Lokaler Zugang" sub="Kein Konto, kein Login: Nur Programme deines Benutzerkontos auf diesem Rechner kommen an beton heran.">
          <div className="mt-2">
            <Premise>Alles läuft auf diesem Rechner · kein Server · Einmal-Link für den Browser</Premise>
          </div>
        </PageHead>
        <Scroll className="px-6 py-4">
          <div className="grid max-w-5xl grid-cols-2 gap-6">
            <F id="AUTH-001">
              <Section title="Erreichbar unter" actions={<Btn tone="quiet"><RotateCw className="size-3.5" /> Token erneuern</Btn>}>
                <KV k="Adressen" mono>
                  127.0.0.1:7420 · [::1]:7420
                </KV>
                <KV k="Unix-Socket" mono>
                  ~/.beton/run/beton.sock (0600)
                </KV>
                <KV k="Token-Datei" mono>
                  ~/.beton/auth/local.token (0600)
                </KV>
                <KV k="Wer liest das Token">beton-CLI, TUI, Desktop-App</KV>
                <p className="mt-1 text-[12px] text-muted-foreground">
                  Andere Benutzerkonten auf diesem Rechner haben weder auf Token noch Socket Zugriff. Nach dem Erneuern verbinden sich offene Clients selbst neu.
                </p>
              </Section>
            </F>
            <F id={['AUTH-002', 'AUTH-003', 'AUTH-004']}>
              <Section title="Schutz vor fremden Webseiten">
                <Mark v="yes">Nur Host-Header 127.0.0.1, [::1], localhost (Schutz vor DNS-Rebinding)</Mark>
                <br />
                <Mark v="yes">Formulare und Cookies nur von derselben Origin (CSRF)</Mark>
                <br />
                <Mark v="yes">WebSocket nur von http://127.0.0.1:7420 und der Desktop-App</Mark>
                <br />
                <Mark v="yes">Keine CORS-Freigaben, kein Private-Network-Access</Mark>
                <p className="mt-2 text-[12px] text-muted-foreground">
                  Im Browser öffnest du beton mit <C>beton open</C>: Das erzeugt einen Einmal-Link (60 s, einmal einlösbar), der dich ohne Token-Kopieren anmeldet.
                </p>
              </Section>
            </F>
            <F id="AUTH-009" className="col-span-2">
              <Section title="Vom Handy erreichbar machen" hint="optional · über Tailscale oder LAN">
                <p className="text-[13px]">
                  Aus. Zum Einschalten braucht beton TLS (<C>tls: tailscale</C> oder eigenes Zertifikat). Über diese Adresse gelten nur gekoppelte Geräte – das lokale
                  Token nie.
                </p>
                <div className="mt-2">
                  <Btn>Handy koppeln …</Btn>
                </div>
              </Section>
            </F>
          </div>
        </Scroll>
      </SettingsLayout>
    )
  }

  if (state === 'insecure-token') {
    return (
      <F id="AUTH-001" className="h-full bg-sunken p-6">
        <Label>Terminal</Label>
        <TerminalOutput>{`$ beton serve
error: ~/.beton/auth/local.token ist für andere lesbar (Modus 0644).
       Andere Benutzer dieses Rechners könnten damit deine Agents steuern.
hint:  chmod 600 ~/.beton/auth/local.token && chmod 700 ~/.beton/auth
       oder neues Token: beton auth rotate-local
beton wurde nicht gestartet.`}</TerminalOutput>
        <p className="mt-3 max-w-xl text-[13px] text-muted-foreground">
          Der Daemon startet nicht, solange Token-Datei oder Ordner Gruppen- oder Fremdrechte haben oder einem anderen Benutzer gehören.
        </p>
      </F>
    )
  }

  const expired = state === 'expired'
  return (
    <Split
      left={
        <F id="AUTH-004">
          <Label>Terminal</Label>
          <TerminalOutput>{expired
            ? `$ beton open ses_7f3k
→ Öffne den Browser (Einmal-Link, 60 s gültig) …
`
            : `$ beton open ses_7f3k
→ Öffne den Browser (Einmal-Link, 60 s gültig) …
  http://127.0.0.1:7420/auth/local/redeem?code=…
✓ Im Browser angemeldet`}</TerminalOutput>
          <p className="mt-3 text-[12px] text-muted-foreground">
            Kein Passwort, kein Konto. Der Link gilt einmal und 60 Sekunden; danach steht er nicht mehr in der Adresszeile oder im Verlauf.
          </p>
        </F>
      }
      right={
        <BrowserChrome secure={false} url={expired ? '127.0.0.1:7420/auth/local/redeem' : '127.0.0.1:7420/sessions/ses_7f3k'}>
          {expired ? (
            <F id="AUTH-004" className="concrete-grain flex h-full items-center justify-center p-8">
              <div className="max-w-sm text-center">
                <p className="type-wide text-[17px] font-semibold">Dieser Link ist nicht mehr gültig</p>
                <p className="mt-2 text-[13px] text-muted-foreground">
                  Einmal-Links gelten 60 Sekunden und nur ein einziges Mal. Führe im Terminal erneut <C>beton open</C> aus.
                </p>
              </div>
            </F>
          ) : (
            <div className="flex h-full flex-col">
              <div className="flex h-11 items-center gap-3 border-b border-border px-4">
                <span className="type-wide text-[14px] font-bold">beton</span>
                <span className="text-[13px]">Rate-Limiter für die Login-API</span>
                <span className="ml-auto">
                  <Premise>lokal · 127.0.0.1</Premise>
                </span>
              </div>
              <div className="flex-1 p-6 text-[13px] text-muted-foreground">Die Session öffnet sich wie in der Desktop-App.</div>
            </div>
          )}
        </BrowserChrome>
      }
    />
  )
}

/* ------------------------------------------------------------------ OIDC-Login */

function Provider({ children }: { children: ReactNode }) {
  return <button className="flex h-9 w-full items-center justify-center gap-2 rounded-md border border-input bg-card text-[13px] font-medium hover:bg-accent">{children}</button>
}

export function TeamLogin({ state }: { state: string }) {
  return (
    <BrowserChrome url="beton.example.com/login">
      <div className="concrete-grain flex min-h-full items-center justify-center p-8">
        <div className="w-[380px] rounded-lg border border-border bg-popover p-6">
          <div className="type-wide text-[22px] font-[750]">beton</div>
          <p className="mt-1 text-[13px] text-muted-foreground">Team-Server der acme GmbH · beton.example.com</p>
          {state === 'denied' && (
            <F id="AUTH-005" className="mt-4">
              <Callout tone="deny" title="Anmeldung nicht möglich">
                <p>
                  Adressen von <b>other.test</b> sind für diese Org nicht zugelassen. Melde dich mit deinem Konto von acme an oder frage die Admins.
                </p>
              </Callout>
            </F>
          )}
          {state === 'disabled' && (
            <F id={['AUTH-005', 'AUTH-017']} className="mt-4">
              <Callout tone="deny" title="Dein Konto ist deaktiviert">
                <p>Du kannst dich nicht mehr anmelden – auch nicht per Passkey. Wende dich an die Admins der Org.</p>
              </Callout>
            </F>
          )}
          {state === 'passkey-reauth' && (
            <F id="AUTH-017" className="mt-4">
              <Callout title="Einmal über Microsoft anmelden">
                <p>Deine letzte Anmeldung über Entra ID ist 30 Tage her. Danach funktioniert dein Passkey wieder allein.</p>
              </Callout>
            </F>
          )}
          {state === 'passkey' ? (
            <F id="AUTH-017" className="mt-5 space-y-3">
              <div className="flex flex-col items-center gap-2 rounded-md border border-border bg-card p-5 text-center">
                <Fingerprint className="size-8" />
                <p className="text-[13px]">Bestätige mit Touch ID oder deinem Sicherheitsschlüssel.</p>
                <p className="text-[12px] text-muted-foreground">Passkey „MacBook Ingo“ · zuletzt genutzt vor 3 Tagen</p>
              </div>
              <Btn tone="quiet">Stattdessen über den Identitätsanbieter anmelden</Btn>
            </F>
          ) : (
            <F id={['AUTH-005', 'AUTH-007']} className="mt-5 space-y-2">
              <Provider>Mit Microsoft Entra ID anmelden</Provider>
              <Provider>Mit Google anmelden</Provider>
              <Provider>Mit GitHub anmelden</Provider>
              <Provider>Mit Keycloak (sso.example.com) anmelden</Provider>
              {state !== 'passkey-reauth' && (
                <F id="AUTH-017">
                  <div className="my-3 flex items-center gap-3 text-[11px] text-muted-foreground">
                    <span className="h-px flex-1 bg-border" /> oder <span className="h-px flex-1 bg-border" />
                  </div>
                  <Provider>
                    <KeyRound className="size-4" /> Mit Passkey anmelden
                  </Provider>
                </F>
              )}
            </F>
          )}
          <p className="mt-5 text-[12px] text-muted-foreground">
            Nur für den Team-Betrieb. Auf deinem eigenen Rechner brauchst du kein Konto – dort läuft beton lokal ohne Anmeldung.
          </p>
        </div>
      </div>
    </BrowserChrome>
  )
}

/* ------------------------------------------------------------------ Org-Bootstrap */

export function OrgBootstrap({ state }: { state: string }) {
  const done = state === 'done'
  return (
    <BrowserChrome url="beton.example.com/setup">
      <div className="concrete-grain flex min-h-full items-center justify-center p-8">
        <F id="AUTH-006" className="w-[460px] rounded-lg border border-border bg-popover p-6">
          {done ? (
            <>
              <p className="type-wide text-[18px] font-semibold">Du bist Owner von „acme“</p>
              <p className="mt-2 text-[13px] text-muted-foreground">
                Der Einrichtungsweg ist jetzt geschlossen. Weitere Admins ernennst du unter Mitglieder &amp; Rollen. Eine Org hat immer mindestens einen Owner.
              </p>
              <div className="mt-4 flex gap-2">
                <PrimaryButton>Anmeldeanbieter prüfen</PrimaryButton>
                <Btn>Mitglieder einladen</Btn>
              </div>
            </>
          ) : (
            <>
              <p className="type-wide text-[18px] font-semibold">Server einrichten</p>
              <p className="mt-2 text-[13px]">
                Du bist als <b>ingo@acme.example</b> angemeldet und bisher Mitglied. Gib den Einrichtungscode ein, den <C>beton serve</C> beim ersten Start
                ausgegeben hat, um erster Owner zu werden.
              </p>
              <div className="mt-3 rounded-md border border-ring bg-card px-3 py-2 font-mono text-[15px] tracking-[0.2em] outline-2 outline-ring">K7PM-2QXR-9WDA</div>
              <p className="mt-1.5 text-[12px] text-muted-foreground">24 Stunden gültig. Steht in den Server-Logs (stdout), nirgends sonst.</p>
              <div className="mt-4 flex justify-end">
                <PrimaryButton>Owner werden</PrimaryButton>
              </div>
            </>
          )}
        </F>
      </div>
    </BrowserChrome>
  )
}
