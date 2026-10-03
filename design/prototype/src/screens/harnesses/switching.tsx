import type { ReactNode } from 'react'
import { Check, Lock } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { Composer, SessionHeader } from '@/app/session-chrome'
import { AgentMessage, SystemNote, ToolCall, TurnFooter, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import type { HarnessId } from '@/mock/data'
import { Code } from './kit'

function Popover({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn('absolute bottom-[78px] z-20 w-[360px] rounded-md border border-border bg-popover p-1.5 text-[13px] shadow-[0_12px_32px_-12px_rgb(0_0_0/0.35)]', className)}>
      {children}
    </div>
  )
}

function Option({ title, sub, selected, disabled, right }: { title: ReactNode; sub?: ReactNode; selected?: boolean; disabled?: boolean; right?: ReactNode }) {
  return (
    <div className={cn('flex items-start gap-2 rounded-[3px] px-2 py-1.5', selected && 'bg-accent', disabled ? 'text-muted-foreground' : 'hover:bg-accent')}>
      <span className="mt-0.5 w-3.5 shrink-0">{selected && <Check className="size-3.5" />}</span>
      <span className="min-w-0 flex-1">
        <span className={cn(disabled ? '' : 'font-medium')}>{title}</span>
        {sub && <span className="block text-[12px] text-muted-foreground">{sub}</span>}
      </span>
      {right}
    </div>
  )
}

function ModelPicker() {
  return (
    <F id="HAR-017" className="absolute inset-x-0 bottom-0">
      <Popover className="left-[150px]">
        <div className="px-2 pt-1 pb-1.5 text-[11px] text-muted-foreground">Modell · Claude Code</div>
        <Option title={<span className="font-mono text-[12.5px]">claude-opus-5-5</span>} sub="aktuell" selected />
        <Option title={<span className="font-mono text-[12.5px]">claude-sonnet-5-5</span>} sub="schneller, schont das Kontingent" />
        <Option title={<span className="font-mono text-[12.5px]">claude-haiku-4-5</span>} sub="kein Effort „sehr hoch“" />
        <div className="my-1.5 border-t border-border" />
        <div className="px-2 pb-1 text-[11px] text-muted-foreground">Effort</div>
        <div className="flex gap-1 px-2 pb-1.5">
          {['niedrig', 'mittel', 'hoch', 'sehr hoch'].map((e) => (
            <span key={e} className={cn('rounded-[3px] border px-2 py-1 text-xs', e === 'hoch' ? 'border-foreground bg-foreground text-background' : 'border-border')}>
              {e}
            </span>
          ))}
        </div>
        <div className="mx-2 mb-1 rounded-sm bg-muted px-2 py-1.5 text-[12px] text-muted-foreground">
          Der Agent arbeitet gerade. Ein Wechsel gilt ab dem nächsten Turn; der Verlauf bleibt erhalten.
        </div>
      </Popover>
    </F>
  )
}

function ModePicker({ blocked }: { blocked?: boolean }) {
  return (
    <F id="HAR-027" className="absolute inset-x-0 bottom-0">
      <Popover className="left-[380px] w-[400px]">
        <div className="px-2 pt-1 pb-1.5 text-[11px] text-muted-foreground">Wie viel darf der Agent ohne Rückfrage?</div>
        <Option title="Nur planen" sub="Liest und plant, ändert nichts." />
        <Option title="Fragen bei Schreibzugriff" sub="Standard des Harness; Freigaben laut deinen Policies." selected />
        <Option title="Dateien ohne Rückfrage ändern" sub="Edits im Worktree laufen durch, Shell-Befehle fragen weiter." />
        {blocked ? (
          <Option
            disabled
            title="YOLO – keine Rückfragen des Agents"
            sub={
              <>
                Nicht verfügbar: Sandbox für Tools ist auf diesem Rechner aus. YOLO startet nur mit Sandbox und Egress-Proxy.{' '}
                <span className="text-foreground underline">Sandbox einrichten</span>
              </>
            }
            right={<Lock className="mt-0.5 size-3.5" />}
          />
        ) : (
          <Option
            title="YOLO – keine Rückfragen des Agents"
            sub={
              <>
                Deine Policies gelten weiter (z. B. kein <Code>git push --force</Code>). Sandbox <Check className="inline size-3 text-ok" /> aktiv ·
                Proxy <Check className="inline size-3 text-ok" /> aktiv
              </>
            }
          />
        )}
        <div className="mx-2 mt-1 mb-1 border-t border-border pt-1.5 text-[11px] text-muted-foreground">Claude Code übernimmt den Modus sofort.</div>
      </Popover>
    </F>
  )
}

