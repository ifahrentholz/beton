import type { ReactNode } from 'react'
import { F } from '@/proto/feature-marker'

const P = () => <span className="text-ok">❯ </span>
const Dim = ({ children }: { children: ReactNode }) => <span className="text-muted-foreground">{children}</span>
const Ok = () => <span className="text-ok">✓ </span>

/** CLI-Sicht auf Sessions: run, attach, resume, fork, export, doctor. */
export function CliScreen({ state }: { state: string }) {
  return (
    <div className="dark text-foreground">
      {state === 'attach' && (
        <pre className="whitespace-pre-wrap">
          <F id="SES-001" as="span">
            <P />
            beton run claude --worktree{'\n'}
            <Ok />
            Session ses_01J9ZK7F3K angelegt · shop-frontend · Worktree beton/neue-session-7f3k von main{'\n'}
            <Dim>{'  Claude Code · claude-opus-5-5 · Claude Max (über claude-CLI angemeldet)\n  starting → idle\n'}</Dim>
          </F>
          {'\n'}
          <F id="SES-002" as="span">
            <P />
            beton attach ses_7f3k{'\n'}
            <Dim>Verbunden mit „Rate-Limiter für die Login-API“ ab Ereignis #1.802 · auch offen: Desktop, Browser{'\n\n'}</Dim>
            <span className="font-semibold">Ingo · Desktop</span>
            {'\n  Danach bitte auch die Register-Route absichern.\n\n'}
            <span className="text-voice-claude">■ Claude Code</span>
            {'\n  ✓ Bearbeiten  src/routes/auth.ts            0,3 s\n  ● Shell       pnpm vitest run auth         läuft …\n'}
            <Dim>{'\n  Ctrl+C trennt nur dieses Terminal · i: Nachricht · s: Stopp\n'}</Dim>
          </F>
        </pre>
      )}
      {state === 'resume' && (
        <pre className="whitespace-pre-wrap">
          <P />
          beton ls --status stopped{'\n'}
          <Dim>{'Id        Titel                       Harness      Zuletzt\n'}</Dim>
          {'ses_6m4d  Terraform-Plan erklären     Gemini CLI   gestern\nses_6h2f  Lokales Modell testen       Ollama       Mo.\n\n'}
          <F id="SES-003" as="span">
            <P />
            beton resume ses_6m4d{'\n'}
            <Dim>↻ Gemini CLI kann nicht nativ fortsetzen, Übergabe mit 5.880 Tokens{'\n'}</Dim>
            <Ok />
            fortgesetzt · idle{'\n\n'}
          </F>
          <F id={['SES-006', 'SES-007']} as="span">
            <P />
            beton run --fork ses_7f3k@148 --harness codex{'\n'}
            <Dim>Ereignis 148 liegt mitten in Turn 3, Fork beginnt am Ende von Turn 2 (141){'\n'}</Dim>
            <Ok />
            Fork ses_01J9ZM2M9Q · Codex (ChatGPT Pro) · Worktree beton/rate-limiter-fork-2m9q{'\n\n'}
          </F>
          <F id="SES-009" as="span">
            <P />
            beton export ses_7f3k -o ~/Downloads/ses_7f3k.jsonl{'\n'}
            <Ok />
            1.938 Ereignisse exportiert (1,2 MB) · keine Geheimnisse enthalten{'\n'}
          </F>
        </pre>
      )}
      {state === 'doctor' && (
        <pre className="whitespace-pre-wrap">
          <P />
          beton doctor{'\n'}
          <Ok />
          Daemon läuft auf localhost:7420, nur von diesem Rechner erreichbar{'\n'}
          <Ok />
          claude 2.4.1 · angemeldet mit Claude Max{'\n'}
          <Ok />
          codex 0.98.0 · angemeldet mit ChatGPT Pro{'\n'}
          <Ok />
          Sandbox: Seatbelt aktiv{'\n'}
          <F id="SES-016" as="span">
            <span className="font-semibold">! </span>1 verwaister Worktree{'\n'}
            {'    ~/.beton/worktrees/shop-frontend-3fa2c1d9/old-search-2b1x   388 MB\n'}
            <Dim>{'    Session am 12.09. gelöscht · Branch gemergt\n    Entfernen: beton worktree prune --orphaned\n'}</Dim>
          </F>
        </pre>
      )}
    </div>
  )
}
