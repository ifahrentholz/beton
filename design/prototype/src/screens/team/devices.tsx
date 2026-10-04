import type { ReactNode } from 'react'
import { Laptop, Monitor, Server, Smartphone, SquareTerminal } from 'lucide-react'
import { TerminalOutput } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { BrowserChrome, FakeQr } from './parts'
import { Btn, C, Callout, InlineDialog, KV, PageHead, Premise, PrimaryButton, Scroll, SettingsLayout, Tag } from '@/app/kit/policies'

/* ------------------------------------------------------------------ Pairing per Code */

export function DevicePairing({ state }: { state: string }) {
  const paired = state === 'paired'
  const limited = state === 'rate-limited'
  const expired = state === 'expired'
  return (
    <div className="grid h-full grid-cols-[minmax(0,1fr)_minmax(0,1.25fr)]">
      <F id="AUTH-008" className="flex flex-col border-r border-border bg-sunken p-4">
        <div className="mb-2 text-[12px] text-muted-foreground">Auf dem Gerät: build-box-01 (Linux, ohne Bildschirm)</div>
        <TerminalOutput>{`$ beton host --server https://beton.example.com
Gerät koppeln: öffne im Browser

  https://beton.example.com/pair

und gib diesen Code ein:   BCDF-GHJK

  ▄▄▄▄▄▄▄ ▄ ▄▄ ▄▄▄▄▄▄▄
  █ ▄▄▄ █ ▀█▄▀ █ ▄▄▄ █
  █ ███ █ ▄▀▀█ █ ███ █
  █▄▄▄▄▄█ █▀▄▀ █▄▄▄▄▄█

Warte auf Bestätigung … (läuft ab in 9:12)
${paired ? `✓ Gekoppelt als „build-box-01“ (Host)
  Token im Schlüsselbund gespeichert, nicht in einer Datei.
  Tunnel zu beton.example.com steht.` : expired ? `✗ Code abgelaufen (10 Min.). Starte beton host erneut.` : ''}`}</TerminalOutput>
      </F>
      <BrowserChrome url={paired ? 'beton.example.com/settings/devices' : 'beton.example.com/pair?code=BCDF-GHJK'}>
        <div className="flex min-h-full items-start justify-center p-8">
          <div className="w-[440px]">
            {paired ? (
              <F id={['AUTH-008', 'AUTH-010']}>
                <Callout tone="ok" title="build-box-01 ist gekoppelt">
                  <p>Der Host kann jetzt Sessions für dich ausführen. Du findest ihn unter Geräte &amp; Hosts und kannst ihn dort jederzeit widerrufen.</p>
                </Callout>
              </F>
            ) : limited ? (
              <F id="AUTH-008">
                <Callout tone="deny" title="Zu viele falsche Codes">
                  <p>Du hast in 10 Minuten fünfmal einen falschen Code eingegeben. Versuche es in 8 Minuten erneut – auch der richtige Code wird bis dahin abgelehnt.</p>
                </Callout>
              </F>
            ) : expired ? (
              <F id="AUTH-008">
                <Callout tone="deny" title="Code abgelaufen">
                  <p>Pairing-Codes gelten 10 Minuten. Starte die Kopplung auf dem Gerät neu.</p>
                </Callout>
              </F>
            ) : (
              <F id="AUTH-008" className="chamfer border-t-4 border-signal bg-popover p-5">
                <p className="type-wide text-[17px] font-semibold">Dieses Gerät koppeln?</p>
                <p className="mt-1 text-[13px] text-muted-foreground">Prüfe, dass auf dem Gerät genau dieser Code steht. Koppel nichts, was du nicht selbst gestartet hast.</p>
                <div className="mt-3 rounded-md border border-border bg-card px-3 py-2 text-center font-mono text-[20px] tracking-[0.25em]">BCDF-GHJK</div>
                <div className="mt-3">
                  <KV k="Art">Host (führt Sessions aus)</KV>
                  <KV k="Name">build-box-01</KV>
                  <KV k="Plattform">Linux x86_64 · beton 1.4.0</KV>
                  <KV k="Anfrage von" mono>
                    203.0.113.24
                  </KV>
                  <KV k="Rechte">
                    <C>hosts:connect</C>
                  </KV>
                </div>
                <div className="mt-4 flex gap-2">
                  <PrimaryButton waiting>Gerät koppeln</PrimaryButton>
                  <Btn>Ablehnen</Btn>
                </div>
              </F>
            )}
          </div>
        </div>
      </BrowserChrome>
    </div>
  )
}

/* ------------------------------------------------------------------ Pairing per QR */

