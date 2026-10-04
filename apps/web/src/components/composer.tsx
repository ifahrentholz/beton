import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { BetonError, type Attachment, type Capabilities, type HarnessInfo, type PermissionMode } from '@beton/sdk'
import { ArrowUp, Paperclip, Square } from 'lucide-react'
import { client } from '@/lib/client'
import { ACCEPT, DEFAULT_LIMITS, insertMention, kindOf, mentionAt, mimeOf, pastedName, rejection, slashQuery, type AttachmentLimits } from '@/lib/composer'
import { cn } from '@/lib/utils'
import { AttachmentStrip, type DraftAttachment, type Rejected } from './attachments'
import { MentionMenu, SlashMenu, type SlashEntry } from './composer-menus'
import { HarnessMenu } from './fork'
import { HarnessBadge } from './harness'
import { ModePicker, ModelPicker, settingsOptions, type SettingsPatch } from './settings-picker'

const DRAFT_PREFIX = 'beton.draft.'

/** Wartezeit nach dem Tippen, bevor die `@`-Suche läuft. */
export const MENTION_DEBOUNCE_MS = 40
/** Treffer im `@`-Menü. */
export const MENTION_LIMIT = 8

/** Entwurf je Session in `localStorage` (WEB-004 AC3). */
export function loadDraft(sessionId: string): string {
  try {
    return localStorage.getItem(DRAFT_PREFIX + sessionId) ?? ''
  } catch {
    return ''
  }
}

export function saveDraft(sessionId: string, text: string): void {
  try {
    if (text) localStorage.setItem(DRAFT_PREFIX + sessionId, text)
    else localStorage.removeItem(DRAFT_PREFIX + sessionId)
  } catch {
    // Speicher nicht verfügbar (privates Fenster): Entwurf bleibt nur im Speicher.
  }
}

/** Wählbare Modelle und Effort-Stufen laut Capabilities (WEB-004 AC1); siehe `settingsOptions`. */
export function pickerOptions(caps: Capabilities | undefined): { models: string[]; efforts: string[] } {
  const { models, efforts } = settingsOptions(caps)
  return { models, efforts }
}

/** Eigener Befehl von beton im Slash-Menü, z. B. `/fork`. */
export interface BetonCommand {
  name: string
  title: string
  description: string
  run: () => void
}

/** Filtert das Slash-Menü: Präfix vor Teilstring. */
export function slashEntries(query: string, commands: BetonCommand[], skills: { name: string; description: string }[]): SlashEntry[] {
  const q = query.toLowerCase()
  const match = (name: string) => name.toLowerCase().includes(q)
  const rank = (name: string) => (name.toLowerCase().startsWith(q) ? 0 : 1)
  const beton: SlashEntry[] = commands.filter((c) => match(c.name)).map((c) => ({ name: c.name, title: c.title, description: c.description, group: 'beton' }))
  const skill: SlashEntry[] = skills.filter((s) => match(s.name)).map((s) => ({ name: s.name, title: s.name, description: s.description, group: 'skill' }))
  const sort = (a: SlashEntry, b: SlashEntry) => rank(a.name) - rank(b.name)
  return [...beton.sort(sort), ...skill.sort(sort)]
}

function problemText(e: unknown): string {
  if (e instanceof BetonError) {
    const detail = (e.problem as { detail?: unknown }).detail
    if (typeof detail === 'string') return detail.replace(/^.*? nicht angehängt:\s*/, '')
    return e.problem.title ?? e.message
  }
  return 'Hochladen fehlgeschlagen.'
}

let nextKey = 0

/**
 * Eingabe mit Pickern (WEB-004): Harness, Modell, Effort und Permission-Mode laut Capabilities;
 * ein Wechsel geht per `PATCH` an den Server und gilt ab dem nächsten Turn (HAR-017, HAR-027).
 * Erweiterungen (WEB-006): Anhänge per Button, Drop oder
 * Paste, `@` für Dateien im Workspace, `/` für Befehle und Skills. Senden mit ⏎,
 * Zeilenumbruch mit ⇧⏎. Während eines Turns heißt der Senden-Button „Einreihen“; die Queue
 * (WEB-005) steht über dem Eingabefeld.
 */