export function SwitchingScreen({ state }: { state: string }) {
  const harness: HarnessId = state === 'restart' ? 'gemini' : state === 'effort-mapped' ? 'codex' : 'claude'
  const title =
    harness === 'gemini' ? 'Terraform-Plan erklären' : harness === 'codex' ? 'Event-Log: seq lückenlos halten' : 'Rate-Limiter für die Login-API'
  const model = state === 'switched' ? 'claude-sonnet-5-5' : state === 'restart' ? 'gemini-3-flash' : undefined
  const running = state === 'model-picker'

  return (
    <AppLayout activeSession={harness === 'gemini' ? 'ses_6m4d' : harness === 'codex' ? 'ses_6p9z' : 'ses_7f3k'}>
      <SessionHeader title={title} harness={harness} model={model} status={running ? 'running' : 'idle'} />
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
            {harness === 'claude' && (
              <>
                <UserMessage>Schreib noch Tests für den Fall, dass Redis nicht erreichbar ist.</UserMessage>
                <ToolCall kind="read" name="Lesen" target="src/middleware/rate-limit.ts" duration="0,1 s" />
                <ToolCall kind="edit" name="Bearbeiten" target="src/middleware/rate-limit.spec.ts" duration="0,4 s" status={running ? 'running' : 'ok'} />
                <AgentMessage harness="claude" streaming={running}>
                  <p>Ich ergänze einen Test, der den Redis-Client ausfallen lässt und prüft, dass die Route offen bleibt (fail open) und warnt.</p>
                </AgentMessage>
                {!running && <TurnFooter duration="48 s" tokens="21.304" cost="Subscription" />}
                {state === 'switched' && (
                  <F id="HAR-017">
                    <SystemNote>
                      Modell gewechselt: claude-opus-5-5 → claude-sonnet-5-5 · sofort wirksam, ab Turn 7 · Verlauf bleibt erhalten
                    </SystemNote>
                  </F>
                )}
                {(state === 'permission-mode' || state === 'yolo-blocked') && (
                  <UserMessage>Mach die restlichen Routen genauso, ohne jedes Mal zu fragen.</UserMessage>
                )}
              </>
            )}
            {harness === 'gemini' && (
              <>
                <UserMessage>Was ändert der Plan an der Datenbank?</UserMessage>
                <AgentMessage harness="gemini">
                  <p>Der Plan ersetzt die Instanzklasse von db.t3.medium auf db.t4g.large. Das erzwingt einen Neustart mit ca. 5 Min. Ausfall.</p>
                </AgentMessage>
                <F id="HAR-017">
                  <SystemNote>Modell gewechselt auf gemini-3-flash · Gemini CLI wurde neu gestartet und hat die Session fortgesetzt</SystemNote>
                </F>
                <div className="ml-9 text-[12px] text-muted-foreground">
                  Gemini CLI kann das Modell nicht während der Session wechseln. beton startet den Agent dafür neu und gibt den bisherigen Verlauf als
                  Übergabe-Dokument mit.
                </div>
              </>
            )}
            {harness === 'codex' && (
              <>
                <UserMessage>Prüf bitte die Invariante für seq bei parallelen Writes sehr gründlich.</UserMessage>
                <F id="HAR-017">
                  <SystemNote>Effort: „sehr hoch“ angefragt, „hoch“ aktiv – gpt-5.3 unterstützt keine höhere Stufe</SystemNote>
                </F>
                <AgentMessage harness="codex">
                  <p>Ich schaue mir zuerst an, wie append() die Sequenz vergibt und ob die Transaktion SERIALIZABLE ist.</p>
                </AgentMessage>
                <ToolCall kind="search" name="Suche" target="rg 'fn append' crates/beton-store" duration="0,2 s" />
              </>
            )}
          </div>
        </div>
        {state === 'model-picker' && <ModelPicker />}
        {state === 'permission-mode' && <ModePicker />}
        {state === 'yolo-blocked' && <ModePicker blocked />}
        <Composer harness={harness} running={running} />
      </div>
    </AppLayout>
  )
}
