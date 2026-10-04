import type { ReactNode } from 'react'
import { Check, Cpu, Download, FolderOpen, HardDrive, Lock, Mic, MicOff, TriangleAlert, X } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { Composer, SessionHeader } from '@/app/session-chrome'
import { AgentMessage, UserMessage } from '@/app/stream'
import { SettingsFrame } from '@/app/settings-shell'
import { Progress } from '@/components/ui/progress'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { DesktopScene, EditorWindow, Segmented, SceneWindow, SettingRow, SettingsHeader, SettingsSection } from './scene'

/*
 * Spracheingabe (VOI) – nur lokal mit Whisper. Keine Cloud, keine Browser-Spracherkennung.
 * Modelle kommen nur auf Klick oder als lokale Datei.
 */

/** Garantie-Hinweis, überall gleich formuliert. */
export function LocalOnlyNote({ className }: { className?: string }) {
  return (
    <F id="VOI-008" className={cn('flex items-start gap-2 rounded-md border border-border bg-card p-3 text-[12.5px]', className)}>
      <Lock className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
      <div>
        <p className="font-medium">Deine Stimme bleibt auf diesem Rechner.</p>
        <p className="text-muted-foreground">
          beton transkribiert lokal mit Whisper. Es gibt keine Cloud-Transkription und keine Browser-Spracherkennung; Audio wird weder
          gespeichert noch ins Session-Log geschrieben. Netzwerk nutzt beton hier nur, wenn du ein Modell herunterlädst.
        </p>
      </div>
    </F>
  )
}

type Model = { name: string; size: string; note: string; installed?: boolean; active?: boolean; recommended?: boolean }

