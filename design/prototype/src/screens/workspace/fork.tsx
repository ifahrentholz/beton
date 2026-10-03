import type { ReactNode } from 'react'
import { ArrowUpRight, Check, GitFork } from 'lucide-react'
import { HarnessBadge } from '@/app/harness'
import { Composer } from '@/app/session-chrome'
import { AgentMessage, SystemNote, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { harnesses, type HarnessId } from '@/mock/data'
import { ShortStream } from './bits'
import { Btn, Callout, DialogPanel, FakeInput, FakeSelect, Field, Menu, MenuItem, MenuLabel, MenuSeparator, Overlay, SessionShell } from './parts'

function Choice({ selected, title, hint, disabled }: { selected?: boolean; title: ReactNode; hint?: ReactNode; disabled?: boolean }) {
  return (
    <div role="radio" aria-checked={selected} aria-disabled={disabled} className={cn('flex gap-2.5 rounded-md border px-2.5 py-2', selected ? 'border-foreground bg-card' : 'border-border', disabled && 'opacity-55')}>
      <span className={cn('mt-0.5 flex size-3.5 shrink-0 items-center justify-center rounded-full border', selected ? 'border-foreground' : 'border-muted-foreground')}>
        {selected && <span className="size-1.5 rounded-full bg-foreground" />}
      </span>
      <span className="min-w-0">
        <span className="block text-[13px]">{title}</span>
        {hint && <span className="block text-[12px] text-muted-foreground">{hint}</span>}
      </span>
    </div>
  )
}

function ForkDialog({ target, error }: { target: HarnessId; error?: boolean }) {
  return (
    <Overlay>
      <F id={['SES-006', 'SES-007']}>
        <DialogPanel
          width="w-[600px]"
          title="Neue Session ab hier abzweigen"
          subtitle="„Rate-Limiter für die Login-API“ bleibt unverändert. Der Fork übernimmt den Verlauf bis zur gewählten Stelle."
          footer={
            <>
              <Btn variant="primary" disabled={error}>
                <GitFork className="size-3.5" /> Fork anlegen und öffnen
              </Btn>
              <Btn variant="ghost" className="ml-auto">
                Abbrechen
              </Btn>
            </>
          }
        >
          <Field label="Ab" className="pt-0">
            <div className="text-[13px]">Ende von Turn 2 · 14:04 · Ereignis 141</div>
            <p className="mt-0.5 text-[12px] text-muted-foreground">Du hast mitten in Turn 3 geklickt (Ereignis 148). Ein Fork beginnt immer am Ende eines vollständigen Turns.</p>
          </Field>
          <Field label="Weiter mit">
            <div className="flex flex-col gap-1.5">
              <Choice selected={target === 'claude'} title={<HarnessBadge id="claude" />} hint="Gleicher Harness: Claude Code übernimmt den Verlauf nativ." />
              <Choice selected={target === 'codex'} title={<HarnessBadge id="codex" />} hint={`Bekommt eine Übergabe-Zusammenfassung (ca. 8.000 Tokens). ${harnesses.codex.auth}.`} />
              <Choice selected={target === 'gemini'} title={<HarnessBadge id="gemini" />} hint={`Übergabe-Zusammenfassung. ${harnesses.gemini.auth}.`} />
            </div>
          </Field>
          {error && (
            <Callout tone="deny" className="mb-2">
              <p className="font-semibold">Gemini CLI kann den Agent „implementer“ nicht ausführen.</p>
              <p className="mt-0.5">Er braucht die Fähigkeit „Sub-Agents starten“, die Gemini CLI nicht anbietet. Wähle Claude Code oder Codex, oder forke ohne Agent.</p>
            </Callout>
          )}
          <Field label="Modell">
            <div className="flex gap-2">
              <FakeSelect value={<span className="font-mono text-[12px]">{harnesses[target].models[0]}</span>} className="w-56" />
              <FakeSelect value="Effort: hoch" className="w-32" />
            </div>
          </Field>
          <Field label="Dateien" hint="Dateien werden nicht auf den Stand von Turn 2 zurückgesetzt.">
            <div className="flex flex-col gap-1.5">
              <Choice selected title="Neuer Worktree" hint={<span className="font-mono text-[11px]">beton/rate-limiter-fork-2m9q von HEAD, inkl. WIP-Snapshot</span>} />
              <Choice title="Gemeinsamer Workspace" hint="Arbeitet in denselben Dateien wie die Quelle. Beide Sessions können sich überschreiben." />
              <Choice title="Leer" hint="Ohne Dateien, nur mit dem Verlauf." />
            </div>
          </Field>
          <Field label="Titel" className="pb-0">
            <FakeInput value={target === 'claude' ? 'Rate-Limiter (Fork)' : `Rate-Limiter (${harnesses[target].name})`} />
          </Field>
        </DialogPanel>
      </F>
    </Overlay>
  )
}

function ContinueMenu() {
  return (
    <Menu className="w-[440px]">
      <MenuLabel>Harness dieser Session</MenuLabel>
      <MenuItem icon={<Check className="size-3.5" />} hint={harnesses.claude.auth}>
        <HarnessBadge id="claude" />
      </MenuItem>
      <MenuSeparator />
      <MenuLabel>Weiter mit … (neue Session ab hier, diese bleibt bei Claude Code)</MenuLabel>
      <MenuItem active icon={<GitFork className="size-3.5" />} hint={harnesses.codex.auth} right="Übergabe">
        Weiter mit Codex
      </MenuItem>
      <MenuItem icon={<GitFork className="size-3.5" />} hint={harnesses.gemini.auth} right="Übergabe">
        Weiter mit Gemini CLI
      </MenuItem>
      <MenuItem icon={<GitFork className="size-3.5" />} hint={harnesses.ollama.auth} right="Übergabe">
        Weiter mit Ollama (lokal)
      </MenuItem>
    </Menu>
  )
}

function MessageForkAction() {
  return (
    <div className="ml-9 flex items-center gap-1 text-[12px] text-muted-foreground">
      <span className="rounded-md border border-border bg-popover px-1.5 py-0.5 text-foreground shadow-sm">
        <GitFork className="mr-1 inline size-3" />
        Ab hier forken
      </span>
      <span>Kopieren</span>
      <span>·</span>
      <span>Kommentieren</span>
    </div>
  )
}

export function ForkScreen({ state }: { state: string }) {
  if (state === 'forked') {
    return (
      <SessionShell title="Rate-Limiter (Codex)" harness="codex" status="running" branch="beton/rate-limiter-fork-2m9q" activeSession="ses_7f3m">
        <F id="SES-007" className="flex items-center gap-2 border-l-2 border-voice-codex bg-card px-3 py-2 text-[13px]">
          <GitFork className="size-4 text-muted-foreground" />
          <span>
            Fortgesetzt aus <span className="font-medium">„Rate-Limiter für die Login-API“</span> auf Codex, ab Turn 3
          </span>
          <a className="ml-auto inline-flex items-center gap-0.5 text-[12px] underline underline-offset-2">
            Original öffnen <ArrowUpRight className="size-3" />
          </a>
        </F>
        <SystemNote>Übergabe an Codex: Zusammenfassung von 3 Turns, 7.940 Tokens · Zugangsdaten als bt_cred_* ersetzt</SystemNote>
        <UserMessage>Bitte schau dir den Rate-Limiter kritisch an: Was passiert hinter einem Proxy, und ist der Speicher begrenzt?</UserMessage>
        <ToolCall kind="read" name="Lesen" target="src/middleware/rate-limit.ts" duration="0,1 s" />
        <AgentMessage harness="codex" streaming>
          <p>
            Zwei Punkte: <code className="rounded-sm bg-muted px-1 text-[13px]">buckets</code> wächst ohne Obergrenze, und hinter einem Proxy ist{' '}
            <code className="rounded-sm bg-muted px-1 text-[13px]">req.ip</code> ohne
          </p>
        </AgentMessage>
      </SessionShell>
    )
  }

  return (
    <SessionShell
      overlay={state === 'dialog' ? <ForkDialog target="codex" /> : state === 'incompatible' ? <ForkDialog target="gemini" error /> : undefined}
      composer={
        <div className="relative">
          {state === 'continue' && (
            <F id="SES-007" className="absolute bottom-[calc(100%-46px)] left-14 z-30">
              <ContinueMenu />
            </F>
          )}
          <Composer harness="claude" />
        </div>
      }
    >
      <ShortStream />
      <UserMessage>Ja, push und PR.</UserMessage>
      {state === 'dialog' || state === 'incompatible' ? <MessageForkAction /> : null}
    </SessionShell>
  )
}
