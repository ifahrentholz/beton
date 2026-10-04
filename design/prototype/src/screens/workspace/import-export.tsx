import { Download, FileUp } from 'lucide-react'
import { SystemNote, UserMessage, AgentMessage, ToolCall } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Callout, DialogPanel, FakeInput, Field, LocalNote, Overlay, Segmented, SessionShell, ShortStream, Tag } from '@/app/kit/workspace'

const candidates = [
  { title: 'Checkout: Stripe-Webhook verifizieren', cwd: '~/code/shop-frontend', project: 'shop-frontend', msgs: 42, date: '28.09.', checked: true },
  { title: 'fix flaky payment test', cwd: '~/code/shop-frontend', project: 'shop-frontend', msgs: 18, date: '26.09.', checked: true },
  { title: 'terraform state mv erklären', cwd: '~/code/infra', project: 'infra', msgs: 9, date: '24.09.', checked: true },
  { title: 'Rate-Limiter erste Skizze', cwd: '~/code/shop-frontend', project: 'shop-frontend', msgs: 11, date: '23.09.', done: true },
  { title: 'kurze Frage zu awk', cwd: '~/tmp', msgs: 3, date: '20.09.' },
]

function Box({ checked, disabled }: { checked?: boolean; disabled?: boolean }) {
  return (
    <span
      role="checkbox"
      aria-checked={checked}
      aria-disabled={disabled}
      className={cn('flex size-3.5 shrink-0 items-center justify-center rounded-[3px] border text-[10px] leading-none', checked ? 'border-foreground bg-foreground text-background' : 'border-muted-foreground', disabled && 'opacity-40')}
    >
      {checked ? '✓' : ''}
    </span>
  )
}

function ImportDialog() {
  return (
    <Overlay>
      <F id="SES-008">
        <DialogPanel
          width="w-[660px]"
          title="Chats aus Claude Code und Codex übernehmen"
          subtitle="Bestehende Chats werden zu beton-Sessions und lassen sich danach fortsetzen."
          footer={
            <>
              <Btn variant="primary">3 Chats übernehmen</Btn>
              <Btn variant="ghost">Alle der letzten 30 Tage auswählen</Btn>
              <LocalNote className="ml-auto">Liest nur lokale Dateien</LocalNote>
            </>
          }
        >
          <div className="mb-2 flex items-center gap-3">
            <Segmented
              value="claude"
              items={[
                { id: 'claude', label: 'Claude Code · 14' },
                { id: 'codex', label: 'Codex · 6' },
              ]}
            />
            <span className="font-mono text-[11px] text-muted-foreground">~/.claude/projects</span>
          </div>
          <div className="border-t border-border">
            {candidates.map((c) => (
              <div key={c.title} className={cn('flex items-center gap-3 border-b border-border py-2 text-[13px]', c.done && 'text-muted-foreground')}>
                <Box checked={c.checked} disabled={c.done} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate">{c.title}</span>
                  <span className="block font-mono text-[11px] text-muted-foreground">{c.cwd}</span>
                </span>
                {c.done ? (
                  <span className="flex items-center gap-2 text-[12px]">
                    Bereits übernommen <Btn size="sm" variant="ghost">Öffnen</Btn>
                  </span>
                ) : (
                  <>
                    {c.project ? <Tag tone="muted">→ {c.project}</Tag> : <Tag tone="muted">ohne Project</Tag>}
                    <span className="w-24 text-right text-[12px] text-muted-foreground tabular-nums">
                      {c.msgs} Nachr. · {c.date}
                    </span>
                  </>
                )}
              </div>
            ))}
          </div>
          <p className="mt-2 text-[12px] text-muted-foreground">Das Project ergibt sich aus dem Ordner, in dem der Chat lief.</p>
        </DialogPanel>
      </F>
    </Overlay>
  )
}