export function VoiceSettings({ state }: { state: string }) {
  const ready = state === 'ready' || state === 'cpu-fallback'
  const models: Model[] = [
    { name: 'small', size: '466 MB', note: 'schnell, für ältere Rechner', installed: ready, active: false },
    { name: 'medium', size: '1,5 GB', note: 'genauer, langsamer' },
    { name: 'large-v3-turbo', size: '1,6 GB', note: 'beste Erkennung', installed: ready, active: ready, recommended: true },
    { name: 'large-v3-turbo (q5_0)', size: '574 MB', note: 'kompakt, fast gleich gut' },
  ]
  return (
    <SettingsFrame active="voice">
      <SettingsHeader page="Spracheingabe" />
      <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-8">
        <div className="max-w-3xl">
          <LocalOnlyNote className="my-4" />

          <F id={['VOI-002', 'VOI-001']} as="section" className="border-t border-border py-5">
            <h3 className="type-wide text-[14px] font-[650]">Modell</h3>
            <p className="mt-0.5 text-[12.5px] text-muted-foreground">
              Whisper-Modelle lädt beton erst, wenn du es willst – oder du wählst eine Modelldatei, die du schon hast. Jede Datei wird gegen die
              im Programm hinterlegte SHA-256-Prüfsumme geprüft.
            </p>

            {state === 'none' && (
              <div className="chamfer mt-3 border-l-4 border-signal bg-signal-soft p-3 text-[12.5px]">
                <p className="font-semibold">Noch kein Modell installiert</p>
                <p className="mt-0.5">Für diesen Mac (Apple M3 Pro) empfehlen wir large-v3-turbo.</p>
                <div className="mt-2.5 flex flex-wrap items-center gap-2">
                  <button className="chamfer-sm inline-flex items-center gap-1 bg-foreground px-3 py-1.5 font-semibold text-background">
                    <Download className="size-3.5" /> large-v3-turbo laden (1,6 GB)
                  </button>
                  <button className="inline-flex items-center gap-1 rounded-md border border-foreground/30 bg-card px-3 py-1.5">
                    <FolderOpen className="size-3.5" /> Modelldatei wählen…
                  </button>
                </div>
                <p className="mt-2 text-[11.5px] text-muted-foreground">
                  Download von huggingface.co (feste Adresse im Programm) nach ~/.beton/models/whisper/. Abgebrochene Downloads werden fortgesetzt.
                </p>
              </div>
            )}
            {state === 'downloading' && (
              <div className="mt-3 max-w-lg text-[12.5px]">
                <div className="flex items-baseline justify-between">
                  <span className="font-medium">large-v3-turbo wird geladen</span>
                  <span className="text-muted-foreground tabular-nums">1,04 von 1,6 GB · noch ≈ 40 s</span>
                </div>
                <Progress value={65} className="mt-2 h-1.5" />
                <div className="mt-2 flex items-center gap-3 text-[11.5px] text-muted-foreground">
                  <span>von huggingface.co · nach dem Download Prüfung per SHA-256 · fortgesetzt ab 410 MB</span>
                  <button className="ml-auto rounded-md border border-border px-2 py-0.5 text-foreground hover:bg-accent">Pausieren</button>
                </div>
              </div>
            )}
            {state === 'checksum' && (
              <div className="mt-3 rounded-md border border-deny/40 bg-deny-soft p-3 text-[12.5px]">
                <p className="font-semibold">Modell verworfen: Prüfsumme stimmt nicht</p>
                <p className="mt-0.5">
                  Die Datei ggml-large-v3-turbo.bin passt nicht zur hinterlegten Prüfsumme. beton hat sie gelöscht; im Modellordner liegt nichts
                  Halbfertiges. Lade sie erneut oder wähle eine geprüfte Datei.
                </p>
                <div className="mt-2 flex gap-2">
                  <button className="rounded-md bg-foreground px-3 py-1.5 font-semibold text-background">Erneut laden (1,6 GB)</button>
                  <button className="rounded-md border border-border bg-card px-3 py-1.5">Modelldatei wählen…</button>
                </div>
              </div>
            )}

            <table className="mt-4 w-full text-[12.5px]">
              <thead>
                <tr className="border-b border-border text-left text-[11.5px] text-muted-foreground">
                  <th className="py-1 font-normal">Modell</th>
                  <th className="py-1 font-normal">Größe</th>
                  <th className="py-1 font-normal" />
                  <th className="py-1 text-right font-normal">Status</th>
                </tr>
              </thead>
              <tbody>
                {models.map((m) => (
                  <tr key={m.name} className="border-b border-border/60">
                    <td className="py-1.5">
                      <span className="font-mono text-[12px]">{m.name}</span>
                      {m.recommended && <span className="ml-2 rounded-sm border border-border px-1 text-[10.5px] text-muted-foreground">empfohlen</span>}
                    </td>
                    <td className="py-1.5 tabular-nums">{m.size}</td>
                    <td className="py-1.5 text-muted-foreground">{m.note}</td>
                    <td className="py-1.5 text-right">
                      {m.active ? (
                        <span className="inline-flex items-center gap-1 text-ok">
                          <Check className="size-3.5" /> aktiv
                        </span>
                      ) : m.installed ? (
                        <span className="inline-flex items-center gap-2 text-muted-foreground">
                          installiert <button className="text-foreground underline-offset-2 hover:underline">Entfernen</button>
                        </span>
                      ) : state === 'downloading' && m.recommended ? (
                        <span className="text-muted-foreground">wird geladen</span>
                      ) : (
                        <button className="inline-flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-[12px] hover:bg-accent">
                          <Download className="size-3" /> Laden
                        </button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="mt-3 grid grid-cols-2 gap-x-8 text-[12.5px]">
              <SettingRow label="Modell entladen nach" hint="voice.idle_unload – gibt den Arbeitsspeicher frei">
                <span className="rounded-md border border-input bg-card px-2 py-1 font-mono text-[12px]">10 Min.</span>
              </SettingRow>
              <SettingRow label="Gleichzeitige Diktate" hint="weitere warten in der Reihe">
                <span className="rounded-md border border-input bg-card px-2 py-1 font-mono text-[12px]">1</span>
              </SettingRow>
            </div>
          </F>

          <F id="VOI-003" as="div">
            <SettingsSection title="Beschleunigung" hint="beton wählt das schnellste verfügbare Verfahren und weicht bei Problemen auf die CPU aus.">
              {state === 'cpu-fallback' ? (
                <div className="flex items-start gap-2 rounded-md border border-border bg-card p-3 text-[12.5px]">
                  <TriangleAlert className="mt-0.5 size-4 shrink-0 text-deny" />
                  <div>
                    <p className="font-medium">Metal ließ sich nicht starten – Diktate laufen auf der CPU</p>
                    <p className="text-muted-foreground">
                      Das ist langsamer (Echtzeitfaktor 0,9 statt 0,2), funktioniert aber. Details stehen im Log und in{' '}
                      <code className="font-mono">beton doctor</code>.
                    </p>
                  </div>
                </div>
              ) : (
                <div className="flex items-center gap-6 text-[12.5px]">
                  <span className="inline-flex items-center gap-1.5">
                    <Cpu className="size-4 text-muted-foreground" />
                    <span className="font-medium">Metal</span>
                    <span className="text-muted-foreground">Apple M3 Pro</span>
                  </span>
                  <span className="text-muted-foreground">{ready ? 'Echtzeitfaktor 0,21 (10 s Sprache in 2,1 s)' : 'wird nach dem ersten Diktat gemessen'}</span>
                  <span className="ml-auto">
                    <Segmented options={['Automatisch', 'Nur CPU']} value="Automatisch" />
                  </span>
                </div>
              )}
            </SettingsSection>
          </F>

          <F id="VOI-006" as="div">
            <SettingsSection title="Sprache" hint="Automatisch erkennt beton Deutsch oder Englisch – andere Sprachen werden nicht gewählt.">
              <Segmented options={['Automatisch (DE/EN)', 'Deutsch', 'Englisch']} value="Automatisch (DE/EN)" />
            </SettingsSection>
          </F>

          <F id={['VOI-007', 'DESK-008']} as="div">
            <SettingsSection title="Push-to-Talk" hint="Der Kurzbefehl funktioniert überall auf dem Rechner, auch wenn beton im Hintergrund ist.">
              <SettingRow label="Kurzbefehl">
                <span className="inline-flex items-center gap-1 rounded-md border border-input bg-card px-2 py-1 font-mono text-[12px]">⌘ ⇧ Leertaste</span>
                <button className="rounded-md border border-border px-2 py-1 text-[12px] hover:bg-accent">Ändern</button>
              </SettingRow>
              <SettingRow label="Verhalten" hint="Halten: Aufnahme, solange gedrückt. Umschalten: einmal drücken startet, nochmal stoppt.">
                <Segmented options={['Halten', 'Umschalten']} value="Halten" />
              </SettingRow>
              <SettingRow label="Transkript sofort senden" hint="Aus: Der Text landet im Composer der zuletzt aktiven Session, du sendest selbst.">
                <Switch aria-label="Sofort senden" />
              </SettingRow>
              <SettingRow label="Mikrofon" hint="MacBook Pro-Mikrofon · Berechtigung erteilt">
                <span className="inline-flex items-center gap-1 text-[12px] text-ok">
                  <Check className="size-3.5" /> erlaubt
                </span>
              </SettingRow>
            </SettingsSection>
          </F>
        </div>
      </div>
    </SettingsFrame>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Push-to-Talk-HUD                                                                            */
/* ------------------------------------------------------------------------------------------ */

function Level() {
  const bars = [4, 9, 14, 8, 16, 11, 6, 12, 15, 7, 10, 5]
  return (
    <span className="flex h-5 items-center gap-[2px]" aria-hidden>
      {bars.map((h, i) => (
        <span key={i} className="w-[3px] animate-pulse rounded-full bg-foreground/70" style={{ height: h, animationDelay: `${i * 90}ms` }} />
      ))}
    </span>
  )
}

function Hud({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <F
      id={['DESK-008', 'VOI-007']}
      className={cn('absolute bottom-24 left-1/2 z-40 w-[520px] -translate-x-1/2 rounded-2xl border border-border bg-popover/95 p-3.5 shadow-xl backdrop-blur', className)}
      badge="top-right"
    >
      {children}
    </F>
  )
}

function HudFooter({ lang = 'Deutsch (erkannt)' }: { lang?: string }) {
  return (
    <div className="mt-2.5 flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-border pt-2 text-[11px] text-muted-foreground">
      <span>
        → <span className="text-foreground">Rate-Limiter für die Login-API</span>
      </span>
      <F id="VOI-006" as="span" badge="bottom-left">
        <span>{lang}</span>
      </F>
      <F id={['VOI-004', 'VOI-008']} as="span" badge="bottom-right" className="ml-auto">
        <span className="inline-flex items-center gap-1">
          <Lock className="size-3" /> lokal · Whisper large-v3-turbo · Audio wird nicht gespeichert
        </span>
      </F>
    </div>
  )
}

export function PushToTalk({ state }: { state: string }) {
  if (state === 'inserted') {
    return (
      <DesktopScene trayCount={3}>
        <SceneWindow title="Rate-Limiter für die Login-API — beton" style={{ top: 48, left: 60, right: 60, bottom: 90 }}>
          <AppLayout activeSession="ses_7f3k">
            <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="idle" branch="beton/rate-limiter-7f3k" />
            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto flex max-w-2xl flex-col gap-4 px-6 py-6">
                <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP.</UserMessage>
                <AgentMessage harness="claude">
                  <p>Fertig – die Login-Route antwortet nach 5 Versuchen mit 429. Alle 10 Tests laufen.</p>
                </AgentMessage>
              </div>
            </div>
            <F id={['VOI-007', 'VOI-005']} className="px-3 pt-1 text-[11.5px] text-muted-foreground">
              Diktat eingefügt (Deutsch, 7 s) – nicht gesendet. Prüfen und mit ⏎ senden.
            </F>
            <Composer harness="claude" draft="Bitte den Rate-Limiter auch für die Register-Route einbauen, gleiche Grenzen, und die Tests ergänzen." />
          </AppLayout>
        </SceneWindow>
      </DesktopScene>
    )
  }

  return (
    <DesktopScene app="Editor" menus={['Ablage', 'Bearbeiten', 'Auswahl', 'Ansicht', 'Fenster', 'Hilfe']} trayRecording={state === 'recording'}>
      <EditorWindow style={{ top: 52, left: 70, right: 70, bottom: 100 }} />

      {state === 'recording' && (
        <Hud>
          <div className="flex items-center gap-3">
            <span className="flex size-8 items-center justify-center rounded-full bg-deny-soft">
              <Mic className="size-4 text-deny" />
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 text-[12px]">
                <span className="font-semibold whitespace-nowrap">Aufnahme läuft</span>
                <span className="truncate text-muted-foreground">Loslassen beendet · Esc bricht ab</span>
              </div>
              <F id="VOI-005" className="mt-1 text-[14px] leading-snug" badge="bottom-right">
                <span>Bitte den Rate-Limiter auch für die Register-Route einbauen, </span>
                <span className="text-muted-foreground">gleiche Grenzen und die Tests</span>
              </F>
            </div>
            <div className="flex flex-col items-end gap-1">
              <Level />
              <span className="text-[11px] text-muted-foreground tabular-nums">0:07 / 2:00</span>
            </div>
          </div>
          <HudFooter />
        </Hud>
      )}

      {state === 'queued' && (
        <Hud>
          <F id="VOI-001" className="flex items-center gap-3">
            <span className="flex size-8 items-center justify-center rounded-full bg-muted">
              <Mic className="size-4 text-muted-foreground" />
            </span>
            <div className="text-[12.5px]">
              <p className="font-semibold">Wartet auf die Transkription – Platz 1 in der Reihe</p>
              <p className="text-muted-foreground">Ein anderes Diktat wird gerade verarbeitet. Deine Aufnahme ist sicher im Arbeitsspeicher und kommt gleich dran.</p>
            </div>
          </F>
          <HudFooter lang="Sprache wird erkannt" />
        </Hud>
      )}

      {state === 'no-mic' && (
        <Hud className="chamfer rounded-none border-l-4 border-l-signal">
          <div className="flex items-start gap-3">
            <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-muted">
              <MicOff className="size-4" />
            </span>
            <div className="text-[12.5px]">
              <p className="font-semibold">beton darf das Mikrofon noch nicht benutzen</p>
              <p className="mt-0.5 text-muted-foreground">
                Erlaube den Zugriff unter Systemeinstellungen › Datenschutz & Sicherheit › Mikrofon. Es wurde nichts aufgenommen oder gesendet.
              </p>
              <div className="mt-2 flex gap-2">
                <button className="chamfer-sm bg-foreground px-3 py-1 text-[12px] font-semibold text-background">Systemeinstellungen öffnen</button>
                <button className="rounded-md border border-border px-2.5 py-1 text-[12px]">Später</button>
              </div>
            </div>
          </div>
        </Hud>
      )}

      {state === 'shortcut-taken' && (
        <Hud className="chamfer rounded-none border-l-4 border-l-signal">
          <div className="text-[12.5px]">
            <p className="font-semibold">⌘⇧Leertaste ist schon vergeben</p>
            <p className="mt-0.5 text-muted-foreground">
              Eine andere App nutzt diesen Kurzbefehl. Wähle einen anderen für Push-to-Talk; bis dahin diktierst du über das Mikrofon im Composer.
            </p>
            <div className="mt-2 flex gap-2">
              <button className="chamfer-sm bg-foreground px-3 py-1 text-[12px] font-semibold text-background">Kurzbefehl ändern</button>
              <button className="rounded-md border border-border px-2.5 py-1 text-[12px]">Ignorieren</button>
            </div>
          </div>
        </Hud>
      )}

      {state === 'no-model' && (
        <Hud className="chamfer rounded-none border-l-4 border-l-signal">
          <F id="VOI-002" className="flex items-start gap-3">
            <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-muted">
              <HardDrive className="size-4" />
            </span>
            <div className="text-[12.5px]">
              <p className="font-semibold">Für Diktate fehlt noch ein Sprachmodell</p>
              <p className="mt-0.5 text-muted-foreground">
                Whisper läuft lokal und braucht dafür eine Modelldatei. Ohne deine Zustimmung lädt beton nichts herunter.
              </p>
              <div className="mt-2 flex flex-wrap gap-2">
                <button className="chamfer-sm inline-flex items-center gap-1 bg-foreground px-3 py-1 text-[12px] font-semibold text-background">
                  <Download className="size-3.5" /> Modell laden (≈ 1,6 GB)
                </button>
                <button className="inline-flex items-center gap-1 rounded-md border border-border px-2.5 py-1 text-[12px]">
                  <FolderOpen className="size-3.5" /> Modelldatei wählen…
                </button>
                <button aria-label="Schließen" className="ml-auto text-muted-foreground">
                  <X className="size-3.5" />
                </button>
              </div>
            </div>
          </F>
        </Hud>
      )}
    </DesktopScene>
  )
}