export function Composer({
  sessionId,
  harness,
  model,
  effort,
  permissionMode,
  capabilities,
  running,
  queue,
  onSend,
  onInterrupt,
  onSettings,
  continueWith,
  onContinue,
  commands = [],
}: {
  sessionId: string
  harness: string
  model?: string | null | undefined
  effort?: string | null | undefined
  permissionMode?: PermissionMode | null | undefined
  capabilities?: Capabilities | undefined
  running: boolean
  /** Serverseitige Queue über dem Eingabefeld (WEB-005). */
  queue?: ReactNode
  onSend: (text: string, attachments: Attachment[]) => void
  onInterrupt: () => void
  /** Modell, Effort oder Permission-Mode wechseln (HAR-017, HAR-027). */
  onSettings?: ((patch: SettingsPatch) => void) | undefined
  /** Harnesses für „Weiter mit …“ im Harness-Picker (SES-007 AC5). */
  continueWith?: HarnessInfo[] | undefined
  /** Fork ab dem letzten `seq` auf einen anderen Harness. */
  onContinue?: ((harness: string) => void) | undefined
  /** beton-eigene Slash-Befehle, die es schon gibt. */
  commands?: BetonCommand[]
}) {
  const [text, setText] = useState(() => loadDraft(sessionId))
  const [caret, setCaret] = useState(0)
  const [focused, setFocused] = useState(false)
  const [dismissed, setDismissed] = useState<string | undefined>()
  const [active, setActive] = useState(0)
  const [attachments, setAttachments] = useState<DraftAttachment[]>([])
  const [rejected, setRejected] = useState<Rejected[]>([])
  const [dragging, setDragging] = useState(false)
  const ref = useRef<HTMLTextAreaElement>(null)
  const fileInput = useRef<HTMLInputElement>(null)
  const pendingCaret = useRef<number | undefined>(undefined)
  useEffect(() => setText(loadDraft(sessionId)), [sessionId])
  useEffect(() => saveDraft(sessionId, text), [sessionId, text])
  useEffect(() => {
    const el = ref.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`
    if (pendingCaret.current !== undefined) {
      el.setSelectionRange(pendingCaret.current, pendingCaret.current)
      setCaret(pendingCaret.current)
      pendingCaret.current = undefined
    }
  }, [text])
  const { models, efforts, modes, modelSwitch } = settingsOptions(capabilities)

  const info = useQuery({ queryKey: ['info'], queryFn: () => client.info(), staleTime: 60_000 })
  const limits: AttachmentLimits = info.data?.attachments ?? DEFAULT_LIMITS

  // --- `@`-Erwähnungen (WEB-006 AC2) -------------------------------------------------------
  const mention = focused ? mentionAt(text, caret) : undefined
  const slash = focused ? slashQuery(text) : undefined
  const menuKey = mention ? `@${mention.start}` : slash !== undefined ? '/' : undefined
  const menuOpen = menuKey !== undefined && dismissed !== menuKey
  const [hits, setHits] = useState<{ query: string; paths: string[]; total?: number | undefined; ms: number } | undefined>()
  const warmed = useRef<string | undefined>(undefined)
  const warm = useCallback(() => {
    // Index beim ersten Fokus laden, damit die erste `@`-Suche sofort antwortet.
    if (warmed.current === sessionId) return
    warmed.current = sessionId
    void client
      .session(sessionId)
      .workspace.search('', 'fuzzy', { limit: 1 })
      .catch(() => undefined)
  }, [sessionId])
  const mentionQuery = mention?.query
  useEffect(() => {
    if (mentionQuery === undefined) return
    let cancelled = false
    const timer = setTimeout(() => {
      const started = performance.now()
      client
        .session(sessionId)
        .workspace.search(mentionQuery, 'fuzzy', { limit: MENTION_LIMIT })
        .then((page) => {
          if (!cancelled) setHits({ query: mentionQuery, paths: page.items.map((h) => h.path), total: page.total, ms: performance.now() - started })
        })
        .catch(() => undefined)
    }, MENTION_DEBOUNCE_MS)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [mentionQuery, sessionId])

  // --- Slash-Menü (WEB-006 AC3) -------------------------------------------------------------
  const skills = useQuery({
    queryKey: ['skills', sessionId],
    queryFn: () => client.session(sessionId).skills(),
    enabled: slash !== undefined,
    staleTime: 30_000,
  })
  const entries = useMemo(
    () => (slash === undefined ? [] : slashEntries(slash, commands, skills.data?.items ?? [])),
    [slash, commands, skills.data],
  )

  const mentionPaths = mention && hits?.query === mention.query ? hits.paths : []
  const count = menuOpen ? (mention ? mentionPaths.length : entries.length) : 0
  useEffect(() => setActive(0), [menuKey, mention?.query, slash])

  const pickMention = (path: string) => {
    const next = insertMention(text, caret, path)
    pendingCaret.current = next.caret
    setText(next.text)
  }
  const pickSlash = (e: SlashEntry) => {
    if (e.group === 'beton') {
      commands.find((c) => c.name === e.name)?.run()
      setText('')
      return
    }
    const next = `/${e.name} `
    pendingCaret.current = next.length
    setText(next)
  }

  // --- Anhänge (WEB-006 AC1) -----------------------------------------------------------------
  const addFiles = (files: File[]) => {
    if (files.length === 0) return
    const fresh: DraftAttachment[] = []
    const refused: Rejected[] = []
    let total = attachments.length
    for (const file of files) {
      const name = file.type.startsWith('image/') ? pastedName(file) : file.name || 'Anhang'
      const why = rejection({ name, size: file.size, type: file.type }, total, limits)
      if (why) {
        refused.push({ name, reason: why })
        continue
      }
      total += 1
      const mime = mimeOf(name, file.type)
      const kind = kindOf(mime) ?? 'text'
      const key = `att${(nextKey += 1)}`
      fresh.push({ key, name, mime, size: file.size, kind, preview: kind === 'image' ? URL.createObjectURL(file) : undefined })
      client
        .session(sessionId)
        .uploadAttachment(file, name, mime)
        .then((uploaded) => setAttachments((list) => list.map((a) => (a.key === key ? { ...a, uploaded } : a))))
        .catch((e: unknown) => setAttachments((list) => list.map((a) => (a.key === key ? { ...a, error: problemText(e) } : a))))
    }
    setRejected(refused)
    if (fresh.length > 0) setAttachments((list) => [...list, ...fresh])
  }
  const remove = (key: string) => {
    setAttachments((list) => {
      const gone = list.find((a) => a.key === key)
      if (gone?.preview) URL.revokeObjectURL(gone.preview)
      return list.filter((a) => a.key !== key)
    })
    setRejected([])
  }
  const uploading = attachments.some((a) => !a.uploaded && !a.error)
  const ready = attachments.filter((a) => a.uploaded).map((a) => a.uploaded as Attachment)

  const send = () => {
    const t = text.trim()
    if ((!t && ready.length === 0) || uploading) return
    onSend(t, ready)
    for (const a of attachments) if (a.preview) URL.revokeObjectURL(a.preview)
    setAttachments([])
    setRejected([])
    setText('')
  }

  return (
    <div
      className="shrink-0 border-t border-border p-3"
      onDragOver={(e) => {
        if (![...e.dataTransfer.types].includes('Files')) return
        e.preventDefault()
        setDragging(true)
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(e) => {
        if (e.dataTransfer.files.length === 0) return
        e.preventDefault()
        setDragging(false)
        addFiles([...e.dataTransfer.files])
      }}
    >
      {queue}
      <AttachmentStrip items={attachments} rejected={rejected} maxFiles={limits.max_files} onRemove={remove} />
      <div className="relative">
        {menuOpen && count >= 0 && (
          <div className="absolute bottom-[calc(100%+4px)] left-1 z-30">
            {mention ? (
              <MentionMenu query={mention.query} paths={mentionPaths} total={hits?.total} ms={hits?.ms} active={active} onPick={pickMention} />
            ) : (
              <SlashMenu entries={entries} agent={skills.data?.agent} active={active} onPick={pickSlash} />
            )}
          </div>
        )}
        <div className={cn('rounded-md border border-input bg-card focus-within:outline-2 focus-within:outline-ring', dragging && 'outline-2 outline-ring')}>
          <textarea
            ref={ref}
            value={text}
            rows={2}
            aria-label="Nachricht"
            aria-autocomplete="list"
            aria-expanded={menuOpen}
            aria-activedescendant={menuOpen && count > 0 ? `${mention ? 'mention' : 'slash'}-${active}` : undefined}
            placeholder="Nachricht an den Agent – @ für Dateien, / für Befehle"
            onFocus={() => {
              setFocused(true)
              warm()
            }}
            onBlur={() => setFocused(false)}
            onChange={(e) => {
              setText(e.target.value)
              setCaret(e.target.selectionStart)
              setDismissed(undefined)
            }}
            onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
            onPaste={(e) => {
              const files = [...e.clipboardData.files]
              if (files.length === 0) return
              if (!e.clipboardData.getData('text/plain')) e.preventDefault()
              addFiles(files)
            }}
            onKeyDown={(e) => {
              if (menuOpen && !e.nativeEvent.isComposing) {
                if (e.key === 'ArrowDown' && count > 0) {
                  e.preventDefault()
                  setActive((a) => (a + 1) % count)
                  return
                }
                if (e.key === 'ArrowUp' && count > 0) {
                  e.preventDefault()
                  setActive((a) => (a - 1 + count) % count)
                  return
                }
                if (e.key === 'Escape') {
                  e.preventDefault()
                  setDismissed(menuKey)
                  return
                }
                if ((e.key === 'Enter' || e.key === 'Tab') && !e.shiftKey && count > 0) {
                  e.preventDefault()
                  if (mention) {
                    const p = mentionPaths[active]
                    if (p) pickMention(p)
                  } else {
                    const entry = entries[active]
                    if (entry) pickSlash(entry)
                  }
                  return
                }
              }
              if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault()
                send()
              }
            }}
            className="block min-h-[52px] w-full resize-none bg-transparent px-3 pt-2.5 text-[14px] outline-none placeholder:text-muted-foreground"
          />
          <div className="flex flex-wrap items-center gap-0.5 px-1.5 pb-1.5">
            <button
              type="button"
              onClick={() => fileInput.current?.click()}
              className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent"
              aria-label="Datei anhängen"
              title="Bilder, PDF oder Textdateien anhängen – auch per Drop oder Einfügen"
            >
              <Paperclip className="size-4" />
            </button>
            <input
              ref={fileInput}
              type="file"
              multiple
              accept={ACCEPT}
              className="hidden"
              data-testid="attach-input"
              onChange={(e) => {
                addFiles([...(e.target.files ?? [])])
                e.target.value = ''
              }}
            />
            {onContinue ? (
              <HarnessMenu harness={harness} targets={continueWith ?? []} onContinue={onContinue} />
            ) : (
              <span className="px-1.5">
                <HarnessBadge harness={harness} />
              </span>
            )}
            <ModelPicker
              harness={harness}
              model={model}
              effort={effort}
              models={models}
              efforts={efforts}
              modelSwitch={modelSwitch}
              running={running}
              onChange={onSettings}
            />
            <ModePicker harness={harness} mode={permissionMode} modes={modes} running={running} onChange={onSettings} />
            <div className="ml-auto flex items-center gap-1">
              {running && (
                <button onClick={onInterrupt} className="flex h-7 items-center gap-1 rounded-md border border-foreground/40 px-2 text-xs" aria-label="Unterbrechen">
                  <Square className="size-3 fill-current" /> Stopp
                </button>
              )}
              <button
                onClick={send}
                disabled={(!text.trim() && ready.length === 0) || uploading}
                className={cn('flex h-7 items-center gap-1 rounded-md bg-foreground px-2 text-xs text-background disabled:opacity-40')}
                aria-label={running ? 'Einreihen' : 'Senden'}
              >
                <ArrowUp className="size-4" />
                <span className="hidden sm:inline">{running ? 'Einreihen' : 'Senden'}</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
