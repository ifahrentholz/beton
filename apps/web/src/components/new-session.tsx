import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import type { HarnessInfo, PermissionMode } from '@beton/sdk'
import { X } from 'lucide-react'
import { client } from '@/lib/client'
import { harnessVisible, useFeatures } from '@/lib/features'
import { harnessName } from './harness'
import { EFFORT_LABEL, MODE_TEXT, YOLO_AVAILABLE } from './settings-picker'

const LAST_CWD = 'beton.lastCwd'

/**
 * Dialog „Neue Session“: Harness, Modell, Effort und Permission-Mode (aus den Capabilities,
 * WEB-004) sowie Arbeitsverzeichnis. YOLO bietet er ohne Sandbox nicht an (HAR-027 AC1).
 */
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
  const [effort, setEffort] = useState('')
  const [mode, setMode] = useState<PermissionMode | ''>('')
  const [cwd, setCwd] = useState(() => localStorage.getItem(LAST_CWD) ?? '')
  const [scenario, setScenario] = useState('')
  const [error, setError] = useState<string | undefined>()
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (!harness && usable[0]) setHarness(usable[0].id)
  }, [harness, usable])
  const info = usable.find((h) => h.id === harness)
  const caps = info?.capabilities[0]
  const models = caps?.models ?? []
  const efforts = caps?.efforts ?? []
  const modes = (caps?.permission_modes ?? []).filter((m) => m !== 'yolo' || YOLO_AVAILABLE)
  useEffect(() => {
    // Ein anderer Harness kennt evtl. andere Stufen und Modi.
    setEffort('')
    setMode('')
  }, [harness])
  const create = async () => {
    setBusy(true)
    setError(undefined)
    try {
      const s = await client.sessions.create({
        target: harness,
        cwd: cwd.trim(),
        ...(model ? { model } : {}),
        ...(effort ? { effort } : {}),
        ...(mode ? { permission_mode: mode } : {}),
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
          {efforts.length > 0 && (
            <label className="block">
              Effort
              <select value={effort} onChange={(e) => setEffort(e.target.value)} className="mt-1 block w-full rounded-md border border-input bg-card px-2 py-1.5">
                <option value="">Standard der CLI</option>
                {efforts.map((e) => (
                  <option key={e} value={e}>
                    {EFFORT_LABEL[e] ?? e}
                  </option>
                ))}
              </select>
            </label>
          )}
          {modes.length > 1 && (
            <label className="block">
              Permission-Mode
              <select value={mode} onChange={(e) => setMode(e.target.value as PermissionMode | '')} className="mt-1 block w-full rounded-md border border-input bg-card px-2 py-1.5">
                <option value="">{MODE_TEXT.default.title}</option>
                {modes
                  .filter((m) => m !== 'default')
                  .map((m) => (
                    <option key={m} value={m}>
                      {MODE_TEXT[m].title}
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