export function QrPairing({ state }: { state: string }) {
  const confirm = state === 'confirm'
  const tailscale = state === 'local-tailscale'
  return (
    <SettingsLayout
      active="devices"
      connection={tailscale ? 'local' : 'server'}
      overlay={
        <InlineDialog
          title={confirm ? 'Pixel 9 koppeln?' : tailscale ? 'Handy mit diesem Rechner verbinden' : 'Handy koppeln'}
          waiting={confirm}
          width="w-[540px]"
          footer={
            confirm ? (
              <>
                <Btn>Ablehnen</Btn>
                <PrimaryButton waiting>Pixel 9 koppeln</PrimaryButton>
              </>
            ) : (
              <Btn tone="quiet">Abbrechen</Btn>
            )
          }
        >
          {confirm ? (
            <F id="AUTH-009" className="space-y-2">
              <p>Ein Handy hat den QR-Code gescannt. Erst wenn du hier bestätigst, bekommt es Zugang.</p>
              <KV k="Name">Pixel 9</KV>
              <KV k="Plattform">Android 16 · Chrome (PWA)</KV>
              <KV k="Rechte">Sessions lesen, Eingaben senden, Freigaben erteilen</KV>
              <p className="text-[12px] text-muted-foreground">Admin-Rechte sind für Handys nicht wählbar.</p>
            </F>
          ) : tailscale ? (
            <F id={['AUTH-009', 'AUTH-001']} className="space-y-3">
              <p>
                Dein beton läuft nur auf diesem Rechner. Damit das Handy ihn erreicht, muss er zusätzlich auf deiner Tailscale-Adresse lauschen – nur mit TLS.
              </p>
              <Callout tone="deny" title="Ohne TLS nicht möglich">
                <p>
                  <C>server.listen: 100.101.7.12:7420</C> ist gesetzt, aber kein TLS. Wähle <C>tls: tailscale</C> (Zertifikat von Tailscale) oder hinterlege
                  Zertifikat und Schlüssel.
                </p>
              </Callout>
              <div className="flex gap-2">
                <PrimaryButton>TLS über Tailscale einschalten</PrimaryButton>
              </div>
              <p className="text-[12px] text-muted-foreground">Über diese Adresse gilt nur das Token gekoppelter Geräte, nie das lokale Token.</p>
            </F>
          ) : (
            <F id="AUTH-009" className="flex gap-5">
              <FakeQr />
              <div className="space-y-2">
                <p>Scanne den Code mit der Kamera deines Handys. Danach bestätigst du hier.</p>
                <p className="text-[12px] text-muted-foreground">Gültig 2 Minuten, nur einmal. Der Code steht im Fragment der Adresse und landet nicht in Server-Logs.</p>
                <p className="font-mono text-[12px] text-muted-foreground">noch 1:42</p>
              </div>
            </F>
          )}
        </InlineDialog>
      }
    >
      <DeviceTable local={tailscale} />
    </SettingsLayout>
  )
}

export function QrPairingPhone({ state }: { state: string }) {
  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex h-11 items-center border-b border-border px-4">
        <span className="type-wide text-[15px] font-bold">beton</span>
        <span className="ml-auto text-[11px] text-muted-foreground">beton.example.com</span>
      </div>
      <F id="AUTH-009" className="flex flex-1 flex-col gap-4 p-5">
        {state === 'name' && (
          <>
            <p className="type-wide text-[17px] font-semibold">Dieses Handy koppeln</p>
            <label className="text-[13px]">
              Gerätename
              <span className="mt-1 flex h-10 items-center rounded-md border border-ring bg-card px-3 outline-2 outline-ring">Pixel 9</span>
            </label>
            <p className="text-[12px] text-muted-foreground">So erscheint das Handy in deiner Geräteliste. Du kannst es dort jederzeit widerrufen.</p>
            <button className="mt-auto h-11 rounded-md bg-foreground text-[14px] font-semibold text-background">Weiter</button>
          </>
        )}
        {state === 'waiting' && (
          <div className="chamfer mt-6 border-l-4 border-signal bg-signal-soft p-4">
            <p className="text-[15px] font-semibold">Bestätige am Rechner</p>
            <p className="mt-1 text-[13px]">Auf „MacBook Ingo“ wartet eine Rückfrage: „Pixel 9 koppeln?“</p>
            <p className="mt-3 font-mono text-[12px] text-muted-foreground">läuft ab in 1:31</p>
          </div>
        )}
        {state === 'done' && (
          <>
            <Callout tone="ok" title="Gekoppelt">
              <p>Du siehst jetzt deine Sessions und kannst Freigaben unterwegs erteilen.</p>
            </Callout>
            <button className="mt-auto h-11 rounded-md bg-foreground text-[14px] font-semibold text-background">Zur Inbox</button>
          </>
        )}
      </F>
    </div>
  )
}

/* ------------------------------------------------------------------ Geräteverwaltung */

const devices = [
  { icon: Laptop, name: 'MacBook Ingo', kind: 'Desktop-App', pf: 'macOS 26 · arm64', created: '12.08.2026', seen: 'jetzt', ip: '100.101.7.12', scopes: 'alle deine Rechte', current: true },
  { icon: SquareTerminal, name: 'ingo-mbp (CLI)', kind: 'CLI/TUI', pf: 'macOS 26', created: '12.08.2026', seen: 'vor 4 Min.', ip: '100.101.7.12', scopes: 'alle deine Rechte' },
  { icon: Server, name: 'build-box-01', kind: 'Host', pf: 'Linux x86_64', created: 'heute', seen: 'vor 1 Min.', ip: '203.0.113.24', scopes: 'hosts:connect', host: true },
  { icon: Smartphone, name: 'Pixel 9', kind: 'Handy (PWA)', pf: 'Android 16', created: 'heute', seen: 'vor 22 Min.', ip: '100.64.12.9', scopes: 'sessions:read, write, approve' },
  { icon: Monitor, name: 'Büro-PC', kind: 'Desktop-App', pf: 'Windows 11', created: '02.06.2026', seen: 'vor 41 Tagen', ip: '10.20.4.51', scopes: 'alle deine Rechte', stale: true },
]

