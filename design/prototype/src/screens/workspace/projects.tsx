import { FileCode2, GripVertical, Plus } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { projects } from '@/mock/data'
import { Btn, Callout, DialogPanel, FakeInput, FakeSelect, Field, Overlay, PageHeader, Segmented, Tag } from '@/app/kit/workspace'

function ProjectList() {
  return (
    <div className="flex w-56 shrink-0 flex-col border-r border-border bg-sidebar">
      <div className="flex items-center justify-between px-3 pt-3 pb-2">
        <span className="text-[13px] font-semibold">Projects</span>
        <button aria-label="Neues Project" className="flex size-7 items-center justify-center rounded-md hover:bg-accent">
          <Plus className="size-3.5" />
        </button>
      </div>
      {projects.map((p) => (
        <div key={p.id} className={cn('flex items-center gap-1.5 border-l-2 px-2 py-1.5', p.id === 'shop' ? 'border-foreground bg-accent' : 'border-transparent hover:bg-accent/60')}>
          <GripVertical className="size-3.5 shrink-0 text-muted-foreground" aria-label="Reihenfolge ändern" />
          <span className="min-w-0">
            <span className="block truncate text-[13px]">{p.name}</span>
            <span className="block truncate font-mono text-[10px] text-muted-foreground">{p.path}</span>
          </span>
        </div>
      ))}
      <p className="mt-auto px-3 pb-3 text-[11px] text-muted-foreground">Reihenfolge per Ziehen; gilt nur für dich.</p>
    </div>
  )
}

function Source({ repo }: { repo?: boolean }) {
  return repo ? (
    <Tag tone="muted">
      <FileCode2 className="size-3" /> aus .beton/config.yaml
    </Tag>
  ) : (
    <Tag tone="neutral">in beton überschrieben</Tag>
  )
}

function Defaults() {
  return (
    <F id="SES-013" className="max-w-3xl">
      <p className="mb-2 text-[13px] text-muted-foreground">
        Neue Sessions in diesem Project starten mit diesen Vorgaben. Werte aus der Repo-Datei gelten, bis du sie hier überschreibst.
      </p>
      <div className="divide-y divide-border border-y border-border">
        <Field label="Repo auf diesem Rechner" hint="Sessions, die du mit beton run unterhalb dieses Ordners startest, landen automatisch hier.">
          <div className="flex items-center gap-2">
            <FakeInput value="~/code/shop-frontend" mono className="flex-1" />
            <Tag tone="ok">Git · origin github.com/acme/shop-frontend</Tag>
          </div>
        </Field>
        <Field label="Harness">
          <div className="flex items-center gap-2">
            <FakeSelect value={<HarnessBadge id="claude" />} className="w-64" />
            <Source repo />
          </div>
        </Field>
        <Field label="Modell und Effort">
          <div className="flex items-center gap-2">
            <FakeSelect value={<span className="font-mono text-[12px]">claude-opus-5-5</span>} className="w-64" />
            <FakeSelect value="Effort: hoch" className="w-32" />
            <Source />
          </div>
        </Field>
        <Field label="Permission-Mode">
          <div className="flex items-center gap-2">
            <FakeSelect value="Fragen bei Schreibzugriff" className="w-64" />
            <Source repo />
          </div>
        </Field>
        <Field label="Worktree" hint="„Immer“ gibt jeder Session einen eigenen Branch unter ~/.beton/worktrees.">
          <div className="flex items-center gap-2">
            <Segmented
              value="always"
              items={[
                { id: 'always', label: 'Immer' },
                { id: 'ask', label: 'Fragen' },
                { id: 'never', label: 'Nie' },
              ]}
            />
            <Source repo />
          </div>
        </Field>
        <Field label="Base-Branch">
          <div className="flex items-center gap-2">
            <FakeSelect value={<span className="font-mono text-[12px]">main</span>} className="w-64" />
            <Source repo />
          </div>
        </Field>
      </div>
      <div className="mt-4 flex gap-2">
        <Btn variant="primary">Vorgaben speichern</Btn>
        <Btn variant="ghost">Überschreibungen zurücksetzen</Btn>
      </div>
    </F>
  )
}

const rules = [
  { src: '.beton/policies/git.yaml', line: 4, what: 'git push', effect: 'Fragen' },
  { src: '.beton/policies/git.yaml', line: 9, what: 'git push --force', effect: 'Ablehnen' },
  { src: 'In beton gespeichert', what: 'Schreiben außerhalb des Worktrees', effect: 'Ablehnen' },
  { src: 'In beton gespeichert', what: 'Netzwerk: registry.npmjs.org', effect: 'Erlauben' },
]

