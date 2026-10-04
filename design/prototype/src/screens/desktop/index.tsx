import type { ReactNode } from 'react'
import {
  Check,
  Copy,
  FileImage,
  KeyRound,
  Laptop,
  Link2,
  Minus,
  Paperclip,
  Plus,
  RefreshCw,
  Server,
  ShieldAlert,
  Square,
  TriangleAlert,
  X,
} from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { StatusMark } from '@/app/harness'
import { Composer, SessionHeader } from '@/app/session-chrome'
import { AgentMessage, Diff, TerminalOutput, ToolCall, UserMessage } from '@/app/stream'
import { SettingsFrame } from '@/app/settings-shell'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { rateLimiterDiff, type SessionStatus } from '@/mock/data'
import {
  BetonGlyph,
  CountBadge,
  DesktopScene,
  EditorWindow,
  InlineDialog,
  Notification,
  SceneWindow,
  Segmented,
  SettingRow,
  SettingsHeader,
  SettingsSection,
} from './scene'
import { PushToTalk, VoiceSettings } from './voice'

/*
 * Gruppe „Desktop-App“ (Tauri 2): alles, was die native Hülle über die Web-UI hinaus kann.
 * Prämisse: Die App startet einen lokalen Daemon auf diesem Rechner; Server-Profile,
 * Updates und Downloads sind optional und passieren nur auf Klick.
 */

/* ------------------------------------------------------------------------------------------ */
/* Gemeinsame Mini-Bausteine                                                                   */
/* ------------------------------------------------------------------------------------------ */

function MenuPanel({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn('w-[340px] rounded-lg border border-border bg-popover p-1 text-[12.5px] shadow-xl', className)}>{children}</div>
}

function MenuLabel({ children }: { children: ReactNode }) {
  return <div className="px-2 pt-1.5 pb-0.5 text-[11px] text-muted-foreground">{children}</div>
}

function MenuItem({ children, kbd, muted, onAccent }: { children: ReactNode; kbd?: string; muted?: boolean; onAccent?: boolean }) {
  return (
    <div className={cn('flex items-center gap-2 rounded-md px-2 py-1', onAccent ? 'bg-accent' : 'hover:bg-accent', muted && 'text-muted-foreground')}>
      {children}
      {kbd && <span className="ml-auto font-mono text-[11px] text-muted-foreground">{kbd}</span>}
    </div>
  )
}

function Sep() {
  return <div className="my-1 h-px bg-border" />
}