function DeviceTable({ local, notice }: { local?: boolean; notice?: ReactNode }) {
  const list = local ? devices.filter((d) => d.current || d.kind === 'Handy (PWA)') : devices
  return (
    <>
      <PageHead
        title="Geräte & Hosts"
        sub="Alles, was mit deinem Konto gekoppelt ist. Widerruf wirkt sofort, auch auf offene Verbindungen."
        actions={
          <>
            <Btn>Host koppeln …</Btn>
            <PrimaryButton>Handy koppeln</PrimaryButton>
          </>
        }
      >
        <div className="mt-2">
          {local ? <Premise>Lokaler Modus: nur dieser Rechner und gekoppelte Handys</Premise> : <Premise kind="team">Team-Server beton.example.com</Premise>}
        </div>
      </PageHead>
      <Scroll className="px-6 py-3">
        {notice && <div className="mb-3">{notice}</div>}
        <F id={['AUTH-010', 'AUTH-011']}>
          <div className="grid grid-cols-[minmax(0,1.3fr)_110px_120px_90px_100px_100px_minmax(0,1fr)_96px] gap-3 border-b border-border pb-1 text-[11px] text-muted-foreground">
            <span>Gerät</span>
            <span>Art</span>
            <span>Plattform</span>
            <span>Gekoppelt</span>
            <span>Zuletzt</span>
            <span>Letzte IP</span>
            <span>Rechte</span>
            <span />
          </div>
          {list.map((d) => (
            <div key={d.name} className="border-b border-border/60 py-1.5">
              <div className="grid grid-cols-[minmax(0,1.3fr)_110px_120px_90px_100px_100px_minmax(0,1fr)_96px] items-center gap-3 text-[12px]">
                <span className="flex min-w-0 items-center gap-2">
                  <d.icon className="size-4 shrink-0 text-muted-foreground" />
                  <span className="truncate font-medium">{d.name}</span>
                  {d.current && <Tag mono={false}>dieses Gerät</Tag>}
                </span>
                <span>{d.kind}</span>
                <span className="text-muted-foreground">{d.pf}</span>
                <span className="text-muted-foreground">{d.created}</span>
                <span className={cn(d.stale ? 'text-foreground' : 'text-muted-foreground')}>{d.seen}</span>
                <span className="font-mono text-[11px] text-muted-foreground">{d.ip}</span>
                <span className="truncate font-mono text-[11px]">{d.scopes}</span>
                <span className="text-right">{!d.current && <Btn tone="deny">Widerrufen</Btn>}</span>
              </div>
              {d.host && !local && (
                <div className="mt-1 ml-6 text-[12px] text-muted-foreground">
                  Nimmt Sessions an von: <b className="text-foreground">nur dir</b> · 2 laufende Sessions · Runner erhalten je Session ein Token (15 Min., erneuerbar, nie im
                  Environment)
                </div>
              )}
            </div>
          ))}
          <p className="mt-2 text-[12px] text-muted-foreground">Gerätetokens liegen im Schlüsselbund des Geräts und erneuern sich alle 30 Tage selbst.</p>
        </F>
      </Scroll>
    </>
  )
}

export function Devices({ state }: { state: string }) {
  return (
    <SettingsLayout
      active="devices"
      connection="server"
      overlay={
        state === 'revoke' ? (
          <InlineDialog
            title="build-box-01 widerrufen?"
            footer={
              <>
                <Btn tone="quiet">Abbrechen</Btn>
                <Btn tone="deny">Host widerrufen</Btn>
              </>
            }
          >
            <p>Der Host verliert sofort den Zugang. Seine Verbindung und alle Runner-Verbindungen werden binnen Sekunden getrennt.</p>
            <p>
              <b>2 laufende Sessions</b> werden gestoppt (Grund <C>host_revoked</C>): „Release-Notes generieren“, „Nightly e2e“. Ihr Verlauf bleibt erhalten.
            </p>
            <p className="text-[12px] text-muted-foreground">Wird im Audit-Log vermerkt. Zum erneuten Koppeln brauchst du einen neuen Code.</p>
          </InlineDialog>
        ) : undefined
      }
    >
      <DeviceTable
        notice={
          state === 'revoked' ? (
            <Callout tone="ok" title="build-box-01 widerrufen">
              <p>Verbindung getrennt, 2 Sessions gestoppt. Das alte Token erhält überall 401 token_revoked.</p>
            </Callout>
          ) : undefined
        }
      />
    </SettingsLayout>
  )
}
