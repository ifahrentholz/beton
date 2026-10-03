import { KeyRound, Plus, ShieldCheck } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Callout, DialogPanel, FakeInput, Field, LocalNote, NetNote, Overlay, PageHeader, Segmented, Tag } from '../workspace/parts'

type Host = {
  host: string
  kind: 'GitHub' | 'GitHub Enterprise' | 'GitLab' | 'GitLab self-hosted'
  api: string
  account?: string
  how?: 'OAuth' | 'Token'
  note?: string
  error?: boolean
}

function hostsFor(state: string): Host[] {
  return [
    { host: 'github.com', kind: 'GitHub', api: 'api.github.com', account: '@ifahrentholz', how: 'OAuth', note: 'Zugriff: repo, read:org' },
    { host: 'ghe.acme.corp', kind: 'GitHub Enterprise', api: 'https://ghe.acme.corp/api/v3' },
    {
      host: 'gitlab.acme.corp',
      kind: 'GitLab self-hosted',
      api: 'https://gitlab.acme.corp/api/v4',
      account: state === 'tls-error' ? undefined : '@ingo.f',
      how: state === 'tls-error' ? undefined : 'Token',
      note: state === 'tls-error' ? undefined : 'Token läuft am 31.12. ab · CA ~/certs/acme-root.pem',
      error: state === 'tls-error',
    },
  ]
}

function HostRow({ h }: { h: Host }) {
  return (
    <div className="border-b border-border py-3">
      <div className="flex items-center gap-3">
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span className="text-[14px] font-medium">{h.host}</span>
            <Tag tone="muted">{h.kind}</Tag>
          </span>
          <span className="block font-mono text-[11px] text-muted-foreground">{h.api}</span>
        </span>
        {h.account ? (
          <span className="text-right text-[12px]">
            <span className="flex items-center justify-end gap-1.5">
              <span className="size-2 rounded-full bg-ok" aria-hidden /> Verbunden als <span className="font-medium">{h.account}</span>
              <span className="text-muted-foreground">· {h.how}</span>
            </span>
            {h.note && <span className="block text-[11px] text-muted-foreground">{h.note}</span>}
          </span>
        ) : (
          <span className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
            <span className="size-2 rounded-full border border-muted-foreground" aria-hidden /> {h.error ? 'Verbindung fehlgeschlagen' : 'Nicht verbunden'}
          </span>
        )}
        {h.account ? <Btn size="sm" variant="ghost">Trennen</Btn> : <Btn size="sm">{h.error ? 'Erneut versuchen' : 'Verbinden'}</Btn>}
      </div>
      {h.error && (
        <Callout tone="deny" className="mt-2">
          <p className="font-semibold">Das Zertifikat von gitlab.acme.corp ist nicht vertrauenswürdig.</p>
          <p className="mt-0.5">
            Der Server nutzt ein Zertifikat eurer eigenen CA. Hinterlege die CA-Datei, zum Beispiel <code className="font-mono text-[12px]">~/certs/acme-root.pem</code>, dann
            verbindet beton sich ohne die Prüfung abzuschalten.
          </p>
          <Btn size="sm" className="mt-2">
            CA-Datei wählen …
          </Btn>
        </Callout>
      )}
    </div>
  )
}

function AddHostDialog() {
  return (
    <Overlay>
      <F id={['GIT-001', 'GIT-003', 'GIT-004']}>
        <DialogPanel
          title="Host hinzufügen"
          subtitle="Für GitHub Enterprise oder ein eigenes GitLab. beton ordnet Remotes anhand des Hosts zu."
          footer={
            <>
              <Btn variant="primary">Im Browser anmelden</Btn>
              <Btn variant="ghost" className="ml-auto">
                Abbrechen
              </Btn>
            </>
          }
        >
          <Field label="Art" className="pt-0">
            <Segmented
              value="gitlab"
              items={[
                { id: 'github', label: 'GitHub Enterprise' },
                { id: 'gitlab', label: 'GitLab' },
              ]}
            />
          </Field>
          <Field label="Host">
            <FakeInput value="gitlab.acme.corp" mono />
          </Field>
          <Field label="API-Adresse">
            <FakeInput value="https://gitlab.acme.corp/api/v4" mono />
          </Field>
          <Field label="CA-Datei" hint="Nur bei Zertifikaten einer eigenen CA nötig.">
            <FakeInput value="~/certs/acme-root.pem" mono />
          </Field>
          <Field label="Anmeldung" hint="Standard ist die Anmeldung im Browser. Ein Personal Access Token ist der Ausweg, wenn eure Admins keine OAuth-App eingerichtet haben.">
            <Segmented
              value="token"
              items={[
                { id: 'oauth', label: 'Im Browser (OAuth)' },
                { id: 'token', label: 'Personal Access Token' },
              ]}
            />
            <FakeInput value="glpat-••••••••••••••••••••" mono className="mt-2" />
          </Field>
          <LocalNote className="mt-2">Wird im macOS-Schlüsselbund gespeichert, nicht in einer Datei.</LocalNote>
        </DialogPanel>
      </F>
    </Overlay>
  )
}

