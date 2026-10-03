import { Copy, Eye, GitFork, LogOut, Smartphone, Users } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge, StatusMark } from '@/app/harness'
import { WorkspaceRail } from '@/app/session-chrome'
import { AgentMessage, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { ShortStream } from '../workspace/bits'
import { Avatar, Btn, Callout, DialogPanel, FakeInput, FakeSelect, Menu, MenuItem, NetNote, Overlay, PageHeader, SessionShell, Tag } from '../workspace/parts'
import { roleHint, roleLabel, shares, teamServer, type Role } from './mock'

function Toggle({ on, label }: { on?: boolean; label: string }) {
  return (
    <span role="switch" aria-checked={on} aria-label={label} className={cn('inline-flex h-4 w-7 shrink-0 items-center rounded-full border p-px', on ? 'border-foreground bg-foreground' : 'border-input bg-muted')}>
      <span className={cn('size-3 rounded-full', on ? 'translate-x-3 bg-background' : 'bg-muted-foreground')} />
    </span>
  )
}

function ShareDialog({ viewOnly }: { viewOnly?: boolean }) {
  return (
    <F id={['COL-001', 'COL-002']}>
      <DialogPanel
        width="w-[600px]"
        title="„Rate-Limiter für die Login-API“ teilen"
        subtitle="Wer Zugriff hat, sieht die Session live. Du kannst Freigaben jederzeit ändern oder beenden; das wirkt sofort."
        footer={
          <>
            <Btn>
              <Copy className="size-3.5" /> Link kopieren
            </Btn>
            <span className="text-[12px] text-muted-foreground">Der Link funktioniert nur für Personen mit Freigabe.</span>
            <Btn variant="primary" className="ml-auto">
              Fertig
            </Btn>
          </>
        }
      >
        <div className="relative flex gap-2">
          <FakeInput value="mara" focus className="flex-1" />
          <FakeSelect value={viewOnly ? roleLabel.view : roleLabel.comment_approve} className="w-56" />
          <Btn variant="primary">Einladen</Btn>
          <div className="absolute top-9 left-0">
            <Menu className="w-80">
              <MenuItem active icon={<Avatar name="Mara Schulz" size="sm" />} hint="@mara · Team Plattform">
                Mara Schulz
              </MenuItem>
              <MenuItem icon={<Users className="size-3.5" />} hint="Team · 4 Personen">
                Marketing-Analytics
              </MenuItem>
            </Menu>
          </div>
        </div>

        {viewOnly && (
          <Callout className="mt-3">
            Auf {teamServer} ist nur <span className="font-medium">Ansehen</span> erlaubt. Kommentieren und Mitsteuern haben die Admins des Team-Servers abgeschaltet.
          </Callout>
        )}

        <div className="mt-3 border-t border-border">
          {shares.map((s) => (
            <div key={s.who} className="flex items-center gap-3 border-b border-border py-2">
              {s.team ? (
                <span className="flex size-6 items-center justify-center rounded-full border border-border">
                  <Users className="size-3.5 text-muted-foreground" />
                </span>
              ) : (
                <Avatar name={s.who} />
              )}
              <span className="min-w-0 flex-1">
                <span className="block text-[13px]">{s.who}</span>
                <span className="block text-[11px] text-muted-foreground">{s.sub}</span>
              </span>
              {s.role === 'owner' ? (
                <span className="text-[12px] text-muted-foreground">Owner</span>
              ) : (
                <>
                  <FakeSelect value={roleLabel[s.role]} className={cn('h-7 w-52 text-xs', viewOnly && s.role !== 'view' && 'border-deny/50')} />
                  <Btn size="sm" variant="ghost">
                    Entfernen
                  </Btn>
                </>
              )}
            </div>
          ))}
        </div>

        <dl className="mt-4 grid grid-cols-[170px_1fr] gap-x-3 gap-y-1.5 text-[12px]">
          {(Object.keys(roleLabel) as Role[]).map((r) => (
            <div key={r} className={cn('contents', viewOnly && r !== 'view' && 'opacity-50')}>
              <dt className="font-medium">{roleLabel[r]}</dt>
              <dd className="text-muted-foreground">
                {roleHint[r]}
                {r === 'drive' && <span className="text-foreground"> Führt Befehle auf deinem Rechner aus.</span>}
              </dd>
            </div>
          ))}
        </dl>

        <label className="mt-4 flex items-start gap-2.5 border-t border-border pt-3 text-[13px]">
          <Toggle label="Dateien für Ansehen freigeben" />
          <span>
            Personen mit „Ansehen“ dürfen Dateien und Diffs sehen
            <span className="block text-[12px] text-muted-foreground">Aus: Sie sehen den Verlauf, aber nicht den Inhalt des Worktrees.</span>
          </span>
        </label>

        <NetNote className="mt-4">Geteilt über den Team-Server {teamServer} (optional). Die Session selbst läuft weiter auf diesem Rechner.</NetNote>
      </DialogPanel>
    </F>
  )
}

function DriveWarning() {
  return (
    <F id="COL-002">
      <DialogPanel
        waiting
        role="alertdialog"
        title="Anna Becker darf mitsteuern?"
        footer={
          <>
            <Btn variant="signal">Mitsteuern erlauben</Btn>
            <Btn>Nur Kommentieren &amp; freigeben</Btn>
            <Btn variant="ghost" className="ml-auto">
              Abbrechen
            </Btn>
          </>
        }
      >
        <ul className="flex list-disc flex-col gap-1.5 pl-5 text-[13px]">
          <li>Anna kann den Agent Befehle ausführen lassen. Sie laufen auf deinem Rechner (ingos-mbp), in der Sandbox dieser Session.</li>
          <li>Ihre Nachrichten laufen über deine Claude-Max-Anmeldung und zählen zu deinem Kontingent.</li>
          <li>Sie kann Terminals öffnen und Freigaben erteilen. Policies gelten für sie genauso wie für dich.</li>
        </ul>
        <label className="mt-3 flex items-center gap-2 text-[13px]">
          <span role="checkbox" aria-checked className="flex size-3.5 items-center justify-center rounded-[3px] border border-foreground bg-foreground text-[10px] text-background">
            ✓
          </span>
          Verstanden: Mitsteuern bedeutet Code-Ausführung auf meinem Rechner.
        </label>
      </DialogPanel>
    </F>
  )
}

function LocalOnly() {
  return (
    <F id={['COL-001', 'COL-002']}>
      <DialogPanel
        title="Teilen mit anderen braucht einen Team-Server"
        footer={
          <>
            <Btn variant="primary">Team-Server verbinden …</Btn>
            <Btn>
              <Smartphone className="size-3.5" /> Eigene Geräte koppeln
            </Btn>
            <Btn variant="ghost" className="ml-auto">
              Schließen
            </Btn>
          </>
        }
      >
        <p className="text-[13px]">
          beton läuft gerade nur auf diesem Rechner. Deine eigenen Geräte, etwa das Handy über Tailscale oder WLAN, koppelst du ohne Server. Um Sessions mit Kolleg:innen zu
          teilen, verbindest du beton mit eurem Team-Server.
        </p>
        <NetNote className="mt-3">Optional. Ohne Team-Server verlässt nichts deinen Rechner außer den Anfragen der CLIs an ihre Modell-Anbieter.</NetNote>
      </DialogPanel>
    </F>
  )
}

export function ShareScreen({ state }: { state: string }) {
  const local = state === 'local-only'
  return (
    <SessionShell
      connection={local ? 'local' : 'server'}
      overlay={
        <Overlay>
          {state === 'drive-warning' ? <DriveWarning /> : local ? <LocalOnly /> : <ShareDialog viewOnly={state === 'view-only'} />}
        </Overlay>
      }
    >
      <ShortStream />
    </SessionShell>
  )
}

const sharedWithMe = [
  { title: 'Checkout-Formular barrierefrei machen', owner: 'Anna Becker', role: 'drive' as Role, harness: 'claude' as const, status: 'waiting' as const, updated: 'vor 2 Min.', unread: true },
  { title: 'Preisberechnung für Bundles', owner: 'Jonas Weber', role: 'comment_approve' as Role, harness: 'codex' as const, status: 'running' as const, updated: 'vor 14 Min.' },
  { title: 'Incident 2024-09-30: Timeouts im Checkout', owner: 'Team Plattform', role: 'view' as Role, harness: 'claude' as const, status: 'idle' as const, updated: 'gestern' },
]

export function SharedWithMeScreen({ state }: { state: string }) {
  if (state === 'list') {
    return (
      <AppLayout sessionList={false} connection="server">
        <PageHeader title="Mit mir geteilt" subtitle={`Sessions anderer, die du über ${teamServer} siehst. Sie laufen auf den Rechnern ihrer Owner.`} />
        <F id={['COL-002', 'COL-001']} className="px-6 py-2">
          {sharedWithMe.map((s) => (
            <div key={s.title} className="flex items-center gap-3 border-b border-border py-2.5">
              <StatusMark status={s.status} />
              <span className="min-w-0 flex-1">
                <span className={cn('block truncate text-[13px]', s.unread && 'font-semibold')}>{s.title}</span>
                <span className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
                  <Avatar name={s.owner} size="sm" /> {s.owner} · {s.updated}
                </span>
              </span>
              <HarnessBadge id={s.harness} />
              <Tag tone="muted" className="w-44 justify-center">
                Du: {roleLabel[s.role]}
              </Tag>
              <Btn size="sm" variant="ghost">
                <LogOut className="size-3.5" /> Freigabe verlassen
              </Btn>
            </div>
          ))}
        </F>
      </AppLayout>
    )
  }

  return (
    <SessionShell
      title="Incident 2024-09-30: Timeouts im Checkout"
      branch="main"
      connection="server"
      activeSession="shared"
      headerExtra={
        <F id="COL-001" as="span" badge="bottom-left">
          <Tag tone="muted">
            <Eye className="size-3" /> Du kannst ansehen
          </Tag>
        </F>
      }
      rail={
        <WorkspaceRail active="changes">
          <div className="p-4">
            <Callout>Team Plattform hat Dateien und Diffs für „Ansehen“ nicht freigegeben. Den Verlauf siehst du vollständig.</Callout>
          </div>
        </WorkspaceRail>
      }
      composer={
        <F id="COL-001" className="flex shrink-0 items-center gap-3 border-t border-border px-4 py-3 text-[13px]">
          <span className="flex-1 text-muted-foreground">Du siehst diese Session nur an. Schreiben, kommentieren und freigeben kann, wem Team Plattform mehr Rechte gibt.</span>
          <Btn>
            <GitFork className="size-3.5" /> In eigene Session forken
          </Btn>
        </F>
      }
      overlay={
        state === 'revoked' ? (
          <Overlay align="center">
            <F id="COL-001">
              <DialogPanel
                width="w-[440px]"
                title="Freigabe beendet"
                footer={
                  <Btn variant="primary" className="ml-auto">
                    Zu meinen Sessions
                  </Btn>
                }
              >
                <p className="text-[13px]">Team Plattform hat die Freigabe für „Incident 2024-09-30“ beendet. Du bist nicht mehr verbunden, und die Session verschwindet aus deiner Liste.</p>
              </DialogPanel>
            </F>
          </Overlay>
        ) : undefined
      }
    >
      <UserMessage author="Lena Krüger">Schau dir die Traces von 14:00 bis 14:30 an. Woher kommen die Timeouts beim Payment-Provider?</UserMessage>
      <ToolCall kind="search" name="Suche" target="rg 'ETIMEDOUT' logs/2024-09-30/" duration="0,4 s" />
      <AgentMessage harness="claude">
        <p>
          Die Timeouts entstehen im Connection-Pool: Er ist auf 10 Verbindungen begrenzt, und der Payment-Provider antwortete ab 14:07 langsamer als 8 s. Danach warteten neue
          Anfragen auf eine freie Verbindung.
        </p>
      </AgentMessage>
    </SessionShell>
  )
}

