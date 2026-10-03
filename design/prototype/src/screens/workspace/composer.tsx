import { FileText, Image, Sparkles, SquareSlash, TerminalSquare, X } from 'lucide-react'
import { Composer } from '@/app/session-chrome'
import { F } from '@/proto/feature-marker'
import { ShortStream } from './bits'
import { Menu, MenuItem, MenuLabel, MenuSeparator, SessionShell } from './parts'

function MentionMenu() {
  const items = [
    { path: 'src/middleware/index.ts', hint: 'exportiert alle Middlewares' },
    { path: 'src/middleware/rate-limit.ts', hint: 'neu in dieser Session' },
    { path: 'src/middleware/rate-limit.spec.ts', hint: 'neu in dieser Session' },
    { path: 'src/middleware/cors.ts', hint: '' },
    { path: 'docs/adr/0007-middleware-order.md', hint: '' },
  ]
  return (
    <Menu className="w-[460px]">
      <MenuLabel>Dateien im Workspace · 5 von 20.418 · 41 ms</MenuLabel>
      {items.map((it, i) => (
        <MenuItem key={it.path} active={i === 0} icon={<FileText className="size-3.5" />} hint={it.hint || undefined} right={i === 0 ? '↵ einfügen' : undefined}>
          <span className="font-mono text-[12px]">
            {it.path.split('mid').map((part, j, arr) => (
              <span key={j}>
                {part}
                {j < arr.length - 1 && <mark className="rounded-[2px] bg-foreground/15 font-semibold text-foreground">mid</mark>}
              </span>
            ))}
          </span>
        </MenuItem>
      ))}
    </Menu>
  )
}

function SlashMenu() {
  return (
    <Menu className="w-[480px]">
      <MenuLabel>beton</MenuLabel>
      <MenuItem active icon={<SquareSlash className="size-3.5" />} hint="Nebenfrage in einem Side-Chat stellen, ohne diese Session zu stören" right="/side">
        Side-Chat
      </MenuItem>
      <MenuItem icon={<SquareSlash className="size-3.5" />} hint="Verlauf vom Harness zusammenfassen lassen und Kontext freigeben" right="/compact">
        Kontext komprimieren
      </MenuItem>
      <MenuItem icon={<SquareSlash className="size-3.5" />} hint="Ab hier eine neue Session abzweigen, auch auf einem anderen Harness" right="/fork">
        Forken
      </MenuItem>
      <MenuSeparator />
      <MenuLabel>Skills des Agents „implementer“</MenuLabel>
      <MenuItem icon={<Sparkles className="size-3.5" />} hint="Prüft die Änderungen dieser Session wie ein Reviewer und listet Funde nach Schwere" right="/review">
        review
      </MenuItem>
      <MenuItem icon={<Sparkles className="size-3.5" />} hint="Schreibt einen Eintrag für CHANGELOG.md aus den Commits des Branches" right="/changelog">
        changelog
      </MenuItem>
      <MenuSeparator />
      <MenuLabel>Claude Code</MenuLabel>
      <MenuItem icon={<TerminalSquare className="size-3.5" />} hint="Wird direkt an Claude Code weitergegeben" right="/init">
        init
      </MenuItem>
    </Menu>
  )
}

function AttachmentStrip({ error }: { error?: boolean }) {
  return (
    <F id="WEB-006" className="flex flex-wrap items-end gap-2 px-4 pt-3">
      <div className="relative">
        <div className="flex h-16 w-24 flex-col justify-end overflow-hidden rounded-md border border-border bg-sunken">
          <div className="m-1.5 flex flex-col gap-1">
            <span className="h-1.5 w-14 rounded-sm bg-foreground/25" />
            <span className="h-1.5 w-10 rounded-sm bg-deny/50" />
            <span className="h-3 w-16 rounded-sm bg-foreground/15" />
          </div>
        </div>
        <span className="absolute top-1 right-1 flex size-4 items-center justify-center rounded-full bg-foreground text-background" aria-label="Anhang entfernen">
          <X className="size-2.5" />
        </span>
        <div className="mt-0.5 flex items-center gap-1 text-[11px] text-muted-foreground">
          <Image className="size-3" /> Bildschirmfoto · 412 KB
        </div>
      </div>
      <div>
        <div className="flex h-16 w-40 items-center gap-2 rounded-md border border-border bg-card px-2">
          <FileText className="size-5 shrink-0 text-muted-foreground" />
          <span className="min-w-0 text-[12px]">
            <span className="block truncate">pentest-bericht-q3.pdf</span>
            <span className="text-[11px] text-muted-foreground">PDF · 2,1 MB</span>
          </span>
        </div>
        <div className="mt-0.5 text-[11px] text-muted-foreground">2 von 10 Dateien</div>
      </div>
      {error && (
        <div className="mb-4 max-w-xs border-l-2 border-deny bg-deny-soft px-2 py-1 text-[12px]">
          <span className="font-semibold">login-trace.har nicht angehängt:</span> 48 MB, erlaubt sind 20 MB pro Datei. Kürze die Datei oder hänge einen Ausschnitt an.
        </div>
      )}
    </F>
  )
}

export function ComposerScreen({ state }: { state: string }) {
  const draft = state === 'mention' ? 'Wende das Muster aus @mid' : state === 'slash' ? '/' : 'Der Fehler aus dem Screenshot taucht nur hinter dem Load-Balancer auf.'
  return (
    <SessionShell
      composer={
        <div className="relative">
          {(state === 'mention' || state === 'slash') && (
            <F id="WEB-006" className="absolute bottom-[calc(100%-10px)] left-4 z-30">
              {state === 'mention' ? <MentionMenu /> : <SlashMenu />}
            </F>
          )}
          {(state === 'attachments' || state === 'too-large') && <AttachmentStrip error={state === 'too-large'} />}
          <Composer harness="claude" draft={draft} />
        </div>
      }
    >
      <ShortStream />
    </SessionShell>
  )
}