function ExportDialog() {
  return (
    <Overlay>
      <F id="SES-009">
        <DialogPanel
          title="Session exportieren"
          subtitle="Für Backups, zum Weitergeben oder für Fehlerberichte. Eine andere beton-Installation kann die Datei importieren."
          footer={
            <>
              <Btn variant="primary">
                <Download className="size-3.5" /> Exportieren
              </Btn>
              <Btn variant="ghost" className="ml-auto">
                Abbrechen
              </Btn>
            </>
          }
        >
          <Field label="Inhalt" className="pt-0">
            <Segmented
              value="jsonl"
              items={[
                { id: 'jsonl', label: 'Nur Verlauf (.jsonl)' },
                { id: 'tar', label: 'Mit Anhängen (.tar.zst)' },
              ]}
            />
            <label className="mt-2 flex items-start gap-2 text-[13px]">
              <Box />
              <span>
                Rohausgaben des Harness einschließen
                <span className="block text-[12px] text-muted-foreground">Redigiert. Hilfreich für Fehlerberichte an beton.</span>
              </span>
            </label>
          </Field>
          <Field label="Speichern unter">
            <FakeInput value="~/Downloads/ses_7f3k-rate-limiter.jsonl" mono />
            <p className="mt-1 text-[12px] text-muted-foreground">1.938 Ereignisse · ca. 1,2 MB</p>
          </Field>
          <Callout className="mt-2">Geheimnisse sind nie enthalten. Zugangsdaten stehen als bt_cred_*-Platzhalter in der Datei.</Callout>
        </DialogPanel>
      </F>
    </Overlay>
  )
}

function ImportFileError() {
  return (
    <Overlay>
      <F id="SES-009">
        <DialogPanel
          title="Session-Datei importieren"
          footer={
            <>
              <Btn variant="primary">
                <FileUp className="size-3.5" /> Andere Datei wählen
              </Btn>
              <Btn variant="ghost" className="ml-auto">
                Schließen
              </Btn>
            </>
          }
        >
          <div className="flex items-center gap-2 rounded-md border border-dashed border-border px-3 py-3 text-[13px]">
            <FileUp className="size-4 text-muted-foreground" />
            <span className="font-mono text-[12px]">ses_9a1b-bugreport.jsonl</span>
            <span className="ml-auto text-[12px] text-muted-foreground">840 KB</span>
          </div>
          <Callout tone="deny" className="mt-3">
            <p className="font-semibold">Diese Datei kann diese Version nicht lesen.</p>
            <p className="mt-0.5">
              Sie hat Exportformat 3 (aus beton 1.4). Du hast beton 1.2, das bis Format 2 liest. Aktualisiere beton und importiere die Datei dann erneut. Es wurde nichts
              angelegt.
            </p>
          </Callout>
        </DialogPanel>
      </F>
    </Overlay>
  )
}

export function ImportExportScreen({ state }: { state: string }) {
  if (state === 'imported') {
    return (
      <SessionShell title="Checkout: Stripe-Webhook verifizieren" status="stopped" branch="main" activeSession="imported">
        <F id="SES-008" className="flex flex-col gap-4">
          <Callout tone="ok">3 Chats übernommen, 1 übersprungen, weil schon übernommen. Sie stehen in der Liste unter ihrem Project.</Callout>
          <SystemNote>Übernommen aus Claude Code am 03.10. · Originalzeiten erhalten · lässt sich nativ fortsetzen</SystemNote>
        </F>
        <UserMessage>Prüf bitte, ob wir die Stripe-Signatur im Webhook korrekt verifizieren.</UserMessage>
        <ToolCall kind="search" name="Suche" target="rg 'stripe-signature' src/" duration="0,2 s" sandbox={false} />
        <AgentMessage harness="claude">
          <p>
            Der Webhook liest den Body schon als JSON, bevor die Signatur geprüft wird. Für <code className="rounded-sm bg-muted px-1 text-[13px]">constructEvent</code> braucht es den
            Roh-Body.
          </p>
        </AgentMessage>
      </SessionShell>
    )
  }
  return (
    <SessionShell
      overlay={state === 'import' ? <ImportDialog /> : state === 'export' ? <ExportDialog /> : state === 'import-error' ? <ImportFileError /> : undefined}
    >
      <ShortStream />
    </SessionShell>
  )
}
