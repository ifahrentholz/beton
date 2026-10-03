import { X } from 'lucide-react'
import { HarnessBadge } from '@/app/harness'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import { Btn, Code, FakeInput, FieldLabel, Mark, Note } from '@/screens/harnesses/kit'
import { AgentLibrary } from './library'

function Dialog({ state }: { state: string }) {
  const name = state === 'maestra-fallback' ? 'maestra' : state === 'duetto-missing' ? 'duetto' : 'pr-fixer'
  const paramError = state === 'param-error'
  const blocked = paramError || state === 'duetto-missing'
  return (
    <div className="absolute inset-0 z-30 flex items-start justify-center bg-foreground/20 pt-12">
      <div role="dialog" aria-label={`${name} starten`} className="flex max-h-[calc(100%-4rem)] w-[580px] flex-col rounded-lg border border-border bg-popover shadow-[0_24px_48px_-24px_rgb(0_0_0/0.5)]">
        <div className="flex items-start gap-3 border-b border-border px-5 py-3">
          <div className="flex-1">
            <h3 className="type-wide text-[16px] font-[700]">{name} starten</h3>
            <p className="text-[12px] text-muted-foreground">
              {name === 'pr-fixer' ? '.beton/agents/pr-fixer · v0.3.0 · Projekt shop-frontend' : `builtin:${name} · v1.0.0`}
            </p>
          </div>
          <button aria-label="Schließen" className="rounded-md p-1 text-muted-foreground hover:bg-accent">
            <X className="size-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4 text-[13px]">
          <div>
            <FieldLabel>Aufgabe</FieldLabel>
            <div className="min-h-[64px] rounded-md border border-input bg-card px-2.5 py-2">
              {name === 'pr-fixer'
                ? 'CI auf fix/checkout-timeout ist rot (e2e: checkout.spec.ts). Bitte beheben.'
                : name === 'maestra'
                  ? 'Stelle den Checkout auf das Stripe Payment Element um, inklusive Webhook-Handler und Tests.'
                  : 'Sollten wir für den Rate-Limiter Redis oder ein Token-Bucket im Gateway nehmen?'}
            </div>
          </div>

          {name === 'pr-fixer' && (
            <F id="AGT-010">
              <FieldLabel hint="aus params in agent.yaml">Parameter</FieldLabel>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <div className="mb-1 flex items-baseline gap-2 text-[12px]">
                    <span className="font-mono">branch</span>
                    <span className="text-muted-foreground">Text · Standard main</span>
                  </div>
                  <FakeInput value="fix/checkout-timeout" mono />
                </div>
                <div>
                  <div className="mb-1 flex items-baseline gap-2 text-[12px]">
                    <span className="font-mono">max_attempts</span>
                    <span className="text-muted-foreground">Ganzzahl · 1 bis 10</span>
                  </div>
                  <FakeInput value={paramError ? '20' : '3'} mono invalid={paramError} />
                  {paramError && <p className="mt-1 text-[12px] text-deny">Höchstens 10 – so legt es der Agent fest.</p>}
                </div>
              </div>
            </F>
          )}

          <F id="AGT-004">
            <FieldLabel hint="aus executor, für diesen Start änderbar">Ausführung</FieldLabel>
            <div className="divide-y divide-border rounded-md border border-border">
              <div className="flex items-center gap-3 px-3 py-2">
                <span className="w-24 text-muted-foreground">Harness</span>
                <HarnessBadge id="claude" model={name === 'pr-fixer' ? 'claude-sonnet-5-5' : 'claude-opus-5-5'} />
                <span className="ml-auto text-[12px] text-muted-foreground">Abo über claude-CLI</span>
              </div>
              <div className="flex items-center gap-3 px-3 py-2">
                <span className="w-24 text-muted-foreground">Modus</span>
                <span>{name === 'pr-fixer' ? 'Dateien ohne Rückfrage ändern' : 'Nur planen (schreibt selbst keinen Code)'}</span>
              </div>
              <div className="flex items-center gap-3 px-3 py-2">
                <span className="w-24 text-muted-foreground">Projekt</span>
                <span>shop-frontend</span>
                <span className="font-mono text-[11px] text-muted-foreground">neuer Worktree von main</span>
              </div>
            </div>
          </F>

          <div className="flex items-start gap-3">
            <div className="flex-1">
              <div className="font-medium">Im Hintergrund laufen lassen</div>
              <p className="text-[12px] text-muted-foreground">Läuft weiter, wenn du das Fenster schließt. Freigaben landen in der Inbox.</p>
            </div>
            <Switch aria-label="Im Hintergrund laufen lassen" />
          </div>

          <F id={['AGT-004', 'AGT-011', 'AGT-012']}>
            <FieldLabel>Vorab geprüft</FieldLabel>
            <ul className="space-y-1">
              <li>
                <Mark kind="ok" label="Claude Code angemeldet (Claude Max)" />
              </li>
              {name === 'pr-fixer' && (
                <>
                  <li>
                    <Mark kind="ok" label="Codex für Sub-Agent quick-check angemeldet" />
                  </li>
                  <li>
                    <Mark kind="ok" label={<>MCP-Server github startbar; Token kommt als Platzhalter an</>} />
                  </li>
                </>
              )}
              {name !== 'pr-fixer' && (
                <li>
                  <Mark kind="deny" label="Codex nicht eingerichtet" />
                </li>
              )}
            </ul>
          </F>

          {state === 'maestra-fallback' && (
            <F id="AGT-011">
              <Note tone="warn" title="Reviews laufen ohne zweiten Vendor">
                maestra lässt Ergebnisse normalerweise von Codex prüfen, wenn Claude implementiert. Ohne Codex reviewt Claude selbst; die Zusammenfassung
                kennzeichnet das als <Code>same-vendor review</Code>.
              </Note>
            </F>
          )}
          {state === 'duetto-missing' && (
            <F id="AGT-012">
              <Note
                tone="deny"
                title="duetto braucht zwei Stimmen"
                action={
                  <Btn variant="outline" size="sm">
                    Codex einrichten
                  </Btn>
                }
              >
                Die Debatte läuft mit Claude Code und Codex. Codex ist auf diesem Rechner nicht installiert; mit nur einer Stimme startet duetto nicht.
              </Note>
            </F>
          )}
        </div>
        <div className="flex items-center gap-2 border-t border-border px-5 py-3">
          <Btn variant="primary" disabled={blocked}>
            {name} starten
          </Btn>
          <Btn variant="ghost">Abbrechen</Btn>
          <span className="ml-auto text-[11px] text-muted-foreground">Agent-Stand wird beim Start eingefroren (sha256:9f2c…e1)</span>
        </div>
      </div>
    </div>
  )
}

export function AgentStart({ state }: { state: string }) {
  return <AgentLibrary state="default" overlay={<Dialog state={state} />} />
}
