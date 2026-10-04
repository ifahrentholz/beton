import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import type { HarnessInfo } from '@beton/sdk'
import { X } from 'lucide-react'
import { client } from '@/lib/client'
import { harnessVisible, useFeatures } from '@/lib/features'
import { harnessName } from './harness'

const LAST_CWD = 'beton.lastCwd'

/** Dialog „Neue Session“: Harness, Modell (aus den Capabilities) und Arbeitsverzeichnis. */
export function NewSessionDialog({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate()
  const catalog = useQuery({
    queryKey: ['harnesses'],
    queryFn: () => client.request<{ items: HarnessInfo[] }>('GET', '/v1/harnesses'),
  })
  const features = useFeatures()
  const usable = (catalog.data?.items ?? []).filter(
    (h) => h.probe.installed && !h.incompatible && harnessVisible(h.id, features),
  )
  const [harness, setHarness] = useState('')
  const [model, setModel] = useState('')
  const [cwd, setCwd] = useState(() => localStorage.getItem(LAST_CWD) ?? '')
  const [scenario, setScenario] = useState('')
  const [error, setError] = useState<string | undefined>()
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (!harness && usable[0]) setHarness(usable[0].id)
  }, [harness, usable])
  const info = usable.find((h) => h.id === harness)
  const models = info?.capabilities[0]?.models ?? []
  const create = async () => {
    setBusy(true)
    setError(undefined)
    try {
      const s = await client.sessions.create({
        target: harness,
        cwd: cwd.trim(),
        ...(model ? { model } : {}),
        ...(harness === 'fake' && scenario.trim() ? { harness_opts: { scenario: scenario.trim() } } : {}),
      })
      localStorage.setItem(LAST_CWD, cwd.trim())
      onClose()
      void navigate({ to: '/s/$sessionId', params: { sessionId: s.id } })
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Session konnte nicht starten')
      setBusy(false)
    }
  }
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-foreground/30 p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-labelledby="new-session-title">
      <div className="w-full max-w-md rounded-lg border border-border bg-popover p-4 shadow-lg">
        <div className="flex items-center">
          <h2 id="new-session-title" className="type-wide text-[15px] font-semibold">
            Neue Session
          </h2>
          <button onClick={onClose} className="ml-auto flex size-7 items-center justify-center rounded-md hover:bg-accent" aria-label="Schließen">
            <X className="size-4" />
          </button>
        </div>
        <div className="mt-3 space-y-3 text-[13px]">
          <label className="block">
            Harness
            <select value={harness} onChange={(e) => setHarness(e.target.value)} className="mt-1 block w-full rounded-md border border-input bg-card px-2 py-1.5">
              {usable.map((h) => (
                <option key={h.id} value={h.id}>
                  {harnessName(h.id)}
                </option>
              ))}
            </select>
            {catalog.isSuccess && usable.length === 0 && (
              <span className="mt-1 block text-[12px] text-deny">Keine nutzbare Harness-CLI gefunden. Im Terminal: beton setup</span>
            )}
          </label>
          {models.length > 0 && (
            <label className="block">
              Modell
              <select value={model} onChange={(e) => setModel(e.target.value)} className="mt-1 block w-full rounded-md border border-input bg-card px-2 py-1.5 font-mono">
                <option value="">Standard der CLI</option>
                {models.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </label>
          )}
          <label className="block">
            Arbeitsverzeichnis
            <input value={cwd} onChange={(e) => setCwd(e.target.value)} placeholder="/pfad/zum/projekt" className="mt-1 block w-full rounded-md border border-input bg-card px-2 py-1.5 font-mono" />
          </label>
          {harness === 'fake' && (
            <label className="block">
              Szenario (Fake-Harness)
              <input value={scenario} onChange={(e) => setScenario(e.target.value)} placeholder="/pfad/zum/szenario.yaml" className="mt-1 block w-full rounded-md border border-input bg-card px-2 py-1.5 font-mono" />
            </label>
          )}
          {error && (
            <p className="text-[12px] text-deny" role="alert">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2 pt-1">
            <button onClick={onClose} className="rounded-md border border-border px-3 py-1.5">
              Abbrechen
            </button>
            <button onClick={() => void create()} disabled={busy || !harness || !cwd.trim()} className="chamfer-sm bg-foreground px-3 py-1.5 font-semibold text-background disabled:opacity-50">
              Session starten
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