/** Eine Session in kompakter Form für Fenster in Szenen. */
function MiniSession({ title, status, children }: { title: string; status: SessionStatus; children: ReactNode }) {
  return (
    <AppLayout sessionList={false}>
      <SessionHeader title={title} harness="claude" status={status} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="flex flex-col gap-3 px-4 py-4">{children}</div>
      </div>
    </AppLayout>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Tray & Dock                                                                                 */
/* ------------------------------------------------------------------------------------------ */

function TrayScreen({ state }: { state: string }) {
  const stopped = state === 'daemon-stopped'
  const tray = (
    <MenuPanel>
      <F id="DESK-002" className="flex items-center gap-2 px-2 py-1.5" badge="top-right">
        <BetonGlyph className="size-5 text-[11px]" />
        <div className="min-w-0 flex-1">
          <div className="font-semibold">beton</div>
          <div className="text-[11px] text-muted-foreground">
            {stopped ? 'Lokaler Daemon gestoppt – Sessions pausieren' : 'Lokaler Daemon läuft · seit 3 Std. · v0.1.0'}
          </div>
        </div>
        {stopped ? <span className="size-2 rounded-[1px] bg-muted-foreground/60" title="gestoppt" /> : <span className="size-2 rounded-full bg-ok" title="läuft" />}
      </F>
      <Sep />
      {stopped ? (
        <F id="DESK-002" className="px-2 py-2">
          <p className="text-muted-foreground">Ohne Daemon laufen keine Agents. Deine Sessions sind gespeichert und laufen nach dem Start weiter.</p>
          <button className="mt-2 rounded-md bg-foreground px-3 py-1 font-semibold text-background">Daemon starten</button>
        </F>
      ) : (
        <>
          <F id="DESK-004" badge="top-right">
            <MenuLabel>Wartet auf dich</MenuLabel>
            <MenuItem>
              <StatusMark status="waiting" />
              <span className="min-w-0 flex-1 truncate">Rate-Limiter für die Login-API</span>
              <span className="text-[11px] text-muted-foreground">git push freigeben</span>
            </MenuItem>
            <MenuItem>
              <StatusMark status="waiting" />
              <span className="min-w-0 flex-1 truncate">Migration Bestellungen</span>
              <F id="DESK-003" as="span" badge="bottom-right">
                <span className="text-[11px] text-muted-foreground">Frage · team.example.com</span>
              </F>
            </MenuItem>
            <MenuLabel>Läuft</MenuLabel>
            <MenuItem>
              <StatusMark status="running" />
              <span className="min-w-0 flex-1 truncate">Review: Rate-Limiter</span>
              <span className="text-[11px] text-muted-foreground">Codex</span>
            </MenuItem>
            <MenuItem>
              <StatusMark status="running" />
              <span className="min-w-0 flex-1 truncate">Nächtliches Dependency-Update</span>
              <span className="text-[11px] text-muted-foreground">im Hintergrund</span>
            </MenuItem>
          </F>
        </>
      )}
      <Sep />
      <MenuItem kbd="⌘N">Neue Session…</MenuItem>
      <MenuItem>Fenster öffnen</MenuItem>
      <MenuItem kbd="⌘⇧Leer">Diktieren (gedrückt halten)</MenuItem>
      <Sep />
      <F id="DESK-002" badge="top-right">
        <MenuLabel>Daemon · 127.0.0.1, nur dieser Rechner</MenuLabel>
        <MenuItem muted={stopped}>Neu starten</MenuItem>
        <MenuItem muted={stopped}>Stoppen</MenuItem>
        <MenuItem>Logs anzeigen</MenuItem>
      </F>
      <Sep />
      <MenuItem kbd="⌘Q">beton beenden</MenuItem>
    </MenuPanel>
  )
  return (
    <DesktopScene app="Editor" menus={['Ablage', 'Bearbeiten', 'Auswahl', 'Ansicht', 'Fenster', 'Hilfe']} tray={tray} trayCount={stopped ? 0 : 3} dockCount={stopped ? 0 : 3}>
      <EditorWindow style={{ top: 52, left: 60, width: '58%', bottom: 100 }} />
      <F id="DESK-004" className="absolute bottom-24 left-[60%] z-30 max-w-[300px] rounded-md border border-border bg-card/90 p-2.5 text-[11.5px] text-muted-foreground backdrop-blur" badge="top-left">
        <p>
          Der Zähler am Tray und im Dock zeigt offene Freigaben plus ungelesene Sessions – über alle verbundenen Profile. Klick auf eine Session
          öffnet ihr Fenster.
        </p>
      </F>
    </DesktopScene>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Notifications                                                                               */
/* ------------------------------------------------------------------------------------------ */

function NotificationsScreen({ state }: { state: string }) {
  if (state === 'focused') {
    return (
      <DesktopScene trayCount={2} dockCount={2}>
        <SceneWindow title="Review: Rate-Limiter — beton" style={{ top: 48, left: 60, right: 60, bottom: 90 }}>
          <MiniSession title="Review: Rate-Limiter" status="idle">
            <UserMessage>Bitte reviewe den Rate-Limiter-Branch.</UserMessage>
            <AgentMessage harness="codex">
              <p>Zwei Anmerkungen: Der Bucket-Speicher wächst unbegrenzt, und der Test für den Retry-After-Header fehlt.</p>
            </AgentMessage>
          </MiniSession>
        </SceneWindow>
        <F id="DESK-005" className="absolute top-10 right-4 z-40 w-[350px] rounded-xl border border-dashed border-muted-foreground/50 p-3 text-[12px] text-muted-foreground" badge="bottom-left">
          Keine Benachrichtigung: Du siehst „Review: Rate-Limiter“ gerade in einem fokussierten Fenster. Die Inbox zeigt das Turn-Ende trotzdem.
        </F>
      </DesktopScene>
    )
  }
  const preview = state !== 'no-preview'
  return (
    <DesktopScene app="Editor" menus={['Ablage', 'Bearbeiten', 'Auswahl', 'Ansicht', 'Fenster', 'Hilfe']} trayCount={4} dockCount={4}>
      <EditorWindow style={{ top: 52, left: 60, width: '60%', bottom: 100 }} />
      <F id="DESK-005" className="absolute top-10 right-4 z-40 flex flex-col gap-2" badge="bottom-left">
        <Notification
          waiting
          title="Rate-Limiter für die Login-API"
          body={preview ? 'Freigabe nötig: git push -u origin beton/rate-limiter-7f3k' : 'Wartet auf dich'}
          actions={['Öffnen']}
        />
        <Notification title="Review: Rate-Limiter" time="vor 1 Min." body={preview ? 'Fertig: Zwei Anmerkungen – Bucket-Speicher wächst unbegrenzt …' : 'Turn beendet'} />
        <Notification title="Nächtliches Dependency-Update" time="vor 4 Min." body={preview ? 'Runner getrennt: Host „build-02“ antwortet nicht mehr.' : 'Runner getrennt'} />
      </F>
      {!preview && (
        <div className="absolute top-[330px] right-4 z-30 w-[350px] text-[11.5px] text-muted-foreground">
          Vorschau aus (<code className="font-mono">notifications.preview: false</code>): Benachrichtigungen zeigen nur Session und Ereignis, keinen
          Nachrichtentext.
        </div>
      )}
    </DesktopScene>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* App-Fenster                                                                                 */
/* ------------------------------------------------------------------------------------------ */

function ShellScreen({ state }: { state: string }) {
  if (state === 'two-windows') {
    return (
      <DesktopScene trayCount={3}>
        <F id="DESK-001" className="contents">
          <SceneWindow title="Rate-Limiter für die Login-API — beton" focused={false} style={{ top: 44, left: 24, width: '49%', bottom: 86 }}>
            <MiniSession title="Rate-Limiter für die Login-API" status="running">
              <ToolCall kind="edit" name="Bearbeiten" target="src/routes/auth.ts" duration="0,3 s" />
              <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth" status="running" />
              <AgentMessage harness="claude" streaming>
                <p>Ich ergänze den Test für den Retry-After-Header</p>
              </AgentMessage>
            </MiniSession>
          </SceneWindow>
          <SceneWindow title="Review: Rate-Limiter — beton" style={{ top: 60, right: 24, width: '49%', bottom: 76 }}>
            <AppLayout sessionList={false}>
              <SessionHeader title="Review: Rate-Limiter" harness="codex" status="running" />
              <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
                <div className="flex flex-col gap-3">
                  <ToolCall kind="git" name="Git" target="git diff main…beton/rate-limiter-7f3k" duration="0,1 s" />
                  <AgentMessage harness="codex" streaming>
                    <p>Der Bucket-Speicher wächst unbegrenzt; ich schlage eine LRU-Grenze vor</p>
                  </AgentMessage>
                </div>
              </div>
            </AppLayout>
          </SceneWindow>
        </F>
      </DesktopScene>
    )
  }

  const menu = (
    <MenuPanel className="w-[280px]">
      <MenuItem kbd="⌘N">Neues Fenster</MenuItem>
      <MenuItem kbd="⌘T">Neue Session</MenuItem>
      <MenuItem kbd="⇧⌘O">Projekt öffnen…</MenuItem>
      <Sep />
      <MenuItem kbd="⇧⌘R">Worktree im Finder zeigen</MenuItem>
      <MenuItem kbd="⌘O">Datei anhängen…</MenuItem>
      <Sep />
      <MenuItem kbd="⇧⌘E">Session exportieren…</MenuItem>
      <MenuItem kbd="⌘W">Fenster schließen</MenuItem>
    </MenuPanel>
  )

  return (
    <DesktopScene appMenu={state === 'menu' ? { title: 'Ablage', content: menu } : undefined} trayCount={3}>
      <SceneWindow title="Rate-Limiter für die Login-API — beton" style={{ top: 48, left: 60, right: 60, bottom: 90 }}>
        <div className="relative h-full">
          <AppLayout activeSession="ses_7f3k">
            <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="idle" branch="beton/rate-limiter-7f3k" />
            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto flex max-w-2xl flex-col gap-4 px-6 py-6">
                <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP.</UserMessage>
                <ToolCall kind="edit" name="Bearbeiten" target="src/routes/auth.ts" duration="0,3 s">
                  <Diff file="src/routes/auth.ts" lines={rateLimiterDiff} />
                </ToolCall>
                <AgentMessage harness="claude">
                  <p>Die Login-Route ist jetzt begrenzt. Alle 10 Tests laufen.</p>
                </AgentMessage>
              </div>
            </div>
            <Composer harness="claude" />
          </AppLayout>
          {state === 'drop' && (
            <F id="DESK-001" className="absolute inset-0 z-40 flex items-center justify-center bg-background/70 p-8">
              <div className="flex w-full max-w-md flex-col items-center gap-2 rounded-xl border-2 border-dashed border-foreground/50 bg-card/80 p-8 text-center">
                <Paperclip className="size-6 text-muted-foreground" />
                <p className="text-[15px] font-semibold">Loslassen zum Anhängen</p>
                <p className="text-[12.5px] text-muted-foreground">
                  login-fehler.png landet als Anhang im Composer von „Rate-Limiter für die Login-API“. Gesendet wird erst, wenn du sendest.
                </p>
              </div>
              <div className="absolute top-[58%] left-[56%] flex rotate-3 items-center gap-2 rounded-md border border-border bg-popover px-2.5 py-1.5 text-[12px] shadow-lg">
                <FileImage className="size-4 text-muted-foreground" /> login-fehler.png
              </div>
            </F>
          )}
        </div>
      </SceneWindow>
      {state === 'menu' && (
        <F id="DESK-001" className="absolute top-9 left-[330px] z-30 max-w-[300px] rounded-md border border-border bg-card/90 p-2.5 text-[11.5px] text-muted-foreground" badge="top-right">
          Natives Menü mit Standard-Bearbeiten-Menü (Kopieren, Einsetzen). Fenstergröße und -position merkt sich beton über Neustarts hinweg.
        </F>
      )}
    </DesktopScene>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Lokaler Daemon                                                                              */
/* ------------------------------------------------------------------------------------------ */

function StepLine({ done, active, children }: { done?: boolean; active?: boolean; children: ReactNode }) {
  return (
    <li className={cn('flex items-center gap-2', !done && !active && 'text-muted-foreground')}>
      {done ? (
        <Check className="size-3.5 text-ok" />
      ) : active ? (
        <RefreshCw className="size-3.5 animate-spin" />
      ) : (
        <span className="mx-[3px] size-2 rounded-full border border-muted-foreground" />
      )}
      {children}
    </li>
  )
}

function DaemonScreen({ state }: { state: string }) {
  if (state === 'starting') {
    return (
      <F id={['DESK-002', 'DESK-001']} className="concrete-grain flex h-full flex-col items-center justify-center gap-5 p-8">
        <BetonGlyph className="size-14 text-[30px]" />
        <p className="type-wide text-xl font-[700]">beton startet</p>
        <ul className="space-y-1.5 text-[13px]">
          <StepLine done>beton 0.1.0 gefunden (mitgeliefert)</StepLine>
          <StepLine done>Kein laufender Daemon – starte einen neuen</StepLine>
          <StepLine active>Lokaler Daemon auf 127.0.0.1, nur von diesem Rechner erreichbar</StepLine>
          <StepLine>Profil „local“ öffnen</StepLine>
        </ul>
        <p className="max-w-md text-center text-[12px] text-muted-foreground">Kein Konto, kein Server im Internet. Deine Sessions liegen unter ~/.beton/.</p>
      </F>
    )
  }

  const app = (
    <AppLayout activeSession="ses_7f3k">
      <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="running" branch="beton/rate-limiter-7f3k" />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-2xl flex-col gap-4 px-6 py-6">
          <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP.</UserMessage>
          <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth" status="running" />
        </div>
      </div>
      <Composer harness="claude" running />
    </AppLayout>
  )

  if (state === 'logs') {
    return (
      <div className="relative h-full">
        {app}
        <F id="DESK-002" className="absolute inset-x-10 top-10 bottom-10 z-40 flex flex-col overflow-hidden rounded-lg border border-border bg-popover shadow-xl">
          <div className="flex h-10 shrink-0 items-center gap-3 border-b border-border px-4 text-[13px]">
            <span className="font-semibold">Daemon-Logs</span>
            <span className="font-mono text-[11.5px] text-muted-foreground">~/.beton/logs/daemon.log</span>
            <span className="ml-auto flex gap-1.5">
              <button className="inline-flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-[12px]">
                <Copy className="size-3" /> Kopieren
              </button>
              <button aria-label="Schließen" className="flex size-6 items-center justify-center rounded-md hover:bg-accent">
                <X className="size-3.5" />
              </button>
            </span>
          </div>
          <div className="min-h-0 flex-1 overflow-auto p-3">
            <TerminalOutput>{`09:38:02.114  INFO beton_server: listening addr=127.0.0.1:7420 mode=local
09:38:02.131  INFO beton_host: connected server=local labels=os=macos,arch=arm64
09:38:02.140  INFO beton_server::proto: protocol version=1.2 client=desktop/0.1.0
09:39:47.902  INFO runner{session=ses_7f3k}: harness started harness=claude transport=native
09:40:12.377  INFO policy{session=ses_7f3k}: decision rule=git-push-fragen action=ask
09:41:05.008  WARN beton_voice: metal init failed, falling back to cpu
09:41:12.512  INFO runner{session=ses_7f3k}: tool.call.started tool=shell cmd="pnpm vitest run auth"`}</TerminalOutput>
          </div>
        </F>
      </div>
    )
  }

  return (
    <div className="relative h-full">
      {app}
      {state === 'existing' && (
        <F id="DESK-002" className="absolute bottom-3 left-14 z-40 w-[340px] rounded-lg border border-border bg-popover p-3 text-[12.5px] shadow-lg">
          <div className="flex items-center gap-2">
            <span className="size-2 rounded-full bg-ok" />
            <span className="font-semibold">Verbunden mit laufendem Daemon</span>
          </div>
          <dl className="mt-2 grid grid-cols-[110px_1fr] gap-y-0.5 text-[12px]">
            <dt className="text-muted-foreground">Gestartet von</dt>
            <dd>
              CLI (<code className="font-mono">beton serve</code>), PID 31877
            </dd>
            <dt className="text-muted-foreground">Adresse</dt>
            <dd className="font-mono">127.0.0.1:7420</dd>
            <dt className="text-muted-foreground">Version</dt>
            <dd>0.1.0 · Protokoll 1.2 · kompatibel</dd>
            <dt className="text-muted-foreground">Läuft seit</dt>
            <dd>2 Std. 14 Min.</dd>
          </dl>
          <p className="mt-2 text-[11.5px] text-muted-foreground">Die App startet keinen zweiten Daemon. Schließt du das Fenster, laufen Sessions weiter.</p>
          <div className="mt-2 flex gap-1.5">
            <button className="rounded-md border border-border px-2 py-0.5 text-[12px]">Neu starten</button>
            <button className="rounded-md border border-border px-2 py-0.5 text-[12px]">Stoppen</button>
            <button className="rounded-md border border-border px-2 py-0.5 text-[12px]">Logs anzeigen</button>
          </div>
        </F>
      )}
      {state === 'incompatible' && (
        <InlineDialog>
          <F id="DESK-002">
            <p className="text-[15px] font-semibold">Der laufende Daemon passt nicht zu dieser App</p>
            <p className="mt-1.5 text-[13px]">
              Daemon 0.1.0 (Protokoll 1.0) läuft noch, die App ist 0.3.2 (Protokoll 1.3). Der Abstand ist zu groß, um sicher zu verbinden.
            </p>
            <p className="mt-1.5 text-[12.5px] text-muted-foreground">
              beton kann den mitgelieferten Daemon 0.3.2 starten. Der Neustart wartet, bis keine Session mehr arbeitet; deine Sessions bleiben
              erhalten.
            </p>
            <div className="mt-4 flex flex-wrap gap-2">
              <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Daemon auf 0.3.2 aktualisieren</button>
              <button className="rounded-md border border-border px-3 py-1.5 text-[13px]">Beenden</button>
            </div>
          </F>
        </InlineDialog>
      )}
      {state === 'quit' && (
        <InlineDialog>
          <F id="DESK-002">
            <p className="text-[15px] font-semibold">beton beenden?</p>
            <p className="mt-1.5 text-[13px]">2 Sessions arbeiten gerade: „Rate-Limiter für die Login-API“ und „Nächtliches Dependency-Update“.</p>
            <div role="radiogroup" className="mt-3 space-y-2 text-[13px]">
              <label className="flex items-start gap-2">
                <span className="mt-1 flex size-3.5 items-center justify-center rounded-full border border-foreground">
                  <span className="size-1.5 rounded-full bg-foreground" />
                </span>
                <span>
                  Daemon weiterlaufen lassen <span className="text-muted-foreground">(empfohlen)</span>
                  <span className="block text-[12px] text-muted-foreground">Sessions laufen ohne Fenster weiter; Tray und Benachrichtigungen bleiben aus.</span>
                </span>
              </label>
              <label className="flex items-start gap-2">
                <span className="mt-1 size-3.5 rounded-full border border-muted-foreground" />
                <span>
                  Daemon stoppen
                  <span className="block text-[12px] text-muted-foreground">Laufende Turns werden abgebrochen; der Verlauf bleibt gespeichert.</span>
                </span>
              </label>
            </div>
            <div className="mt-4 flex gap-2">
              <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">beton beenden</button>
              <button className="rounded-md border border-border px-3 py-1.5 text-[13px]">Abbrechen</button>
            </div>
          </F>
        </InlineDialog>
      )}
    </div>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Deep-Links                                                                                  */
/* ------------------------------------------------------------------------------------------ */

function LinkLine({ children }: { children: ReactNode }) {
  return (
    <div className="mt-2 flex items-center gap-1.5 rounded-sm bg-sunken px-2 py-1 font-mono text-[11.5px] break-all">
      <Link2 className="size-3 shrink-0 text-muted-foreground" />
      {children}
    </div>
  )
}

function DeepLinkScreen({ state }: { state: string }) {
  return (
    <div className="relative h-full">
      <AppLayout activeSession="ses_7f3k">
        <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="idle" branch="beton/rate-limiter-7f3k" />
        {state === 'open' && (
          <F id="DESK-006" className="flex h-8 shrink-0 items-center gap-2 border-b border-border bg-card px-4 text-[12px]">
            <Link2 className="size-3.5 text-muted-foreground" />
            <span className="text-muted-foreground">Geöffnet über</span>
            <code className="font-mono text-[11.5px]">beton://local/s/ses_7f3k?seq=214</code>
            <span className="text-muted-foreground">· Event 214</span>
          </F>
        )}
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto flex max-w-2xl flex-col gap-4 px-6 py-6">
            <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP.</UserMessage>
            <ToolCall kind="read" name="Lesen" target="src/routes/auth.ts" duration="0,1 s" />
            <div className={cn(state === 'open' && 'rounded-md outline-2 outline-offset-4 outline-foreground/60')}>
              <ToolCall kind="edit" name="Bearbeiten" target="src/routes/auth.ts · Event 214" duration="0,3 s" defaultOpen>
                <Diff file="src/routes/auth.ts" lines={rateLimiterDiff} />
              </ToolCall>
            </div>
            <AgentMessage harness="claude">
              <p>Die Login-Route ist jetzt begrenzt. Alle 10 Tests laufen.</p>
            </AgentMessage>
          </div>
        </div>
        <Composer harness="claude" />
      </AppLayout>
      {state === 'unknown' && (
        <InlineDialog>
          <F id={['DESK-006', 'DESK-003']}>
            <p className="text-[15px] font-semibold">Link auf einen Server, den beton nicht kennt</p>
            <LinkLine>beton://beton.team.example.com/s/ses_9a1b?comment=c_42</LinkLine>
            <p className="mt-2 text-[13px]">
              Für <span className="font-medium">beton.team.example.com</span> gibt es noch kein Profil. Soll beton eins anlegen und dich dort anmelden?
            </p>
            <p className="mt-1 text-[12px] text-muted-foreground">Bis du zustimmst, baut beton keine Verbindung zu diesem Server auf.</p>
            <div className="mt-4 flex gap-2">
              <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Profil anlegen und anmelden</button>
              <button className="rounded-md border border-border px-3 py-1.5 text-[13px]">Abbrechen</button>
            </div>
          </F>
        </InlineDialog>
      )}
      {state === 'pair' && (
        <InlineDialog>
          <F id={['DESK-006', 'DESK-003']}>
            <p className="text-[15px] font-semibold">Mit einem anderen Rechner koppeln?</p>
            <LinkLine>beton://pair?server=https://ingo-laptop.tail5c2e.ts.net&amp;code=K7Q-4MX</LinkLine>
            <p className="mt-2 text-[13px]">Vergleiche den Code mit dem, den der andere Rechner gerade anzeigt:</p>
            <p className="type-wide mt-2 text-center font-mono text-2xl font-semibold tracking-widest">K7Q-4MX</p>
            <p className="mt-2 text-[12px] text-muted-foreground">
              Erst nach deiner Bestätigung verbindet sich beton mit ingo-laptop über dein Tailnet. Das Token landet im Schlüsselbund.
            </p>
            <div className="mt-4 flex gap-2">
              <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Code stimmt – koppeln</button>
              <button className="rounded-md border border-border px-3 py-1.5 text-[13px]">Abbrechen</button>
            </div>
          </F>
        </InlineDialog>
      )}
    </div>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Server-Profile                                                                              */
/* ------------------------------------------------------------------------------------------ */

type Profile = { name: string; url: string; auth: string; status: 'connected' | 'disconnected' | 'expired'; unread?: number; local?: boolean }

function ProfilesScreen({ state }: { state: string }) {
  const profiles: Profile[] = [
    { name: 'local', url: 'Dieser Rechner (127.0.0.1)', auth: 'kein Konto, kein Token', status: 'connected', unread: 3, local: true },
    { name: 'team', url: 'https://beton.team.example.com', auth: 'OIDC (Entra ID) · Token im Schlüsselbund', status: state === 'expired' ? 'expired' : 'connected', unread: 1 },
    { name: 'laptop', url: 'https://ingo-laptop.tail5c2e.ts.net', auth: 'Gerät gekoppelt · Token im Schlüsselbund', status: 'disconnected' },
  ]
  if (state === 'expired') {
    return (
      <AppLayout activeSession="ses_7f3k" connection="server">
        <F id="DESK-003" className="chamfer flex shrink-0 items-center gap-3 border-l-4 border-signal bg-signal-soft px-4 py-2 text-[12.5px]">
          <KeyRound className="size-4 shrink-0" />
          <span>
            <span className="font-semibold">Anmeldung bei beton.team.example.com abgelaufen.</span> Sessions dieses Servers siehst du wieder, sobald du
            dich neu anmeldest. Lokale Sessions laufen weiter.
          </span>
          <button className="chamfer-sm ml-auto shrink-0 bg-foreground px-3 py-1 font-semibold text-background">Im Browser neu anmelden</button>
        </F>
        <SessionHeader title="Migration Bestellungen" harness="claude" status="stopped" />
        <div className="concrete-grain flex flex-1 items-center justify-center p-8 text-center text-[13px] text-muted-foreground">
          Dieses Fenster gehört zum Profil „team“. Der Verlauf lädt nach der Anmeldung.
        </div>
      </AppLayout>
    )
  }
  return (
    <SettingsFrame active="servers">
      <SettingsHeader page="Server-Profile" />
      <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-8">
        <div className="max-w-3xl">
          <p className="py-4 text-[13px] text-muted-foreground">
            beton läuft vollständig auf diesem Rechner. Weitere Server – ein Team-Server oder dein zweiter Rechner – sind optional (ab M4). Jedes
            Fenster gehört zu genau einem Profil; Zähler und Benachrichtigungen fassen alle zusammen. Profile teilst du mit der CLI
            (<code className="font-mono">beton profile</code>); Tokens liegen nur im Schlüsselbund.
          </p>
          <F id="DESK-003" className="border-t border-border">
            {profiles.map((p) => (
              <div key={p.name} className="flex items-center gap-3 border-b border-border py-3">
                {p.local ? <Laptop className="size-4 text-muted-foreground" /> : <Server className="size-4 text-muted-foreground" />}
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 text-[13px]">
                    <span className="font-semibold">{p.name}</span>
                    {p.local && <span className="rounded-sm border border-border px-1 text-[10.5px] text-muted-foreground">Standard</span>}
                    <span className="truncate font-mono text-[11.5px] text-muted-foreground">{p.url}</span>
                  </div>
                  <div className="text-[11.5px] text-muted-foreground">{p.auth}</div>
                </div>
                {p.unread ? <CountBadge n={p.unread} /> : null}
                <span className="w-28 text-right text-[12px]">
                  {p.status === 'connected' && (
                    <span className="inline-flex items-center gap-1.5">
                      <span className="size-2 rounded-full bg-ok" /> verbunden
                    </span>
                  )}
                  {p.status === 'disconnected' && (
                    <span className="inline-flex items-center gap-1.5 text-muted-foreground">
                      <span className="size-2 rounded-full border border-muted-foreground" /> getrennt
                    </span>
                  )}
                </span>
                <button className="rounded-md border border-border px-2 py-0.5 text-[12px] hover:bg-accent">{p.status === 'disconnected' ? 'Verbinden' : 'Fenster öffnen'}</button>
              </div>
            ))}
          </F>
          {state === 'add' ? (
            <F id="DESK-003" className="mt-4 rounded-lg border border-border bg-card p-4">
              <p className="text-[13px] font-semibold">Profil hinzufügen</p>
              <div className="mt-3 grid grid-cols-[120px_1fr] items-center gap-x-3 gap-y-2 text-[13px]">
                <label className="text-muted-foreground">Name</label>
                <span className="rounded-md border border-input bg-background px-2 py-1">kunde-x</span>
                <label className="text-muted-foreground">Server-Adresse</label>
                <span className="rounded-md border border-ring bg-background px-2 py-1 font-mono text-[12px]">https://beton.kunde-x.example</span>
                <label className="text-muted-foreground">Anmeldung</label>
                <Segmented options={['Im Browser (OIDC)', 'Mit Code koppeln']} value="Im Browser (OIDC)" />
              </div>
              <p className="mt-3 text-[12px] text-muted-foreground">beton verbindet sich erst, wenn du auf „Verbinden“ klickst. Das Token wird im Schlüsselbund gespeichert.</p>
              <div className="mt-3 flex gap-2">
                <button className="rounded-md bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Verbinden</button>
                <button className="rounded-md border border-border px-3 py-1.5 text-[13px]">Abbrechen</button>
              </div>
            </F>
          ) : (
            <button className="mt-4 inline-flex items-center gap-1 rounded-md border border-border px-3 py-1.5 text-[13px] hover:bg-accent">
              <Plus className="size-3.5" /> Profil hinzufügen
            </button>
          )}
        </div>
      </div>
    </SettingsFrame>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Updates                                                                                     */
/* ------------------------------------------------------------------------------------------ */

function UpdateSettings({ state }: { state: string }) {
  return (
    <SettingsFrame active="updates">
      <SettingsHeader page="Updates" />
      <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-8">
        <div className="max-w-3xl">
          <F id="DESK-007" className="py-4">
            <div className="flex items-center gap-3 text-[13px]">
              <BetonGlyph className="size-9 text-[18px]" />
              <div>
                <div className="font-semibold">beton 0.1.0</div>
                <div className="text-[12px] text-muted-foreground">App und mitgelieferter Daemon · Kanal stable · zuletzt geprüft: nie</div>
              </div>
              <button className="ml-auto inline-flex items-center gap-1 rounded-md bg-foreground px-3 py-1.5 font-semibold text-background">
                <RefreshCw className="size-3.5" /> Jetzt nach Updates suchen
              </button>
            </div>
            <p className="mt-2 text-[11.5px] text-muted-foreground">
              Fragt die signierten Release-Manifeste auf github.com ab. Es werden keine Daten über dich oder deine Sessions gesendet.
            </p>
          </F>
          {state === 'bad-signature' && (
            <F id="DESK-007" className="mb-4 flex items-start gap-2 rounded-md border border-deny/40 bg-deny-soft p-3 text-[12.5px]">
              <ShieldAlert className="mt-0.5 size-4 shrink-0 text-deny" />
              <div>
                <p className="font-semibold">Update verworfen: Signatur ungültig</p>
                <p className="mt-0.5">
                  Das Manifest für 0.1.3 (Kanal stable) ist nicht mit dem Schlüssel von beton signiert. Nichts wurde geladen oder installiert; der
                  Vorgang steht im Log. Versuche es später erneut oder lade die Version selbst von der Release-Seite.
                </p>
              </div>
            </F>
          )}
          <SettingsSection title="Automatisch">
            <SettingRow label="Automatisch nach Updates suchen" hint="Beim Start und alle 6 Std. · verbindet sich dafür mit github.com · aus, bis du es einschaltest">
              <Switch aria-label="Automatisch suchen" />
            </SettingRow>
            <SettingRow label="Installieren" hint="Immer erst nach deiner Bestätigung. Der Daemon startet neu, wenn keine Session mehr arbeitet.">
              <span className="text-[12px] text-muted-foreground">nach Rückfrage</span>
            </SettingRow>
          </SettingsSection>
          <SettingsSection title="Kanal" hint="Ein Wechsel gilt ab der nächsten Prüfung.">
            <Segmented options={['stable', 'beta', 'nightly']} value="stable" />
          </SettingsSection>
        </div>
      </div>
    </SettingsFrame>
  )
}

function UpdateScreen({ state }: { state: string }) {
  if (state === 'settings' || state === 'bad-signature') return <UpdateSettings state={state} />
  return (
    <div className="relative h-full">
      <AppLayout activeSession="ses_7f3k">
        {state === 'drain' && (
          <F id="DESK-007" className="flex shrink-0 items-center gap-3 border-b border-border bg-card px-4 py-2 text-[12.5px]">
            <RefreshCw className="size-4 shrink-0 text-muted-foreground" />
            <span>
              <span className="font-semibold">beton 0.1.3 ist installiert.</span>{' '}
              <span className="text-muted-foreground">Der Daemon startet neu, sobald 2 laufende Sessions fertig sind.</span>
            </span>
            <span className="ml-auto flex shrink-0 gap-1.5">
              <button className="rounded-md border border-border px-2.5 py-1">Jetzt neu starten</button>
            </span>
          </F>
        )}
        <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="running" branch="beton/rate-limiter-7f3k" />
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto flex max-w-2xl flex-col gap-4 px-6 py-6">
            <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP.</UserMessage>
            <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth" status="running" />
          </div>
        </div>
        <Composer harness="claude" running />
      </AppLayout>
      {state === 'available' && (
        <InlineDialog>
          <F id="DESK-007">
            <p className="text-[15px] font-semibold">beton 0.1.3 ist verfügbar</p>
            <p className="mt-0.5 text-[12px] text-muted-foreground">Du hast 0.1.0 · Kanal stable · 38 MB von github.com</p>
            <ul className="mt-3 list-disc space-y-0.5 pl-5 text-[13px]">
              <li>Inspect-Mode: Mehrfachauswahl mit ⇧ + Klick</li>
              <li>Schnellerer Start des lokalen Daemons</li>
              <li>Behoben: Notifications nach Ruhezustand doppelt</li>
            </ul>
            <p className="mt-3 inline-flex items-center gap-1 text-[12px] text-ok">
              <Check className="size-3.5" /> Signatur geprüft
            </p>
            <p className="mt-1 text-[12px] text-muted-foreground">
              2 Sessions arbeiten gerade. Sie laufen weiter; der Daemon startet erst neu, wenn sie fertig sind – oder wenn du es sagst.
            </p>
            <div className="mt-4 flex gap-2">
              <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Installieren</button>
              <button className="rounded-md border border-border px-3 py-1.5 text-[13px]">Später</button>
            </div>
          </F>
        </InlineDialog>
      )}
    </div>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Windows (Beta)                                                                              */
/* ------------------------------------------------------------------------------------------ */

function WindowsChrome({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full min-h-[640px] flex-col overflow-hidden rounded-md border border-border bg-background shadow-lg">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border bg-sidebar pl-3 text-[12px]">
        <BetonGlyph className="size-4 text-[9px]" />
        <span>beton</span>
        <span className="rounded-sm border border-border px-1 text-[10.5px] text-muted-foreground">Beta</span>
        <span className="ml-auto flex h-full">
          {[Minus, Square, X].map((I, i) => (
            <span key={i} className="flex w-11 items-center justify-center text-muted-foreground hover:bg-accent">
              <I className="size-3.5" />
            </span>
          ))}
        </span>
      </div>
      <div className="min-h-0 flex-1">{children}</div>
    </div>
  )
}

function WindowsScreen({ state }: { state: string }) {
  return (
    <WindowsChrome>
      <AppLayout activeSession="ses_7f3k">
        <F id="DESK-009" className="flex shrink-0 items-center gap-3 border-b border-border bg-card px-4 py-2 text-[12.5px]">
          <TriangleAlert className="size-4 shrink-0 text-muted-foreground" />
          <span>
            <span className="font-semibold">Windows-Beta.</span>{' '}
            <span className="text-muted-foreground">
              Sandbox laut <code className="font-mono">beton doctor</code>: eingeschränkt (Restricted Token, AppContainer). Für volle Isolation nutze
              den Docker-Runner oder WSL2.
            </span>
          </span>
          <button className="ml-auto shrink-0 rounded-md border border-border px-2 py-0.5">Details</button>
        </F>
        {state === 'doctor' ? (
          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
            <F id="DESK-009" className="max-w-2xl">
              <h3 className="type-wide text-[15px] font-[650]">Was auf Windows (Beta) geht</h3>
              <p className="mt-0.5 text-[12.5px] text-muted-foreground">Stand aus beton doctor auf diesem Rechner · Windows 11 24H2 · x64 · WebView2 131</p>
              <ul className="mt-3 divide-y divide-border border-y border-border text-[13px]">
                {[
                  { f: 'Sessions, Freigaben, Browser, Push-to-Talk', s: 'ok', note: 'wie auf macOS und Linux' },
                  { f: 'Sandbox Stufe 1 (Harness)', s: 'degraded', note: 'Restricted Token + Job Object; Netzwerk ohne Proxy-Zwang' },
                  { f: 'Sandbox Stufe 2 (Tools)', s: 'degraded', note: 'AppContainer; ohne Netz, außer du erlaubst ungeprüftes Netz' },
                  { f: 'Tools mit Netz in der Sandbox', s: 'off', note: 'nicht verfügbar: AppContainer erreicht den Proxy nicht ohne Admin-Ausnahme' },
                ].map((r) => (
                  <li key={r.f} className={cn('flex items-center gap-3 py-2', r.s === 'off' && 'text-muted-foreground')}>
                    <span className="w-24 shrink-0 text-[12px]">
                      {r.s === 'ok' && (
                        <span className="inline-flex items-center gap-1 text-ok">
                          <Check className="size-3.5" /> verfügbar
                        </span>
                      )}
                      {r.s === 'degraded' && (
                        <span className="inline-flex items-center gap-1">
                          <TriangleAlert className="size-3.5" /> Beta
                        </span>
                      )}
                      {r.s === 'off' && (
                        <span className="inline-flex items-center gap-1">
                          <X className="size-3.5" /> aus
                        </span>
                      )}
                    </span>
                    <span className="flex-1">{r.f}</span>
                    <span className="w-[46%] text-[12px] text-muted-foreground">{r.note}</span>
                  </li>
                ))}
              </ul>
            </F>
          </div>
        ) : (
          <>
            <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="idle" branch="beton/rate-limiter-7f3k" />
            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto flex max-w-2xl flex-col gap-4 px-6 py-6">
                <UserMessage>Lass die Tests laufen und installiere vorher die Abhängigkeiten.</UserMessage>
                <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth" duration="5,2 s" />
                <F id="DESK-009" className="ml-9 rounded-md border border-border bg-card p-2.5 text-[12.5px] text-muted-foreground">
                  <span className="font-medium text-foreground">pnpm install</span> braucht Netz. Auf Windows (Beta) gibt es in der Tool-Sandbox kein
                  geprüftes Netz – der Agent hat stattdessen den vorhandenen Store genutzt.
                </F>
                <AgentMessage harness="claude">
                  <p>Alle 10 Tests laufen.</p>
                </AgentMessage>
              </div>
            </div>
            <Composer harness="claude" />
          </>
        )}
      </AppLayout>
    </WindowsChrome>
  )
}

export const group: ScreenGroup = {
  id: 'desktop',
  title: 'Desktop-App',
  order: 180,
  screens: [
    {
      id: 'desktop-tray',
      title: 'Tray & Dock',
      description:
        'beton im Tray: laufende Sessions, offene Freigaben, Schnellaktionen und der Status des lokalen Daemons. Dock-Badge und Tray zählen, was auf dich wartet – über alle Profile.',
      features: ['DESK-004', 'DESK-002', 'DESK-003'],
      states: [
        { id: 'default', title: 'Tray geöffnet' },
        { id: 'daemon-stopped', title: 'Daemon gestoppt' },
      ],
      frame: 'none',
      component: TrayScreen,
    },
    {
      id: 'desktop-notifications',
      title: 'Benachrichtigungen',
      description:
        'System-Benachrichtigungen nur für Sessions, die du gerade nicht siehst: Freigaben, Turn-Ende, getrennte Runner. Ein Klick öffnet die Session an der richtigen Stelle.',
      features: ['DESK-005'],
      states: [
        { id: 'unfocused', title: 'beton im Hintergrund' },
        { id: 'focused', title: 'Session sichtbar' },
        { id: 'no-preview', title: 'Ohne Vorschau' },
      ],
      frame: 'none',
      component: NotificationsScreen,
    },
    {
      id: 'desktop-shell',
      title: 'App-Fenster',
      description:
        'Die Tauri-App zeigt dieselbe UI wie das Web, ergänzt um natives Menü, mehrere Fenster und Dateien per Drag & Drop als Anhang.',
      features: ['DESK-001'],
      states: [
        { id: 'menu', title: 'Natives Menü' },
        { id: 'drop', title: 'Datei ablegen' },
        { id: 'two-windows', title: 'Zwei Fenster' },
      ],
      frame: 'none',
      component: ShellScreen,
    },
    {
      id: 'desktop-daemon',
      title: 'Lokaler Daemon',
      description:
        'Die App bringt beton mit und startet bei Bedarf den lokalen Daemon – oder verbindet sich mit einem, der schon läuft. Schließen des Fensters beendet keine Sessions.',
      features: ['DESK-002', 'DESK-001'],
      states: [
        { id: 'starting', title: 'Start' },
        { id: 'existing', title: 'Verbunden mit CLI-Daemon' },
        { id: 'incompatible', title: 'Daemon inkompatibel' },
        { id: 'quit', title: 'Beenden mit laufenden Sessions' },
        { id: 'logs', title: 'Logs' },
      ],
      component: DaemonScreen,
    },
    {
      id: 'desktop-deeplink',
      title: 'Deep-Links',
      description:
        'Links wie beton://local/s/… öffnen die Session in der laufenden App, an der richtigen Stelle. Links auf unbekannte Server oder zum Koppeln fragen immer erst nach.',
      features: ['DESK-006', 'DESK-003'],
      states: [
        { id: 'open', title: 'Session an Event geöffnet' },
        { id: 'unknown', title: 'Unbekannter Server (ab M4)' },
        { id: 'pair', title: 'Koppeln (ab M4)' },
      ],
      component: DeepLinkScreen,
    },
    {
      id: 'desktop-profiles',
      title: 'Server-Profile',
      description:
        'Neben dem lokalen Profil kannst du optional weitere Server verbinden (ab M4). Tokens liegen im Schlüsselbund, Profile teilst du mit der CLI.',
      features: ['DESK-003'],
      states: [
        { id: 'list', title: 'Profile' },
        { id: 'add', title: 'Profil hinzufügen' },
        { id: 'expired', title: 'Anmeldung abgelaufen' },
      ],
      component: ProfilesScreen,
    },
    {
      id: 'desktop-update',
      title: 'Updates',
      description:
        'Updates gibt es nur, wenn du danach fragst – oder die automatische Suche selbst einschaltest. Installiert wird immer erst nach Bestätigung; laufende Sessions werden nicht unterbrochen.',
      features: ['DESK-007'],
      states: [
        { id: 'settings', title: 'Einstellungen' },
        { id: 'available', title: 'Update verfügbar' },
        { id: 'drain', title: 'Neustart wartet' },
        { id: 'bad-signature', title: 'Signatur ungültig' },
      ],
      component: UpdateScreen,
    },
    {
      id: 'desktop-voice',
      title: 'Spracheingabe',
      description:
        'Einstellungen für Diktate: lokales Whisper-Modell (Download auf Klick oder eigene Datei), Beschleunigung, Sprache und Push-to-Talk. Keine Cloud.',
      features: ['VOI-001', 'VOI-002', 'VOI-003', 'VOI-006', 'VOI-007', 'VOI-008', 'DESK-008'],
      states: [
        { id: 'none', title: 'Kein Modell' },
        { id: 'downloading', title: 'Modell lädt' },
        { id: 'ready', title: 'Bereit' },
        { id: 'checksum', title: 'Prüfsumme falsch' },
        { id: 'cpu-fallback', title: 'GPU nicht verfügbar' },
      ],
      component: VoiceSettings,
    },
    {
      id: 'desktop-ptt',
      title: 'Push-to-Talk',
      description:
        'Kurzbefehl halten, sprechen, loslassen – auch wenn eine andere App vorne ist. Das HUD zeigt Teil-Ergebnisse; der Text landet ungesendet im Composer der zuletzt aktiven Session.',
      features: ['DESK-008', 'VOI-007', 'VOI-004', 'VOI-005', 'VOI-006', 'VOI-001', 'VOI-002', 'VOI-008'],
      states: [
        { id: 'recording', title: 'Aufnahme' },
        { id: 'inserted', title: 'Im Composer' },
        { id: 'queued', title: 'In der Warteschlange' },
        { id: 'no-mic', title: 'Kein Mikrofonzugriff' },
        { id: 'shortcut-taken', title: 'Kurzbefehl belegt' },
        { id: 'no-model', title: 'Kein Modell' },
      ],
      frame: 'none',
      component: PushToTalk,
    },
    {
      id: 'desktop-windows',
      title: 'Windows (Beta)',
      description:
        'Die Desktop-App auf Windows mit WebView2. Solange die Windows-Sandbox Beta ist, sagt die App das deutlich und graut aus, was nicht geht – mit Begründung.',
      features: ['DESK-009'],
      states: [
        { id: 'session', title: 'Session' },
        { id: 'doctor', title: 'Was geht' },
      ],
      frame: 'none',
      component: WindowsScreen,
    },
  ],
}