const audit = [
  ['14:12', 'git push', 'Rate-Limiter für die Login-API', 'github.com', 'Proxy'],
  ['14:11', 'PR #482 Status abgefragt', 'Rate-Limiter für die Login-API', 'api.github.com', 'Panel'],
  ['13:40', 'MR !118 Pipeline abgefragt', 'Bundle-Preise', 'gitlab.acme.corp', 'Panel'],
]

export function ConnectScreen({ state }: { state: string }) {
  const none = state === 'none'
  return (
    <div className="relative h-full">
      <AppLayout nav="settings" sessionList={false}>
        <PageHeader
          title="GitHub & GitLab"
          subtitle="Optional. Mit Verbindung zeigt beton zu jeder Session Pull- und Merge-Requests, Checks und Reviews. git selbst funktioniert auch ohne."
          actions={
            !none && (
              <Btn>
                <Plus className="size-3.5" /> Host hinzufügen
              </Btn>
            )
          }
        />
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
          <div className="max-w-3xl">
            <NetNote>Netzwerkzugriff: beton fragt die Provider direkt von diesem Rechner ab, alle 30 Sekunden, solange du ein PR-Panel ansiehst, sonst alle 5 Minuten.</NetNote>

            {none ? (
              <F id={['GIT-004', 'GIT-002', 'GIT-003']} className="mt-6 border-y border-border py-8 text-center">
                <p className="text-[14px] font-medium">Kein Konto verbunden</p>
                <p className="mx-auto mt-1 max-w-md text-[13px] text-muted-foreground">
                  Deine Sessions committen und pushen weiter wie gewohnt. Für Status, Checks und Reviews im PR-Panel verbindest du dein Konto.
                </p>
                <div className="mt-4 flex justify-center gap-2">
                  <Btn variant="primary">Mit GitHub verbinden</Btn>
                  <Btn>Mit GitLab verbinden</Btn>
                  <Btn variant="ghost">Eigenen Host hinzufügen</Btn>
                </div>
              </F>
            ) : (
              <F id={['GIT-001', 'GIT-002', 'GIT-003', 'GIT-004']} className="mt-4 border-t border-border">
                {hostsFor(state).map((h) => (
                  <HostRow key={h.host} h={h} />
                ))}
              </F>
            )}

            <F id="GIT-004" className="mt-6">
              <h3 className="flex items-center gap-1.5 text-[13px] font-semibold">
                <ShieldCheck className="size-4" /> So nutzen Agents deine Zugänge
              </h3>
              <p className="mt-1 text-[13px] text-muted-foreground">
                Tokens liegen im macOS-Schlüsselbund. In der Sandbox sehen <code className="font-mono text-[12px]">git</code>, <code className="font-mono text-[12px]">gh</code> und{' '}
                <code className="font-mono text-[12px]">glab</code> nur Platzhalter wie <code className="font-mono text-[12px]">bt_cred_gh_7Hq2wQ</code>; der lokale Proxy setzt den
                echten Wert erst beim Request an den Host ein. Wer deine Session mitsieht, nutzt für das PR-Panel sein eigenes Konto, nie deins.
              </p>
              {!none && (
                <table className="mt-3 w-full text-left text-[12px]">
                  <thead className="text-muted-foreground">
                    <tr className="border-b border-border">
                      <th className="py-1.5 font-medium">Zeit</th>
                      <th className="py-1.5 font-medium">Nutzung</th>
                      <th className="py-1.5 font-medium">Session</th>
                      <th className="py-1.5 font-medium">Host</th>
                      <th className="py-1.5 font-medium">Über</th>
                    </tr>
                  </thead>
                  <tbody>
                    {audit.map((a) => (
                      <tr key={a[0] + a[1]} className="border-b border-border">
                        {a.map((c, i) => (
                          <td key={i} className={cn('py-1.5 pr-3', (i === 0 || i === 3) && 'font-mono text-[11px]')}>
                            {c}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </F>

            {!none && (
              <F id="GIT-001" className="mt-6">
                <h3 className="flex items-center gap-1.5 text-[13px] font-semibold">
                  <KeyRound className="size-4" /> Remotes zuordnen
                </h3>
                <p className="mt-1 text-[13px] text-muted-foreground">beton erkennt den Provider am Host des Remotes. Remotes ohne passenden Host zeigen im Panel „kein Provider“.</p>
                <pre className="mt-2 rounded-md border border-border bg-card p-3 font-mono text-[12px] leading-relaxed">{`git@github.com:acme/shop-frontend.git        → GitHub · acme/shop-frontend
git@ghe.acme.corp:team/payments.git            → GitHub Enterprise · team/payments
https://gitlab.acme.corp/shop/backend/api.git  → GitLab · shop/backend/api
git@git.internal:legacy/tools.git              → kein Provider`}</pre>
              </F>
            )}
          </div>
        </div>
      </AppLayout>
      {state === 'add-host' && <AddHostDialog />}
    </div>
  )
}