function Policies() {
  return (
    <F id="SES-014" className="max-w-3xl">
      <p className="mb-2 text-[13px] text-muted-foreground">
        Die Projekt-Ebene liegt zwischen deinen eigenen Regeln und denen einzelner Agents. Über alle Ebenen gilt: die strengere Regel gewinnt. Änderungen wirken beim
        nächsten Tool-Call laufender Sessions, ohne Neustart.
      </p>
      <table className="w-full text-left text-[13px]">
        <thead className="text-[12px] text-muted-foreground">
          <tr className="border-b border-border">
            <th className="py-1.5 pr-3 font-medium">Regel</th>
            <th className="py-1.5 pr-3 font-medium">Wirkung</th>
            <th className="py-1.5 font-medium">Herkunft</th>
          </tr>
        </thead>
        <tbody>
          {rules.map((r) => (
            <tr key={r.what} className="border-b border-border">
              <td className="py-2 pr-3 font-mono text-[12px]">{r.what}</td>
              <td className="py-2 pr-3">
                <Tag tone={r.effect === 'Ablehnen' ? 'deny' : r.effect === 'Erlauben' ? 'ok' : 'neutral'}>{r.effect}</Tag>
              </td>
              <td className="py-2 text-[12px] text-muted-foreground">
                {r.line ? (
                  <span className="font-mono">
                    {r.src}:{r.line}
                  </span>
                ) : (
                  r.src
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="mt-6">
        <div className="text-[13px] font-semibold">Was gilt für einen Befehl?</div>
        <FakeInput value="git push -u origin beton/rate-limiter-7f3k" mono className="mt-2" />
        <ol className="mt-2 flex flex-col text-[13px]">
          {[
            ['Org', '–', 'keine Regel'],
            ['Du', 'Erlauben', 'Regel git-push-erlauben'],
            ['Project shop-frontend', 'Fragen', '.beton/policies/git.yaml:4'],
            ['Agent implementer', '–', 'keine Regel'],
          ].map(([lvl, eff, why]) => (
            <li key={lvl} className="grid grid-cols-[180px_100px_1fr] items-center border-b border-border py-1.5">
              <span>{lvl}</span>
              <span>{eff === '–' ? <span className="text-muted-foreground">–</span> : <Tag tone={eff === 'Erlauben' ? 'ok' : 'neutral'}>{eff}</Tag>}</span>
              <span className="font-mono text-[12px] text-muted-foreground">{why}</span>
            </li>
          ))}
        </ol>
        <Callout className="mt-3">
          Ergebnis: <span className="font-semibold">Fragen</span>. Die Projekt-Regel ist strenger als deine; Sessions außerhalb von shop-frontend betrifft sie nicht.
        </Callout>
      </div>
    </F>
  )
}

export function ProjectsScreen({ state }: { state: string }) {
  const tab = state === 'policies' ? 'policies' : 'defaults'
  return (
    <div className="relative h-full">
      <AppLayout sessionList={false}>
        <div className="flex min-h-0 flex-1">
          <ProjectList />
          <div className="flex min-w-0 flex-1 flex-col">
            <PageHeader
              title="shop-frontend"
              subtitle="6 Sessions · gebunden an ~/code/shop-frontend · liest .beton/config.yaml mit"
              actions={
                <Btn variant="ghost" className="text-deny">
                  Project löschen
                </Btn>
              }
            />
            <div role="tablist" className="flex gap-4 border-b border-border px-6 text-[13px]">
              {[
                ['defaults', 'Vorgaben'],
                ['policies', 'Policies'],
                ['sessions', 'Sessions (6)'],
              ].map(([id, label]) => (
                <span key={id} role="tab" aria-selected={id === tab} className={cn('-mb-px border-b-2 py-2', id === tab ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground')}>
                  {label}
                </span>
              ))}
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">{tab === 'policies' ? <Policies /> : <Defaults />}</div>
          </div>
        </div>
      </AppLayout>
      {state === 'delete' && (
        <Overlay>
          <F id="SES-013">
            <DialogPanel
              title="Project shop-frontend löschen?"
              footer={
                <>
                  <Btn variant="danger">Project löschen, Sessions archivieren</Btn>
                  <Btn variant="ghost" className="ml-auto">
                    Abbrechen
                  </Btn>
                </>
              }
            >
              <ul className="flex list-disc flex-col gap-1 pl-5 text-[13px]">
                <li>Die 6 Sessions werden archiviert, nicht gelöscht. Du findest sie unter „Archiviert“.</li>
                <li>Laufende Sessions (1) werden dabei gestoppt; ihre Worktrees bleiben.</li>
                <li>Das Repo und seine .beton/config.yaml bleiben unverändert.</li>
                <li>Gespeicherte Vorgaben und Projekt-Policies dieses Projects gehen verloren.</li>
              </ul>
            </DialogPanel>
          </F>
        </Overlay>
      )}
    </div>
  )
}
