import { Cmd, Ln, Term } from '@/app/kit/harness-term'
import { F } from '@/proto/feature-marker'

export function SetupTerminal({ state }: { state: string }) {
  if (state === 'doctor')
    return (
      <Term>
        <F id={['HAR-003', 'HAR-014']}>
          <Cmd>beton doctor --harnesses</Cmd>
          <Ln>Harness        Pfad                          Version   Kompatibel  Anmeldung</Ln>
          <Ln>claude         /opt/homebrew/bin/claude      2.3.1     ja          angemeldet (Abo)</Ln>
          <Ln>codex          ~/.local/share/pnpm/codex     0.52.0    ja          angemeldet (Abo)</Ln>
          <Ln>acp:gemini     /opt/homebrew/bin/gemini      0.21.0    ja          unbekannt</Ln>
          <Ln tone="muted">acp:goose      –                             –         –           nicht installiert</Ln>
          <Ln>direct:ollama  http://127.0.0.1:11434/v1     –         ja          kein Schlüssel</Ln>
          <Ln />
          <Ln>Original-TUI</Ln>
          <Ln>  claude  Freigaben über Hooks (--settings), Tool-Prüfung: jeder Call</Ln>
          <Ln>  codex   Freigaben über Bildschirm-Spiegelung (Strategie B), Tool-Prüfung: nur Freigaben</Ln>
          <Ln tone="muted">          Lese-Zugriffe sind nur beobachtbar; die Sandbox ist der Backstop.</Ln>
          <Ln />
          <Ln tone="ok">✓ Alles in Ordnung. 4 Harnesses startklar.</Ln>
        </F>
      </Term>
    )
  if (state === 'json')
    return (
      <Term>
        <F id="HAR-016">
          <Cmd>beton setup --check --json | jq '.harnesses[0:2]'</Cmd>
          <Ln>{`[
  {
    "id": "claude",
    "installed": true,
    "path": "/opt/homebrew/bin/claude",
    "version": "2.3.1",
    "auth_status": "logged_in",
    "auth_source": "vendor_cli",
    "api_key_env_found": false
  },
  {
    "id": "codex",
    "installed": false,
    "version": null,
    "auth_status": "unknown",
    "api_key_env_found": false,
    "install_hint": "npm install -g @openai/codex"
  }
]`}</Ln>
        </F>
      </Term>
    )
  return (
    <Term>
      <F id={['HAR-016', 'HAR-015']}>
        <Cmd>beton setup</Cmd>
        <Ln>beton 0.4.0 · prüfe, welche Coding-Agents auf diesem Rechner nutzbar sind …</Ln>
        <Ln />
        <Ln>
          <span className="text-ok">✓</span> Claude Code   2.3.1  /opt/homebrew/bin/claude
        </Ln>
        <Ln tone="muted">                 über claude-CLI angemeldet (claude auth status) · Claude Max</Ln>
        <Ln>
          <span className="text-deny">✗</span> Codex         nicht installiert
        </Ln>
        <Ln>
          <span className="text-ok">✓</span> Gemini CLI    0.21.0 als acp:gemini · Anmeldestatus unbekannt
        </Ln>
        <Ln>
          <span className="text-ok">✓</span> Ollama        läuft auf 127.0.0.1:11434 · 3 Modelle
        </Ln>
        <Ln tone="muted">· LM Studio, goose, qwen: nicht gefunden</Ln>
        <Ln tone="muted">· OPENROUTER_API_KEY gefunden (sk-or-…3f9a), optional</Ln>
        <Ln />
        <Ln>Codex jetzt installieren? Ausgeführt wird:</Ln>
        <Ln>    npm install -g @openai/codex</Ln>
        <Ln>
          Installieren? <span className="text-signal">[j/N]</span> <span className="animate-pulse">▍</span>
        </Ln>
      </F>
    </Term>
  )
}

export function DevTerminal({ state }: { state: string }) {
  if (state === 'fake')
    return (
      <Term>
        <F id="HAR-026">
          <Cmd>beton run fake --dev --scenario tests/fake/push-ask.yaml -p "Bitte pushen"</Cmd>
          <Ln tone="muted">fake · Fähigkeiten: approval=native_request model_switch=live fork_history=preamble</Ln>
          <Ln>Ich pushe jetzt.</Ln>
          <Ln>
            ▸ Bash <span className="text-muted-foreground">git push origin main</span>
          </Ln>
          <Ln>
            <span className="text-signal">? Freigabe nötig: Shell · git push origin main</span> [e]rlauben / [a]blehnen: a
          </Ln>
          <Ln>Push abgelehnt.</Ln>
          <Ln tone="muted">1.200 → 80 Tokens · 0,01 $ (simuliert) · Event-Log: 14 Events, deterministisch</Ln>
        </F>
      </Term>
    )
  if (state === 'record-abort')
    return (
      <Term>
        <F id="HAR-025">
          <Cmd>beton dev record-golden --harness claude --scenario bash-tool</Cmd>
          <Ln tone="muted">nehme claude 2.3.1 auf … 38 Zeilen stdout</Ln>
          <Ln tone="muted">entferne bekannte Schlüssel-Muster und bt_cred_*-Platzhalter …</Ln>
          <Ln tone="deny">✗ Abgebrochen: raw.jsonl Zeile 17 enthält nach dem Bereinigen noch ein Muster wie „sk-ant-…“ (≥ 20 Zeichen).</Ln>
          <Ln tone="deny">  Nichts wurde gespeichert. Prüfe die Aufnahme und ergänze ggf. eine Scrub-Regel.</Ln>
        </F>
      </Term>
    )
  return (
    <Term>
      <F id="HAR-025">
        <Cmd>cargo nextest run -p beton-harness-claude golden</Cmd>
        <Ln>    Starting 14 tests across 1 binary</Ln>
        <Ln tone="ok">        PASS [   0.031s] golden::text_turn</Ln>
        <Ln tone="ok">        PASS [   0.044s] golden::bash_tool</Ln>
        <Ln tone="deny">        FAIL [   0.052s] golden::approval_deny</Ln>
        <Ln />
        <Ln>golden/approval-deny (aufgenommen mit claude 2.2.4, installiert 2.3.1)</Ln>
        <Ln tone="muted">expected.events.jsonl ↔ tatsächlich, Ereignis 7:</Ln>
        <Ln tone="del">- {`{"type":"tool.call.completed","status":"denied","reason":"nicht erlaubt"}`}</Ln>
        <Ln tone="add">+ {`{"type":"harness.unmapped","raw":{"type":"control_cancel_request", …}}`}</Ln>
        <Ln />
        <Ln tone="muted">Neu aufnehmen: beton dev record-golden --harness claude --scenario approval-deny</Ln>
        <Ln>     Summary [   0.611s] 14 tests run: 13 passed, 1 failed</Ln>
      </F>
    </Term>
  )
}
