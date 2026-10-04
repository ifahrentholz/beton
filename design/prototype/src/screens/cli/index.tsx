import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { ApiReference } from './api'
import { Ask, Bar, Blank, Comment, Cursor, ExitCode, L, Prompt, S, Term, type Tone } from './term'

/**
 * Gruppe „Kommandozeile“: echte Terminal-Sitzungen mit `beton`.
 * Farben wie im Terminal: Stimmfarben für Harnesses, Schalungsgelb nur für Rückfragen
 * an dich (Freigaben, Installationen), Rot für Fehler und Exit-Codes ≠ 0.
 */

const voiceTone: Record<string, Tone> = { 'Claude Code': 'claude', Codex: 'codex', 'Gemini CLI': 'acp', 'Ollama (lokal)': 'direct' }
const H = ({ name, pad = 0 }: { name: string; pad?: number }) => <S tone={voiceTone[name] ?? 'plain'}>{name.padEnd(pad)}</S>

/* ───────────────────────── beton run (interaktiv) ───────────────────────── */

function Tool({ name, target, meta, status = 'ok' }: { name: string; target: string; meta: string; status?: 'ok' | 'wait' | 'run' | 'fail' }) {
  return (
    <L>
      {'  '}
      <S tone="dim">⎿ </S>
      <S tone="bold">{name.padEnd(11)}</S>
      {target.padEnd(46)}
      <S tone="dim">{meta} </S>
      {status === 'ok' && <S tone="ok">✓</S>}
      {status === 'fail' && <S tone="deny">✗</S>}
      {status === 'run' && <S tone="dim">…</S>}
      {status === 'wait' && <S tone="signal">wartet auf dich</S>}
    </L>
  )
}

function RunHeader({ daemon = true, session = 'ses_7f3k' }: { daemon?: boolean; session?: string }) {
  return (
    <F id="CLI-002" badge="top-right">
      {daemon && <L tone="dim">Lokaler Daemon läuft nicht – starte ihn … ok (pid 48211, 127.0.0.1:7420)</L>}
      <L tone="dim">
        Session {session} · <S tone="claude">Claude Code</S> · claude-opus-5-5 · Subscription Claude Max (über claude-CLI angemeldet)
      </L>
      <L tone="dim">Im Browser: http://127.0.0.1:7420/s/{session}</L>
      <L tone="dim">Ctrl+C unterbricht den Turn · 2× Ctrl+C trennt, die Session läuft weiter</L>
    </F>
  )
}

function CliRun({ state }: { state: string }) {
  if (state === 'continue') {
    return (
      <Term>
        <Prompt>beton run -c</Prompt>
        <L tone="dim">Setze ses_7f3k fort – zuletzt in diesem Verzeichnis benutzt (vor 4 Min.)</L>
        <RunHeader daemon={false} />
        <Blank />
        <L tone="dim">… 38 frühere Events · ganzer Verlauf: beton attach ses_7f3k</L>
        <L>
          <S tone="claude">● Claude Code</S>
        </L>
        <L>{'  '}Alle 10 Tests laufen. Soll ich den Branch pushen und einen PR öffnen?</L>
        <Blank />
        <L>
          <S tone="bold">› </S>Erst noch die Register-Route mit demselben Limit absichern. <Cursor />
        </L>
      </Term>
    )
  }
  if (state === 'worktree') {
    return (
      <Term>
        <Prompt>beton run claude --worktree --title "Register-Route absichern"</Prompt>
        <F id="CLI-002">
          <L tone="dim">Worktree angelegt: ../shop-frontend.worktrees/register-limit-8a2c</L>
          <L tone="dim">Branch beton/register-limit-8a2c von main (a41c0e2) · dein Arbeitsverzeichnis bleibt unberührt</L>
        </F>
        <RunHeader daemon={false} session="ses_8a2c" />
        <Blank />
        <L>
          <S tone="bold">› </S>
          <Cursor />
        </L>
      </Term>
    )
  }
  return (
    <Term>
      <Prompt>beton run claude</Prompt>
      <RunHeader />
      <Blank />
      <L>
        <S tone="bold">› </S>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.
      </L>
      <Blank />
      <L>
        <S tone="claude">● Claude Code</S>
      </L>
      <L tone="italic">{'  '}Überlegung: Middleware oder Gateway? (eingeklappt, „r“ zeigt sie)</L>
      <Tool name="Suche" target="rg 'auth.post' src/" meta="0,2 s" />
      <Tool name="Lesen" target="src/routes/auth.ts" meta="0,1 s" />
      <Tool name="Bearbeiten" target="src/routes/auth.ts  +2 −1" meta="0,3 s" />
      <Tool name="Shell" target="pnpm vitest run auth" meta="4,8 s · 10 Tests bestanden" />
      <L>{'  '}Die Login-Route ist jetzt auf 5 Versuche pro Minute und IP begrenzt. Danach antwortet sie</L>
      <L>
        {'  '}mit <S tone="bold">429</S> und einem <S tone="bold">Retry-After</S>-Header.
        {state === 'streaming' && <Cursor />}
      </L>
      {state !== 'streaming' && <L>{'  '}Alle 10 Tests laufen. Soll ich den Branch pushen und einen PR öffnen?</L>}
      {state === 'approval' && (
        <>
          <L tone="dim">{'  '}1 Min. 12 s · 38.410 Tokens · Subscription</L>
          <Blank />
          <L>
            <S tone="bold">› </S>Ja, push und PR.
          </L>
          <Blank />
          <L>
            <S tone="claude">● Claude Code</S>
          </L>
          <Tool name="Shell" target="git push -u origin beton/rate-limiter-7f3k" meta="" status="wait" />
          <F id={['CLI-002', 'CLI-001']}>
            <Ask title="Freigabe nötig · Shell · Regel git-push-fragen (Projekt shop-frontend)" question="Erlauben?" choice="[y/N/s = für diese Session]">
              <L>{'  '}git push -u origin beton/rate-limiter-7f3k</L>
              <L tone="dim">{'  '}Pushes verlassen deinen Rechner. Die Projekt-Policy verlangt dafür deine Freigabe.</L>
            </Ask>
          </F>
        </>
      )}
      {state === 'interrupted' && (
        <F id="CLI-002">
          <L tone="dim">^C</L>
          <L tone="dim">Turn unterbrochen. Der laufende Tool-Call wurde abgebrochen.</L>
          <L>
            <S tone="bold">› </S>
          </L>
          <L tone="dim">^C^C</L>
          <L tone="dim">Getrennt. Die Session läuft weiter – wieder verbinden mit: beton attach ses_7f3k</L>
          <Prompt>
            <Cursor />
          </Prompt>
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── Skript-Modus ───────────────────────── */

function CliScript({ state }: { state: string }) {
  return (
    <Term>
      {state === 'text' && (
        <F id={['API-006', 'CLI-001']}>
          <Comment>stdout bekommt nur die Antwort, Fortschritt und URL gehen auf stderr</Comment>
          <Prompt>echo "Fasse die Änderungen seit v2.3 in drei Sätzen zusammen" | beton run claude -p - &gt; out.txt</Prompt>
          <L tone="dim">Session ses_8b1q · http://127.0.0.1:7420/s/ses_8b1q</L>
          <L tone="dim">● Lesen CHANGELOG.md · ● Shell git log v2.3..HEAD --oneline</L>
          <L tone="dim">fertig in 18 s · 12.904 Tokens · Subscription Claude Max</L>
          <Prompt>cat out.txt</Prompt>
          <L>Seit v2.3 begrenzt die Login-Route Anmeldeversuche auf fünf pro Minute und IP. Das Checkout-Formular</L>
          <L>ist per Tastatur bedienbar und meldet Fehler an Screenreader. Außerdem wurde der flaky Test in</L>
          <L>payment_spec durch eine feste Uhr ersetzt.</L>
          <ExitCode code={0} />
        </F>
      )}
      {state === 'json' && (
        <F id="API-006">
          <Prompt>beton run claude -p "Welche Tests sind flaky?" --output-format json | jq</Prompt>
          <L tone="dim">Session ses_8b2r · http://127.0.0.1:7420/s/ses_8b2r</L>
          <L>{'{'}</L>
          <L>
            {'  '}
            <S tone="codex">"session_id"</S>: <S tone="ok">"ses_8b2r"</S>,
          </L>
          <L>
            {'  '}
            <S tone="codex">"status"</S>: <S tone="ok">"completed"</S>,
          </L>
          <L>
            {'  '}
            <S tone="codex">"result"</S>: <S tone="ok">"payment_spec.ts:88 hängt von der Systemuhr ab; checkout.e2e.ts:41 wartet auf eine feste Zeit."</S>,
          </L>
          <L>
            {'  '}
            <S tone="codex">"cost_usd"</S>: <S tone="dim">null</S>,
          </L>
          <L>
            {'  '}
            <S tone="codex">"usage"</S>: {'{ '}
            <S tone="codex">"input_tokens"</S>: 41210, <S tone="codex">"output_tokens"</S>: 1874, <S tone="codex">"billing"</S>:{' '}
            <S tone="ok">"subscription"</S>, <S tone="codex">"plan"</S>: <S tone="ok">"Claude Max"</S>
            {' }'},
          </L>
          <L>
            {'  '}
            <S tone="codex">"duration_ms"</S>: 26410
          </L>
          <L>{'}'}</L>
          <Comment>Subscription: kein Euro-Betrag, cost_usd bleibt null</Comment>
        </F>
      )}
      {state === 'stream' && (
        <F id="API-006">
          <Prompt>beton run codex -p "Aktualisiere die Lockfile" --output-format stream-json</Prompt>
          {[
            ['1', 'session.created', '"harness":"codex","model":"gpt-5.3-codex"'],
            ['2', 'turn.started', '"turn_id":"t_01"'],
            ['3', 'tool.started', '"tool":"shell","command":"pnpm install --lockfile-only"'],
            ['4', 'tool.output', '"stream":"stdout","text":"Lockfile is up to date, resolution step is skipped"'],
            ['5', 'tool.completed', '"exit_code":0,"duration_ms":2310'],
            ['6', 'message.delta', '"text":"Die Lockfile ist aktuell; "'],
            ['7', 'message.delta', '"text":"es gab nichts zu ändern."'],
            ['8', 'turn.completed', '"usage":{"input_tokens":8120,"output_tokens":214,"billing":"subscription"}'],
          ].map(([seq, type, rest]) => (
            <L key={seq}>
              {'{'}
              <S tone="codex">"seq"</S>:{seq},<S tone="codex">"type"</S>:<S tone="ok">"{type}"</S>,{rest}
              {'}'}
            </L>
          ))}
        </F>
      )}
      {state === 'deny' && (
        <F id={['API-006', 'CLI-001']}>
          <Comment>CI: Freigaben ohne Zuschauer sofort ablehnen statt zu hängen</Comment>
          <Prompt cwd="~/ci/shop-frontend">beton run codex -p "Aktualisiere die Abhängigkeiten und pushe" --on-ask deny</Prompt>
          <L tone="dim">Session ses_8b3s · http://127.0.0.1:7420/s/ses_8b3s</L>
          <L tone="dim">● Shell pnpm update --latest ✓ · ● Shell pnpm test ✓</L>
          <L tone="deny">✗ Freigabe abgelehnt (--on-ask deny): Shell git push origin HEAD · Regel git-push-fragen</L>
          <L tone="deny">Lauf beendet: Der Agent brauchte eine Freigabe, aber niemand schaut zu.</L>
          <ExitCode code={4} cwd="~/ci/shop-frontend" />
        </F>
      )}
      {state === 'budget' && (
        <F id="API-006">
          <Comment>Optional: Lauf mit API-Schlüssel aus dem Secret-Store statt Subscription, mit Budget</Comment>
          <Prompt cwd="~/ci/shop-frontend">beton run claude --profile ci-api -p "Schreib Tests für src/cart" --max-cost 2 --timeout 15m</Prompt>
          <L tone="dim">Session ses_8b4t · Abrechnung über API-Schlüssel anthropic/ci (Budget 2,00 USD)</L>
          <L tone="dim">● Bearbeiten src/cart/cart.spec.ts ✓ · ● Shell pnpm vitest run cart ✓</L>
          <L tone="deny">✗ Budget erschöpft: 2,04 USD von 2,00 USD. Der Turn wurde beendet, die Session bleibt fortsetzbar.</L>
          <ExitCode code={5} cwd="~/ci/shop-frontend" />
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── Sessions ───────────────────────── */

const sessionRows: [string, string, string, string, string, Tone][] = [
  ['ses_7f3k', 'Rate-Limiter für die Login-API', 'Claude Code', 'wartet auf dich', 'jetzt', 'signal'],
  ['ses_7f3m', 'Review: Rate-Limiter', 'Codex', 'läuft', 'vor 1 Min.', 'ok'],
  ['ses_6q2a', 'Checkout-Formular barrierefrei machen', 'Claude Code', 'bereit', 'vor 12 Min.', 'plain'],
  ['ses_6p9z', 'Event-Log: seq lückenlos halten', 'Codex', 'bereit', 'vor 1 Std.', 'plain'],
  ['ses_6n1c', 'Nächtliches Dependency-Update', 'Claude Code', 'läuft', 'vor 2 Std.', 'ok'],
  ['ses_6m4d', 'Terraform-Plan erklären', 'Gemini CLI', 'gestoppt', 'gestern', 'dim'],
  ['ses_6k8e', 'Flaky Test in payment_spec', 'Claude Code', 'Fehler', 'gestern', 'deny'],
]

function CliSessions({ state }: { state: string }) {
  return (
    <Term>
      {state === 'list' && (
        <F id={['CLI-003', 'CLI-001']}>
          <Prompt>beton session list</Prompt>
          <L tone="dim">{'ID        TITEL                                   HARNESS       STATUS           AKTUALISIERT'}</L>
          {sessionRows.map(([id, title, h, st, upd, tone]) => (
            <L key={id}>
              {id}
              {'  '}
              {title.padEnd(40)}
              <H name={h} pad={14} />
              <S tone={tone}>{st.padEnd(17)}</S>
              <S tone="dim">{upd}</S>
            </L>
          ))}
          <Blank />
          <Prompt>beton session rename last "Rate-Limiter für Login und Register"</Prompt>
          <L>ses_7f3k umbenannt</L>
        </F>
      )}
      {state === 'attach' && (
        <F id="CLI-003">
          <Prompt>beton attach 7f3m --read-only</Prompt>
          <L tone="dim">Verbunden mit ses_7f3m „Review: Rate-Limiter“ · nur lesen · Replay ab seq 1 (142 Events)</L>
          <L tone="dim">──────────────── Verlauf ────────────────</L>
          <L>
            <S tone="bold">› </S>Bitte reviewe den Rate-Limiter auf beton/rate-limiter-7f3k.
          </L>
          <L>
            <S tone="codex">● Codex</S>
          </L>
          <Tool name="Shell" target="git diff main...beton/rate-limiter-7f3k" meta="0,4 s" />
          <Tool name="Lesen" target="src/middleware/rate-limit.ts" meta="0,1 s" />
          <L tone="dim">──────────────── live ────────────────</L>
          <Tool name="Shell" target="pnpm vitest run rate-limit --reporter=dot" meta="läuft" status="run" />
          <L>
            {'  '}Der Token-Bucket wird nie aufgeräumt; bei vielen IPs wächst die Map unbegrenzt.
            <Cursor />
          </L>
          <Blank />
          <L tone="dim">Nur lesen: Eingaben sind aus. Ctrl+C trennt.</L>
        </F>
      )}
      {state === 'ambiguous' && (
        <F id={['CLI-003', 'CLI-001']}>
          <Prompt>beton attach 6</Prompt>
          <L tone="deny">Fehler: „6“ passt zu 5 Sessions. Gib mehr Zeichen der ID an:</L>
          {sessionRows
            .filter(([id]) => id.startsWith('ses_6'))
            .map(([id, title, h]) => (
              <L key={id}>
                {'  '}
                {id}
                {'  '}
                {title.padEnd(40)}
                <H name={h} />
              </L>
            ))}
          <ExitCode code={2} />
        </F>
      )}
      {state === 'fork' && (
        <F id="CLI-003">
          <Prompt>beton session fork ses_6p9z@120 --harness codex</Prompt>
          <L>
            Fork <S tone="bold">ses_9c1d</S> aus ses_6p9z bei seq 120 · <S tone="codex">Codex</S> · gpt-5.3-codex
          </L>
          <L tone="dim">Kontext übergeben: 120 Events, Zusammenfassung 2.840 Tokens · Branch wp-03-persistence-fork-9c1d</L>
          <Blank />
          <Prompt>beton resume ses_6m4d</Prompt>
          <L tone="dim">
            Starte ses_6m4d neu (gestoppt gestern 18:42) · <S tone="acp">Gemini CLI</S> · gemini-3-pro
          </L>
          <L tone="dim">Verlauf wiederhergestellt (64 Events). Weiter mit deiner Nachricht.</L>
          <L>
            <S tone="bold">› </S>
            <Cursor />
          </L>
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── serve & host ───────────────────────── */

function CliServe({ state }: { state: string }) {
  return (
    <Term>
      {state === 'local' && (
        <F id="CLI-004">
          <Prompt>beton serve</Prompt>
          <L>
            <S tone="bold">beton 1.3.0</S> · Server auf diesem Rechner
          </L>
          <L>{'  '}Bindung      127.0.0.1:7420 (nur Loopback)</L>
          <L>{'  '}Daten        ~/.beton/beton.db (SQLite, 412 MB)</L>
          <L>{'  '}Token-Datei  ~/.beton/token (0600)</L>
          <L>
            {'  '}Harnesses    <S tone="claude">claude 2.3.1</S> <S tone="ok">✓</S> angemeldet (Claude Max) · <S tone="codex">codex 0.61.0</S>{' '}
            <S tone="ok">✓</S> angemeldet (ChatGPT Pro)
          </L>
          <L>{'  '}Host         local (eingebaut) · Provider local</L>
          <L tone="ok">Bereit. Keine Verbindungen nach außen – außer denen der Vendor-CLIs zu ihren Modell-Anbietern.</L>
          <Cursor />
        </F>
      )}
      {state === 'refused' && (
        <F id={['CLI-004', 'CLI-001']}>
          <Prompt>beton serve --bind 0.0.0.0</Prompt>
          <L tone="deny">Fehler: --bind 0.0.0.0 ist ohne konfigurierte Anmeldung nicht erlaubt.</L>
          <L>{'  '}Sonst könnte jeder in deinem Netz Sessions starten. Bleib bei 127.0.0.1 oder richte OIDC ein</L>
          <L>{'  '}(server.auth in ~/.beton/config.yaml). Fürs Handy reicht ein Einmal-Link: beton open</L>
          <ExitCode code={2} />
        </F>
      )}
      {state === 'pair' && (
        <F id={['CLI-004', 'CLI-006']}>
          <Comment>Optional: ein zweiter Rechner als Host an einem selbst betriebenen Team-Server</Comment>
          <Prompt host="ingo@build-box-01" cwd="~">
            beton host pair --server https://beton.team.example
          </Prompt>
          <div className="chamfer my-1 border-l-2 border-signal bg-signal-soft px-3 py-1.5">
            <div className="font-semibold text-signal">Pairing-Code: K7QF-2M9X</div>
            <div>Bestätige den Code in beton unter Einstellungen → Hosts → Host hinzufügen (gültig 10 Min.).</div>
            <div className="text-muted-foreground">
              Warte auf Bestätigung … <Cursor />
            </div>
          </div>
          <L tone="ok">✓ Bestätigt von ingo@diva-e.com</L>
          <L>
            Host <S tone="bold">build-box-01</S> registriert · Labels os=linux arch=amd64 repo=beton tier=ci
          </L>
          <Prompt host="ingo@build-box-01" cwd="~">
            beton host --background
          </Prompt>
          <L tone="dim">Tunnel zu wss://beton.team.example/v1/tunnel aufgebaut (ausgehend, kein offener Port)</L>
          <L>
            Provider: local <S tone="ok">✓</S> · docker <S tone="ok">✓</S> Podman 5.2 rootless
          </L>
        </F>
      )}
      {state === 'enable' && (
        <F id="CLI-004">
          <Prompt host="ingo@build-box-01" cwd="~">
            beton host enable
          </Prompt>
          <L>User-Dienst angelegt: ~/.config/systemd/user/beton-host.service</L>
          <L tone="ok">✓ Aktiv. Der Host verbindet sich nach jedem Neustart von selbst.</L>
          <Prompt host="ingo@build-box-01" cwd="~">
            beton host status
          </Prompt>
          <L>
            build-box-01 · <S tone="ok">online</S> seit 3 Tg. · Tunnel ok (Ping 18 ms) · Runner 1/8 · Dienst systemd --user (aktiv)
          </L>
          <Prompt host="ingo@build-box-01" cwd="~">
            beton host disable
          </Prompt>
          <L>User-Dienst entfernt. Der Host läuft bis zum Abmelden weiter; beton host stop beendet ihn sofort.</L>
        </F>
      )}
      {state === 'login' && (
        <F id="CLI-006">
          <Comment>Nur für einen zentralen Server nötig – lokal gibt es keinen Login</Comment>
          <Prompt>beton login https://beton.team.example --device-code</Prompt>
          <div className="chamfer my-1 border-l-2 border-signal bg-signal-soft px-3 py-1.5">
            <div>
              Öffne <S tone="bold">https://beton.team.example/device</S> und gib diesen Code ein:{' '}
              <span className="bg-signal px-1 font-semibold text-signal-foreground">WDJB-MJHT</span>
            </div>
            <div className="text-muted-foreground">
              Warte auf Bestätigung … <Cursor />
            </div>
          </div>
          <L tone="ok">✓ Angemeldet als ingo@diva-e.com · Profil „team“ angelegt · Token im macOS-Schlüsselbund</L>
          <Prompt>beton profile list</Prompt>
          <L tone="dim">{'  NAME   SERVER                           BENUTZER'}</L>
          <L>{'* local  127.0.0.1:7420 (dieser Rechner)   ingo'}</L>
          <L>{'  team   https://beton.team.example       ingo@diva-e.com'}</L>
          <Prompt>beton --server team session list --json | jq length</Prompt>
          <L>12</L>
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── setup, doctor, diagnose ───────────────────────── */

function Check({ s, id, msg, hint }: { s: 'ok' | 'warn' | 'fail'; id: string; msg: string; hint?: string }) {
  return (
    <>
      <L>
        {'  '}
        {s === 'ok' && <S tone="ok">✓</S>}
        {s === 'warn' && <S tone="bold">△</S>}
        {s === 'fail' && <S tone="deny">✗</S>} {id.padEnd(30)}
        <S tone={s === 'fail' ? 'deny' : 'plain'}>{msg}</S>
      </L>
      {hint && <L tone="dim">{'    '.padEnd(33)}→ {hint}</L>}
    </>
  )
}

function CliSetup({ state }: { state: string }) {
  return (
    <Term>
      {state === 'setup' && (
        <F id={['CLI-005', 'DIST-015']}>
          <Prompt>beton setup</Prompt>
          <L>
            <S tone="bold">beton einrichten</S> · alles läuft lokal auf diesem Rechner
          </L>
          <Blank />
          <L tone="dim">Harness-CLIs</L>
          <L>
            {'  '}
            <S tone="ok">✓</S> <S tone="claude">claude 2.3.1  </S> ~/.local/bin/claude        angemeldet: Claude Max (claude auth status)
          </L>
          <L>
            {'  '}
            <S tone="deny">✗</S> <S tone="codex">codex        </S> nicht gefunden
          </L>
          <L>
            {'  '}
            <S tone="ok">✓</S> <S tone="acp">gemini 0.14.2</S> /opt/homebrew/bin/gemini   angemeldet: Google-Konto
          </L>
          <L>
            {'  '}
            <S tone="ok">✓</S> <S tone="direct">ollama 0.9.1 </S> 127.0.0.1:11434            lokal, kein Konto nötig
          </L>
          <Blank />
          <L>Du brauchst keinen API-Schlüssel: beton nutzt die Anmeldung der offiziellen CLIs.</L>
          <Blank />
          <Ask title="Codex CLI installieren?" question="Jetzt ausführen?">
            <L>{'  '}Offizielle Installation des Herstellers, genau dieser Befehl wird ausgeführt:</L>
            <L tone="bold">{'    '}npm install -g @openai/codex</L>
            <L tone="dim">{'  '}Ohne sudo, in dein npm-Präfix (~/.npm-global). Danach: codex login (Browser, ChatGPT-Abo genügt).</L>
          </Ask>
        </F>
      )}
      {state === 'noninteractive' && (
        <F id={['CLI-005', 'DIST-015']}>
          <Prompt cwd="~/ci">beton setup --non-interactive</Prompt>
          <L>
            {'  '}
            <S tone="ok">✓</S> claude 2.3.1 angemeldet
          </L>
          <L>
            {'  '}
            <S tone="deny">✗</S> codex nicht gefunden – installiere nur mit --install-clis oder interaktiv
          </L>
          <L tone="dim">Nichts installiert.</L>
          <ExitCode code={1} cwd="~/ci" />
        </F>
      )}
      {state === 'doctor' && (
        <F id="CLI-005">
          <Prompt>beton doctor</Prompt>
          <L>
            <S tone="bold">beton 1.3.0</S> · Kanal stable · Updates nur auf Anfrage
          </L>
          <Check s="ok" id="daemon" msg="läuft (pid 48211) auf 127.0.0.1:7420" />
          <Check s="ok" id="config" msg="~/.beton/config.yaml und .beton/config.yaml gültig" />
          <Check s="ok" id="permissions" msg="~/.beton 0700 · Token-Datei 0600" />
          <Check s="ok" id="store.sqlite" msg="quick_check ok (412 MB)" />
          <Check s="ok" id="harness.claude" msg="2.3.1 im unterstützten Bereich (≥ 2.1, < 2.5) · angemeldet" />
          <Check s="warn" id="harness.codex" msg="nicht gefunden" hint="beton setup bietet die Installation an" />
          <Check s="ok" id="harness.gemini" msg="0.14.2 · angemeldet" />
          <Check s="ok" id="sandbox.macos" msg="sandbox-exec verfügbar" />
          <Check s="ok" id="proxy.ca" msg="Egress-Proxy-CA im Schlüsselbund" />
          <Check s="ok" id="runtime.docker" msg="Podman 5.2 (Docker-API, rootless)" />
          <Check s="warn" id="plugin.acme-git-gitea" msg="unsigniert (aus Git installiert)" />
          <Check s="fail" id="plugin.beton-runner-hetzner" msg="crashlooping: 5 Neustarts in 10 Min." hint="beton plugin doctor beton-runner-hetzner" />
          <Check s="ok" id="telemetry" msg="aus" />
          <L>13 Prüfungen · 10 ok · 2 Hinweise · 1 Fehler</L>
          <ExitCode code={2} />
        </F>
      )}
      {state === 'json' && (
        <F id={['CLI-005', 'CLI-001']}>
          <Prompt>beton doctor --json | jq '.checks[] | select(.status != "ok")'</Prompt>
          {[
            ['harness.codex', 'warn', 'nicht gefunden', 'beton setup bietet die Installation an'],
            ['plugin.acme-git-gitea', 'warn', 'unsigniert (aus Git installiert)', 'Signiertes Release installieren oder Warnung akzeptieren'],
            ['plugin.beton-runner-hetzner', 'fail', 'crashlooping: 5 Neustarts in 10 Min.', 'beton plugin doctor beton-runner-hetzner'],
          ].map(([id, st, msg, hint]) => (
            <div key={id}>
              <L>{'{'}</L>
              <L>
                {'  '}
                <S tone="codex">"id"</S>: <S tone="ok">"{id}"</S>,
              </L>
              <L>
                {'  '}
                <S tone="codex">"status"</S>: <S tone={st === 'fail' ? 'deny' : 'ok'}>"{st}"</S>,
              </L>
              <L>
                {'  '}
                <S tone="codex">"message"</S>: <S tone="ok">"{msg}"</S>,
              </L>
              <L>
                {'  '}
                <S tone="codex">"hint"</S>: <S tone="ok">"{hint}"</S>
              </L>
              <L>{'}'}</L>
            </div>
          ))}
        </F>
      )}
      {state === 'diagnose' && (
        <F id="CLI-005">
          <Prompt>beton diagnose -o beton-diagnose.zip --anonymize</Prompt>
          <L tone="dim">Sammle Version, Konfiguration ohne Secrets, doctor --json, Logs der letzten 24 Std., System-Infos …</L>
          <L>
            {'  '}Ausgelassen: Tokens, Secrets, Credential-Volumes, Session-Inhalte <S tone="dim">(mit --include-session ID einzeln dazunehmen)</S>
          </L>
          <L>{'  '}Anonymisiert: Benutzer- und Hostnamen, Pfade unter ~</L>
          <L tone="ok">✓ beton-diagnose.zip (2,4 MB)</L>
          <L>Die Datei wird nirgendwo hochgeladen. Hänge sie selbst an dein Issue, wenn du magst.</L>
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── config ───────────────────────── */

function CliConfig({ state }: { state: string }) {
  const rows: [string, string, string, string?][] = [
    ['update.channel', 'stable', 'default'],
    ['update.auto_check', 'false', 'default'],
    ['harness.default', 'claude', 'user', '~/.beton/config.yaml'],
    ['permission_mode', 'ask-on-write', 'project', '.beton/config.yaml'],
    ['sandbox.level', '2', 'project', '.beton/config.yaml'],
    ['server.bind', '127.0.0.1:7420', 'default'],
    ['telemetry.enabled', 'false', 'default'],
    ['voice.model', 'whisper-small-de', 'user', '~/.beton/config.yaml'],
    ['log.level', 'debug', 'env', 'BETON_LOG_LEVEL'],
  ]
  return (
    <Term>
      <F id="CLI-008">
        {state === 'list' && (
          <>
            <Prompt>beton config list</Prompt>
            <L tone="dim">{'KEY                   WERT                QUELLE'}</L>
            {rows.map(([k, v, src, file]) => (
              <L key={k}>
                {k.padEnd(22)}
                {v.padEnd(20)}
                <S tone={src === 'default' ? 'dim' : 'bold'}>{src.padEnd(9)}</S>
                <S tone="dim">{file ?? ''}</S>
              </L>
            ))}
          </>
        )}
        {state === 'invalid' && (
          <>
            <Prompt>beton config set update.channel weekly --global</Prompt>
            <L tone="deny">Fehler: update.channel: „weekly“ ist nicht erlaubt. Erlaubt: stable, beta, nightly</L>
            <L tone="dim">{'  '}Geprüft gegen das veröffentlichte Schema (config.schema.json). ~/.beton/config.yaml ist unverändert.</L>
            <ExitCode code={1} />
          </>
        )}
        {state === 'set' && (
          <>
            <Prompt>beton config set permission_mode ask-always --project</Prompt>
            <L>.beton/config.yaml: permission_mode = ask-always (vorher ask-on-write)</L>
            <Prompt>beton config get permission_mode</Prompt>
            <L>
              ask-always <S tone="dim">(project · .beton/config.yaml)</S>
            </L>
            <Prompt>beton config edit --global</Prompt>
            <L tone="dim">Öffne ~/.beton/config.yaml in $EDITOR (nvim) … gespeichert, Schema ok.</L>
          </>
        )}
      </F>
    </Term>
  )
}

/* ───────────────────────── import & export ───────────────────────── */

function CliImport({ state }: { state: string }) {
  return (
    <Term>
      <F id="CLI-007">
        {state === 'pick' && (
          <>
            <Prompt>beton import</Prompt>
            <L tone="dim">Gefundene Chats anderer Werkzeuge (nur lokal gelesen, Originale bleiben unverändert):</L>
            <div className="chamfer my-1 border-l-2 border-signal bg-signal-soft px-3 py-1.5">
              {[
                ['x', 'Claude Code', 'Payment-Webhook debuggen', 'shop-frontend', 'vor 2 Tg.', '84'],
                [' ', 'Claude Code', 'Docker-Build beschleunigen', 'infra', 'vor 5 Tg.', '31'],
                ['x', 'Codex', 'Migration auf Vite 8', 'shop-frontend', 'vor 1 Wo.', '57'],
                [' ', 'Gemini CLI', 'Terraform-Module aufräumen', 'infra', 'vor 2 Wo.', '19'],
              ].map(([x, h, title, proj, when, n], i) => (
                <div key={i}>
                  {i === 0 ? <S tone="signal">› </S> : '  '}[{x === 'x' ? <S tone="bold">x</S> : ' '}] <H name={h} pad={12} />
                  {title.padEnd(30)}
                  <S tone="dim">
                    {proj.padEnd(15)}
                    {when.padEnd(11)}
                    {n} Nachrichten
                  </S>
                </div>
              ))}
              <div className="mt-1">
                Leertaste wählt · <span className="bg-signal px-1 font-semibold text-signal-foreground">Enter</span> importiert 2 Chats · Esc bricht ab{' '}
                <Cursor />
              </div>
            </div>
          </>
        )}
        {state === 'last' && (
          <>
            <Prompt>beton import --harness claude --last 5</Prompt>
            {[
              ['ses_9d01', 'Payment-Webhook debuggen'],
              ['ses_9d02', 'Docker-Build beschleunigen'],
              ['ses_9d03', 'Cookie-Banner entfernen'],
              ['ses_9d04', 'i18n-Schlüssel aufräumen'],
              ['ses_9d05', 'GraphQL-Fehler im Warenkorb'],
            ].map(([id, t]) => (
              <L key={id}>
                <S tone="ok">✓</S> {id}  {t}
              </L>
            ))}
            <L tone="dim">5 Sessions importiert aus ~/.claude/projects. Fortsetzen z. B. mit: beton resume ses_9d01</L>
          </>
        )}
        {state === 'export' && (
          <>
            <Prompt>beton export ses_7f3k -o rate-limiter.jsonl --with-blobs</Prompt>
            <L>
              <S tone="ok">✓</S> rate-limiter.jsonl · 1.284 Events · 3 Anhänge · 2,1 MB
            </L>
            <Prompt>beton import rate-limiter.jsonl</Prompt>
            <L>
              <S tone="ok">✓</S> ses_9d06  Rate-Limiter für die Login-API <S tone="dim">(inhaltsgleich, neue ID)</S>
            </L>
          </>
        )}
      </F>
    </Term>
  )
}

/* ───────────────────────── agent ───────────────────────── */

function CliAgent({ state }: { state: string }) {
  return (
    <Term>
      <F id="CLI-009">
        {state === 'list' && (
          <>
            <Prompt>beton agent list</Prompt>
            <L tone="dim">{'NAME               QUELLE      HARNESS        BESCHREIBUNG'}</L>
            {[
              ['builtin:reviewer', 'eingebaut', 'Codex', 'Reviewt Diffs gegen Projektkonventionen'],
              ['builtin:triage', 'eingebaut', 'Claude Code', 'Sortiert Issues und schlägt Labels vor'],
              ['release-notes', 'User', 'Claude Code', '~/.beton/agents/release-notes'],
              ['shop-a11y', 'Projekt', 'Gemini CLI', '.beton/agents/shop-a11y'],
            ].map(([n, src, h, d]) => (
              <L key={n}>
                {n.padEnd(19)}
                <S tone="dim">{src.padEnd(12)}</S>
                <H name={h} pad={15} />
                {d}
              </L>
            ))}
          </>
        )}
        {state === 'validate' && (
          <>
            <Prompt>beton agent validate ./agents/reviewer</Prompt>
            <L>
              <S tone="deny">✗</S> agent.yaml:7:13  <S tone="bold">executor.harness</S>: „claude-code“ ist unbekannt. Meintest du „claude“?
            </L>
            <L>
              <S tone="deny">✗</S> agent.yaml:15:5  <S tone="bold">policies[1]</S>: policies/strict.yaml existiert nicht
            </L>
            <L>2 Fehler · Schema: beton agent schema</L>
            <ExitCode code={1} />
          </>
        )}
        {state === 'new' && (
          <>
            <Prompt>beton agent new reviewer</Prompt>
            <L>Angelegt: reviewer/agent.yaml, reviewer/prompt.md, reviewer/policies/</L>
            <Prompt>beton agent validate ./reviewer</Prompt>
            <L>
              <S tone="ok">✓</S> reviewer ist gültig
            </L>
            <Prompt>beton run ./reviewer -p "Review den letzten Commit"</Prompt>
            <L tone="dim">Session ses_9e11 · Agent reviewer · Claude Code · claude-sonnet-5-5</L>
          </>
        )}
      </F>
    </Term>
  )
}

/* ───────────────────────── usage ───────────────────────── */

function CliUsage({ state }: { state: string }) {
  return (
    <Term>
      <F id="CLI-010">
        {state === 'table' && (
          <>
            <Prompt>beton usage --by model --since 2026-10-01</Prompt>
            <L tone="dim">Zeitraum 01.10.–03.10.2026 · lokal gezählt</L>
            <Blank />
            <L tone="bold">Subscriptions · Nutzung des Abo-Fensters, kein Euro-Betrag</L>
            <L tone="dim">{'MODELL            HARNESS        SESSIONS  TOKENS EIN   TOKENS AUS  FENSTER'}</L>
            <L>
              {'claude-opus-5-5   '}
              <H name="Claude Code" pad={15} />
              {'14        2,41 Mio.    198.400     '}
              <Bar pct={38} width={10} /> 38 % <S tone="dim">· neu in 2 Std. 14 Min.</S>
            </L>
            <L>
              {'gpt-5.3-codex     '}
              <H name="Codex" pad={15} />
              {' 6        812.300      61.020      '}
              <Bar pct={12} width={10} /> 12 %
            </L>
            <L>
              {'gemini-3-pro      '}
              <H name="Gemini CLI" pad={15} />
              {' 2        94.100       8.210       '}
              <S tone="dim">keine Angabe der CLI</S>
            </L>
            <Blank />
            <L tone="bold">Nach Verbrauch · API-Schlüssel oder lokal</L>
            <L tone="dim">{'MODELL            HARNESS        SESSIONS  TOKENS EIN   TOKENS AUS  KOSTEN'}</L>
            <L>
              {'qwen3-coder:30b   '}
              <H name="Ollama (lokal)" pad={15} />
              {' 3        120.400      18.300      0,00 $'}
            </L>
            <Blank />
            <L tone="dim">Subscription- und API-Nutzung werden nie zusammengerechnet.</L>
          </>
        )}
        {state === 'json' && (
          <>
            <Prompt>beton usage --by model --since 2026-10-01 --json | jq '.groups[0]'</Prompt>
            <L>{'{'}</L>
            <L>
              {'  '}
              <S tone="codex">"model"</S>: <S tone="ok">"claude-opus-5-5"</S>, <S tone="codex">"harness"</S>: <S tone="ok">"claude"</S>,
            </L>
            <L>
              {'  '}
              <S tone="codex">"billing"</S>: <S tone="ok">"subscription"</S>, <S tone="codex">"sessions"</S>: 14,
            </L>
            <L>
              {'  '}
              <S tone="codex">"input_tokens"</S>: 2410388, <S tone="codex">"output_tokens"</S>: 198400,
            </L>
            <L>
              {'  '}
              <S tone="codex">"cost_eur"</S>: <S tone="dim">null</S>, <S tone="codex">"window_used_pct"</S>: 38
            </L>
            <L>{'}'}</L>
          </>
        )}
      </F>
    </Term>
  )
}

/* ───────────────────────── schedule ───────────────────────── */

function CliSchedule({ state }: { state: string }) {
  return (
    <Term>
      <F id="CLI-011">
        {state === 'list' && (
          <>
            <Prompt>beton schedule list</Prompt>
            <L tone="dim">{'NAME             CRON          AGENT            NÄCHSTER LAUF       RUNNER      STATUS'}</L>
            <L>{'nightly-deps     0 3 * * *     dep-updater      heute 03:00         os=linux    '}<S tone="ok">aktiv</S></L>
            <L>{'triage-morgens   0 7 * * 1-5   builtin:triage   Mo. 06.10. 07:00    lokal       '}<S tone="ok">aktiv</S></L>
            <L>{'weekly-report    0 16 * * 5    release-notes    –                   lokal       '}<S tone="dim">pausiert</S></L>
            <Blank />
            <Prompt>beton run-history nightly-deps</Prompt>
            <L tone="dim">{'LAUF       START              DAUER     ERGEBNIS          SESSION'}</L>
            <L>{'run_4410   heute 03:00        6 Min.    '}<S tone="ok">fertig, PR #412</S>{'   ses_6n1c'}</L>
            <L>{'run_4398   gestern 03:00      4 Min.    '}<S tone="ok">fertig, nichts</S>{'    ses_6l2b'}</L>
            <L>{'run_4371   Mi. 01.10. 03:00   12 Min.   '}<S tone="deny">abgelehnt (Policy)</S>{' ses_6j7x'}</L>
          </>
        )}
        {state === 'create' && (
          <>
            <Prompt>beton schedule create triage-morgens --cron "0 7 * * 1-5" --agent triage --prompt "Sortiere neue Issues" --catch-up skip</Prompt>
            <L>
              <S tone="ok">✓</S> triage-morgens angelegt · nächster Lauf Mo. 06.10.2026 07:00 (Europe/Berlin)
            </L>
            <L tone="dim">{'  '}Läuft auf diesem Rechner. Ist er zur Laufzeit aus, wird der Lauf übersprungen (catch_up: skip).</L>
          </>
        )}
        {state === 'run-now' && (
          <>
            <Prompt>beton schedule run-now nightly-deps --json</Prompt>
            <L>
              {'{'}
              <S tone="codex">"run_id"</S>:<S tone="ok">"run_4411"</S>,<S tone="codex">"session_id"</S>:<S tone="ok">"ses_9f20"</S>
              {'}'}
            </L>
            <Prompt>beton schedule pause weekly-report</Prompt>
            <L>weekly-report pausiert. Fortsetzen mit beton schedule resume weekly-report</L>
          </>
        )}
      </F>
    </Term>
  )
}

/* ───────────────────────── plugin ───────────────────────── */

function CliPlugin({ state }: { state: string }) {
  return (
    <Term>
      {state === 'install' && (
        <F id={['CLI-012', 'PLG-005', 'PLG-008']}>
          <Prompt cwd="~/code">beton plugin install ./beton-runner-hetzner --allow-unsigned</Prompt>
          <L>{'Quelle     lokaler Pfad ~/code/beton-runner-hetzner (kein Download)'}</L>
          <L>
            {'Plugin     beton-runner-hetzner 0.3.1 · runner_provider · plugin_api 1 · beton >=1.0, <2.0 '}
            <S tone="ok">✓</S>
          </L>
          <L>
            {'Prüfsumme  sha256 4f1c9a…e07b stimmt mit dem Manifest überein '}
            <S tone="ok">✓</S>
          </L>
          <L>
            {'Signatur   '}
            <S tone="bold">△ keine</S> – wird dauerhaft als „unsigniert“ markiert
          </L>
          <Ask title="Das Plugin verlangt diese Berechtigungen" question="Installieren und Berechtigungen gewähren?">
            <L>{'  network   api.hetzner.cloud:443        nur über den Egress-Proxy'}</L>
            <L>{'  fs_read   ~/.config/hcloud'}</L>
            <L>{'  secrets   hetzner/api-token            nur als Platzhalter bt_cred_…'}</L>
            <L>{'  exec      ssh'}</L>
            <L tone="dim">{'  fs_write  –      env  –'}</L>
          </Ask>
        </F>
      )}
      {state === 'update' && (
        <F id={['CLI-012', 'PLG-008', 'PLG-010']}>
          <Prompt>beton plugin update beton-runner-hetzner</Prompt>
          <L>
            0.3.1 → <S tone="bold">0.4.0</S> (höchste mit beton 1.3 und plugin_api 1 kompatible Version)
          </L>
          <L>
            Signatur <S tone="ok">✓</S> github.com/acme/beton-runner-hetzner/.github/workflows/release.yml@refs/tags/v0.4.0
          </L>
          <Ask title="Das Update erweitert die Berechtigungen" question="Update installieren?">
            <L>{'  network   api.hetzner.cloud:443'}</L>
            <L tone="ok">{'+ network   objects.hetzner.cloud:443     neu'}</L>
            <L tone="ok">{'+ fs_write  ~/.cache/beton-hetzner        neu'}</L>
            <L tone="dim">{'  unverändert: fs_read, secrets, exec'}</L>
          </Ask>
        </F>
      )}
      {state === 'list' && (
        <F id={['CLI-012', 'CLI-001']}>
          <Prompt>beton plugin list --json | jq -c '.[]'</Prompt>
          {[
            ['beton-runner-hetzner', '0.3.1', 'runner_provider', 'crashlooping'],
            ['acme-git-gitea', '0.2.0', 'git_provider', 'unsigned'],
            ['echo-harness', '0.1.0-dev', 'harness', 'ok'],
            ['beton-runner-proxmox', '0.9.2', 'runner_provider', 'incompatible'],
          ].map(([n, v, k, st]) => (
            <L key={n}>
              {'{'}
              <S tone="codex">"name"</S>:<S tone="ok">"{n}"</S>,<S tone="codex">"version"</S>:<S tone="ok">"{v}"</S>,<S tone="codex">"kind"</S>:
              <S tone="ok">"{k}"</S>,<S tone="codex">"status"</S>:<S tone={st === 'ok' ? 'ok' : st === 'unsigned' ? 'bold' : 'deny'}>"{st}"</S>
              {'}'}
            </L>
          ))}
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── upgrade & uninstall ───────────────────────── */

function CliUpgrade({ state }: { state: string }) {
  return (
    <Term>
      {state === 'check' && (
        <F id={['CLI-013', 'DIST-016', 'DIST-017']}>
          <Comment>beton sucht nur nach Updates, wenn du es aufrufst</Comment>
          <Prompt>beton upgrade --check</Prompt>
          <L>Installiert   1.3.0 (stable) · Installer-Skript (~/.beton/bin/beton)</L>
          <L>
            Neueste      <S tone="bold">1.4.0</S> (stable, 30.09.2026) · Änderungen: beton upgrade --check --changelog
          </L>
          <Prompt>beton upgrade --check --json</Prompt>
          <L>
            {'{'}
            <S tone="codex">"installed"</S>:<S tone="ok">"1.3.0"</S>,<S tone="codex">"latest"</S>:<S tone="ok">"1.4.0"</S>,<S tone="codex">"channel"</S>:
            <S tone="ok">"stable"</S>,<S tone="codex">"install_method"</S>:<S tone="ok">"script"</S>,<S tone="codex">"update_available"</S>:true{'}'}
          </L>
          <Prompt>beton config set update.channel nightly && beton upgrade --check</Prompt>
          <L>
            Neueste      <S tone="bold">1.5.0-nightly.20261003</S> (nightly)
          </L>
        </F>
      )}
      {state === 'script' && (
        <F id={['CLI-013', 'DIST-016']}>
          <Prompt>beton upgrade</Prompt>
          <L tone="dim">Lade beton-1.4.0-aarch64-apple-darwin.tar.gz (18,2 MB) von github.com/ifahrentholz/beton …</L>
          <L>
            {'  '}
            <S tone="ok">✓</S> SHA-256 stimmt mit SHA256SUMS überein
          </L>
          <L>
            {'  '}
            <S tone="ok">✓</S> Sigstore-Signatur: …/.github/workflows/release.yml@refs/tags/v1.4.0
          </L>
          <L>
            {'  '}
            <S tone="ok">✓</S> ~/.beton/bin/beton ersetzt · vorherige Version als beton.old
          </L>
          <L>Daemon neu gestartet (keine laufenden Turns).</L>
          <L tone="ok">beton 1.4.0</L>
        </F>
      )}
      {state === 'file' && (
        <F id={['CLI-013', 'DIST-016']}>
          <Comment>Ohne Netz: Archiv und .sigstore.json vorher auf einem anderen Rechner geladen</Comment>
          <Prompt>beton upgrade --file ~/Downloads/beton-1.4.0-aarch64-apple-darwin.tar.gz</Prompt>
          <L>
            {'  '}
            <S tone="ok">✓</S> SHA-256 stimmt mit SHA256SUMS (daneben) überein
          </L>
          <L>
            {'  '}
            <S tone="ok">✓</S> Sigstore-Bundle beton-1.4.0-aarch64-apple-darwin.tar.gz.sigstore.json gültig (offline geprüft)
          </L>
          <L>
            {'  '}
            <S tone="ok">✓</S> ersetzt · vorherige Version als beton.old
          </L>
          <L tone="ok">beton 1.4.0</L>
        </F>
      )}
      {state === 'brew' && (
        <F id={['CLI-013', 'DIST-016']}>
          <Prompt>beton upgrade</Prompt>
          <L>Installationsart: Homebrew (/opt/homebrew/Cellar/beton/1.3.0)</L>
          <L>beton aktualisiert sich hier nicht selbst, damit Homebrew den Überblick behält. Führe aus:</L>
          <L tone="bold">{'  '}brew upgrade beton</L>
          <ExitCode code={0} />
        </F>
      )}
      {state === 'busy' && (
        <F id={['CLI-013', 'DIST-016']}>
          <Prompt>beton upgrade</Prompt>
          <L>
            {'  '}
            <S tone="ok">✓</S> beton 1.4.0 geladen und geprüft
          </L>
          <L>
            2 Turns laufen (<S tone="codex">ses_7f3m</S>, <S tone="claude">ses_6n1c</S>). Der Daemon startet neu, sobald beide fertig sind.
          </L>
          <L tone="dim">
            Warte auf Leerlauf … <Cursor />
          </L>
          <L tone="dim">Ctrl+C lässt 1.3.0 weiterlaufen · --force startet sofort neu und unterbricht die Turns</L>
        </F>
      )}
      {state === 'uninstall' && (
        <F id={['CLI-013', 'DIST-020']}>
          <Prompt>beton uninstall --purge</Prompt>
          <L>Wird entfernt:</L>
          <L>{'  '}Daemon und Host stoppen · LaunchAgent dev.beton.daemon</L>
          <L>{'  '}~/.beton/bin/beton und der PATH-Eintrag in ~/.zshrc</L>
          <L>Zusätzlich wegen --purge:</L>
          <L>{'  '}~/.beton – Daten 412 MB, Logs, Whisper-Modelle 466 MB, 3 Plugins</L>
          <L>{'  '}2 beton-Einträge im Schlüsselbund und die Egress-Proxy-CA</L>
          <L tone="dim">Bleibt: die Anmeldungen von claude, codex und gemini.</L>
          <Ask title="Alle Sessions dieses Rechners werden gelöscht" question="Endgültig löschen?">
            <L>{'  '}Das lässt sich nicht rückgängig machen. Sichern vorher: beton export &lt;id&gt; -o datei.jsonl</L>
          </Ask>
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── Shell-Completion ───────────────────────── */

function CliCompletion({ state }: { state: string }) {
  return (
    <Term>
      <F id="CLI-014">
        <Prompt>{'beton completion zsh > "${fpath[1]}/_beton" && exec zsh'}</Prompt>
        {state === 'zsh' && (
          <>
            <Prompt>beton attach </Prompt>
            {sessionRows.slice(0, 6).map(([id, title, , st]) => (
              <L key={id}>
                <S tone="bold">{id}</S>
                <S tone="dim">{'  -- '}</S>
                {title} <S tone="dim">({st})</S>
              </L>
            ))}
            <Blank />
            <Prompt>beton --server </Prompt>
            <L>
              <S tone="bold">local</S>
              <S tone="dim">{'  -- 127.0.0.1:7420 (dieser Rechner)'}</S>
            </L>
            <L>
              <S tone="bold">team </S>
              <S tone="dim">{'  -- https://beton.team.example'}</S>
            </L>
          </>
        )}
        {state === 'offline' && (
          <>
            <Comment>Daemon gestoppt: Vervollständigung bricht nach höchstens 300 ms leer ab, die Shell hängt nicht</Comment>
            <Prompt>
              beton attach <Cursor />
            </Prompt>
            <Prompt>beton att</Prompt>
            <L>
              <S tone="bold">attach</S> <S tone="dim">-- Mit einer laufenden Session verbinden</S>
            </L>
          </>
        )}
      </F>
    </Term>
  )
}

/* ───────────────────────── --help & Konventionen ───────────────────────── */

const helpTree: [string, string][] = [
  ['run', 'Session starten und im Terminal mitlesen (-p für Skripte)'],
  ['resume', 'Gestoppte Session neu starten'],
  ['attach', 'Mit laufender Session verbinden (--read-only)'],
  ['open', 'Session im Browser öffnen (Einmal-Link)'],
  ['session', 'Sessions auflisten, umbenennen, forken, archivieren …'],
  ['serve', 'Server starten (lokal: nur 127.0.0.1)'],
  ['host', 'Diesen Rechner als Host anmelden, pair, enable …'],
  ['hosts, runners', 'Hosts und Runner anzeigen, Logs, stoppen'],
  ['runner login', 'Vendor-CLI im Container anmelden (Device-Flow)'],
  ['setup, doctor', 'Einrichten und Umgebung prüfen'],
  ['diagnose', 'Support-Bundle ohne Secrets erzeugen'],
  ['login, profile', 'An zentralem Server anmelden, Profile'],
  ['import, export', 'Chats anderer Werkzeuge übernehmen, JSONL'],
  ['config', 'Konfiguration lesen und schreiben'],
  ['usage', 'Verbrauch nach Tag, Session, Modell …'],
  ['agent', 'Agent-Definitionen auflisten, prüfen, anlegen'],
  ['mcp, policy, sandbox', 'MCP-Server, Policies, Sandbox'],
  ['secrets, audit, proxy', 'Secrets, Audit-Log, Egress-Proxy'],
  ['schedule, run-history', 'Zeitpläne und ihre Läufe'],
  ['plugin', 'Plugins installieren, prüfen, entwickeln'],
  ['voice, telemetry', 'Sprachmodelle, Telemetrie (aus)'],
  ['upgrade, uninstall', 'Aktualisieren (nur auf Aufruf), entfernen'],
  ['tui', 'Vollbild-Oberfläche im Terminal'],
  ['completion', 'Shell-Vervollständigung erzeugen'],
]

const exitCodes: [string, string][] = [
  ['0', 'ok'],
  ['1', 'allgemeiner Fehler'],
  ['2', 'falscher Aufruf (unbekanntes Flag, mehrdeutige ID)'],
  ['3', 'von Policy verboten'],
  ['4', 'Freigabe abgelehnt oder Zeit abgelaufen'],
  ['5', 'Budget erschöpft'],
  ['6', 'Harness-Fehler'],
  ['7', 'Server nicht erreichbar'],
  ['130', 'unterbrochen (Ctrl+C)'],
]

function CliHelp({ state }: { state: string }) {
  return (
    <Term>
      {state === 'help' && (
        <F id="CLI-001">
          <Prompt>beton --help</Prompt>
          <L>
            <S tone="bold">beton</S> – Meta-Harness für Coding-Agents. Läuft lokal; nutzt die Anmeldung von claude, codex & Co.
          </L>
          <Blank />
          <L>
            <S tone="bold">Aufruf:</S> beton [OPTIONEN] &lt;BEFEHL&gt;
          </L>
          <Blank />
          <L tone="bold">Befehle:</L>
          {helpTree.map(([c, d]) => (
            <L key={c}>
              {'  '}
              <S tone="bold">{c.padEnd(24)}</S>
              {d}
            </L>
          ))}
          <Blank />
          <L tone="bold">Globale Optionen:</L>
          <L>{'  --server PROFIL|URL     Ziel-Server (Standard: local)'}</L>
          <L>{'  --json                  Maschinenlesbare Ausgabe auf stdout'}</L>
          <L>{'  -q, --quiet / -v        Weniger / mehr Diagnose auf stderr'}</L>
          <L>{'  --no-color              Keine Farben (automatisch, wenn kein Terminal)'}</L>
          <Blank />
          <L tone="dim">Mehr: beton &lt;BEFEHL&gt; --help · man beton</L>
        </F>
      )}
      {state === 'conventions' && (
        <F id="CLI-001">
          <Comment>stdout enthält nur Nutzdaten – kein Spinner, kein Banner</Comment>
          <Prompt>beton session list --json | jq -r '.[] | select(.status=="waiting") | .id'</Prompt>
          <L>ses_7f3k</L>
          <Blank />
          <Comment>Fortschritt geht auf stderr und lässt sich getrennt umleiten</Comment>
          <Prompt>beton export ses_7f3k 2&gt;/dev/null | wc -l</Prompt>
          <L>1284</L>
          <Blank />
          <Comment>Unbekanntes Flag: Hilfe auf stderr, Exit-Code 2</Comment>
          <Prompt>beton run claude --modle opus</Prompt>
          <L tone="deny">Fehler: unbekanntes Argument „--modle“</L>
          <L>{'  '}Meintest du „--model“?</L>
          <L tone="dim">{'  '}Aufruf: beton run [OPTIONEN] [TARGET] · Hilfe: beton run --help</L>
          <ExitCode code={2} />
          <Blank />
          <Comment>Keine Farben ohne Terminal</Comment>
          <Prompt>beton doctor | cat</Prompt>
          <L>ok    daemon        läuft (pid 48211) auf 127.0.0.1:7420</L>
          <L>warn  harness.codex nicht gefunden</L>
        </F>
      )}
      {state === 'exit' && (
        <F id="CLI-001">
          <Prompt>man beton | sed -n '/^EXIT-CODES/,/^$/p'</Prompt>
          <L tone="bold">EXIT-CODES</L>
          {exitCodes.map(([c, d]) => (
            <L key={c}>
              {'       '}
              <S tone={c === '0' ? 'ok' : 'deny'}>{c.padEnd(5)}</S>
              {d}
            </L>
          ))}
          <L>{'       Einzelne Befehle dokumentieren eigene Codes, z. B. doctor: 0 ok · 1 Hinweise · 2 Fehler.'}</L>
        </F>
      )}
    </Term>
  )
}

/* ───────────────────────── SSE ───────────────────────── */

function CliSse({ state }: { state: string }) {
  return (
    <Term>
      <F id="API-003">
        {state === 'stream' && (
          <>
            <Prompt>curl -N -H "Authorization: Bearer $(cat ~/.beton/token)" \</Prompt>
            <L>{'    "http://127.0.0.1:7420/v1/sessions/ses_7f3m/events/stream?from_seq=0"'}</L>
            {[
              ['1', 'session.created', '{"harness":"codex","title":"Review: Rate-Limiter"}'],
              ['2', 'message.user', '{"text":"Bitte reviewe den Rate-Limiter …"}'],
              ['3', 'tool.started', '{"tool":"shell","command":"git diff main...beton/rate-limiter-7f3k"}'],
              ['…', '', ''],
              ['142', 'message.delta', '{"text":"Der Token-Bucket wird nie aufgeräumt"}'],
            ].map(([id, ev, data], i) =>
              id === '…' ? (
                <L key={i} tone="dim">
                  …
                </L>
              ) : (
                <div key={i}>
                  <L>
                    <S tone="codex">id:</S> {id}
                  </L>
                  <L>
                    <S tone="codex">event:</S> {ev}
                  </L>
                  <L>
                    <S tone="codex">data:</S> {data}
                  </L>
                  <Blank />
                </div>
              ),
            )}
            <L>
              <Cursor />
            </L>
          </>
        )}
        {state === 'resume' && (
          <>
            <Comment>Verbindung weg? Mit der letzten id lückenlos weiter</Comment>
            <Prompt>curl -N -H "Authorization: Bearer $(cat ~/.beton/token)" -H "Last-Event-ID: 42" \</Prompt>
            <L>{'    "http://127.0.0.1:7420/v1/sessions/ses_7f3m/events/stream"'}</L>
            <L>
              <S tone="codex">id:</S> 43
            </L>
            <L>
              <S tone="codex">event:</S> tool.completed
            </L>
            <L>
              <S tone="codex">data:</S> {'{"exit_code":0,"duration_ms":412}'}
            </L>
            <Blank />
            <L>
              <S tone="codex">id:</S> 44
            </L>
            <L>
              <S tone="codex">event:</S> message.delta
            </L>
            <L>
              <S tone="codex">data:</S> {'{"text":"Die Tests decken nur den Happy Path ab."}'}
            </L>
            <Blank />
            <Cursor />
          </>
        )}
      </F>
    </Term>
  )
}

/* ───────────────────────── Entwickler-Ansicht (QA) ───────────────────────── */

function CliDev({ state }: { state: string }) {
  return (
    <Term>
      {state === 'coverage' && (
        <>
          <Comment>Welche Akzeptanzkriterien von M0 haben noch keinen Test? (Testnamen tragen die AC-ID)</Comment>
          <Prompt cwd="~/Develop/ai/beton">cargo xtask spec-coverage --milestone M0 --report</Prompt>
          <L>spec-coverage M0: 23 von 214 ACs ohne Test</L>
          {[
            ['CLI-002', '4', 'Must', '`beton run`'],
            ['CLI-003', '1', 'Must', '`resume`, `attach` & `session`-Verwaltung'],
            ['API-003', '2', 'Should', 'SSE-Stream für Skripte'],
            ['API-006', '3', 'Must', 'Skript-Modus `beton run -p`'],
            ['RUN-003', '2', 'Must', 'Runner-Lebenszyklus & Supervision'],
            ['QA-007', '1', 'Must', 'E2E-Tests Web (Playwright)'],
          ].map(([id, ac, prio, t]) => (
            <L key={id + ac}>
              {'  '}
              {id} AC{ac} <S tone={prio === 'Must' ? 'deny' : 'dim'}>[{prio}]</S> {t}
            </L>
          ))}
          <L tone="dim">{'  '}… 17 weitere</L>
          <Blank />
          <Prompt cwd="~/Develop/ai/beton">cargo xtask spec-check</Prompt>
          <L>spec-check: 392 Features ok</L>
        </>
      )}
      {state === 'checks' && (
        <>
          <Comment>Dieselben Pflicht-Checks wie in CI, lokal vor dem PR</Comment>
          <Prompt cwd="~/Develop/ai/beton">cargo xtask commit-lint --range origin/main..HEAD</Prompt>
          <L tone="deny">3e1f0a9c2b: feat braucht eine Feature-ID, z. B. „feat(cli): CLI-014 …“</L>
          <Prompt cwd="~/Develop/ai/beton">git commit --amend -s -m "feat(cli): CLI-014 dynamische Vervollständigung von Session-IDs"</Prompt>
          <Prompt cwd="~/Develop/ai/beton">cargo xtask commit-lint --range origin/main..HEAD && cargo xtask dco --range origin/main..HEAD</Prompt>
          <L>commit-lint: 4 Commit(s) ok</L>
          <L>dco: 4 Commit(s) ok</L>
          <Prompt cwd="~/Develop/ai/beton">cargo nextest run --workspace</Prompt>
          <L>
            {'     Summary [  38.214s] 1412 tests run: '}
            <S tone="ok">1412 passed</S>, 3 skipped
          </L>
          <Prompt cwd="~/Develop/ai/beton">cargo llvm-cov -p beton-policy --summary-only | tail -1</Prompt>
          <L>
            TOTAL{'   '}Zeilen <S tone="ok">92,4 %</S> (Floor 90 %, letzter Stand 92,1 %)
          </L>
        </>
      )}
      {state === 'fake' && (
        <>
          <Comment>Ohne echte Vendor-CLI: Fake-Harness spielt ein Szenario ab (Tests, Demos, Benchmarks)</Comment>
          <Prompt cwd="~/Develop/ai/beton">beton run fake --scenario tests/scenarios/approval-push.yaml</Prompt>
          <L tone="dim">Session ses_t001 · fake · Szenario approval-push (6 Schritte)</L>
          <L>
            <S tone="direct">● fake</S>
          </L>
          <Tool name="Shell" target="git push -u origin feature/x" meta="" status="wait" />
          <Ask title="Freigabe nötig · Shell · Regel git-push-fragen" question="Erlauben?" answered="y">
            <L>{'  '}git push -u origin feature/x</L>
          </Ask>
          <Tool name="Shell" target="git push -u origin feature/x" meta="0,0 s" />
          <L>{'  '}Szenario erfüllt: 6/6 Schritte, erwartete Eingaben erhalten.</L>
        </>
      )}
      {state === 'golden' && (
        <>
          <Comment>Neue Claude-CLI-Version: Golden-Transkripte neu aufnehmen, Drift im PR zeigen</Comment>
          <Prompt cwd="~/Develop/ai/beton">beton dev record-golden --harness claude --cli-version 2.4.0</Prompt>
          <L tone="dim">12 Szenarien aufgenommen · Secrets normalisiert · tests/golden/claude/2.4.0/</L>
          <Prompt cwd="~/Develop/ai/beton">cargo nextest run -p beton-harness-claude golden</Prompt>
          <L tone="deny">FAIL golden::tool_use_streaming (2.4.0)</L>
          <L tone="deny">{'  - {"type":"content_block_delta","delta":{"type":"input_json_delta", …}}'}</L>
          <L tone="ok">{'  + {"type":"content_block_delta","delta":{"type":"tool_input_delta", …}}'}</L>
          <L>
            {'     Summary 11 '}
            <S tone="ok">passed</S>, 1 <S tone="deny">failed</S> · 2.4.0 bleibt „nicht unterstützt“, bis der Adapter angepasst ist
          </L>
        </>
      )}
    </Term>
  )
}

/* ───────────────────────── Gruppe ───────────────────────── */

export const group: ScreenGroup = {
  id: 'cli',
  title: 'Kommandozeile',
  order: 210,
  screens: [
    {
      id: 'cli-run',
      title: 'beton run',
      description:
        'Session im Terminal starten: startet bei Bedarf den lokalen Daemon, streamt Tool-Calls und Antworten in Stimmfarbe, Freigaben fragen gelb mit [y/N]. Läuft über die Anmeldung der offiziellen CLI, ohne API-Schlüssel.',
      features: ['CLI-002', 'CLI-001'],
      frame: 'terminal',
      states: [
        { id: 'streaming', title: 'Agent arbeitet' },
        { id: 'approval', title: 'Freigabe [y/N]' },
        { id: 'interrupted', title: 'Ctrl+C / trennen' },
        { id: 'continue', title: 'Fortsetzen (-c)' },
        { id: 'worktree', title: '--worktree' },
      ],
      component: CliRun,
    },
    {
      id: 'cli-script',
      title: 'Skript-Modus: beton run -p',
      description:
        'Nicht-interaktiv für Skripte und CI: Antwort auf stdout, Session-URL und Fortschritt auf stderr. Ausgabe als Text, JSON oder NDJSON; definierte Exit-Codes bei abgelehnter Freigabe und erschöpftem Budget.',
      features: ['API-006', 'CLI-001'],
      frame: 'terminal',
      states: [
        { id: 'text', title: 'Text auf stdout' },
        { id: 'json', title: '--output-format json' },
        { id: 'stream', title: 'stream-json' },
        { id: 'deny', title: '--on-ask deny (Exit 4)' },
        { id: 'budget', title: '--max-cost (Exit 5)' },
      ],
      component: CliScript,
    },
    {
      id: 'cli-sessions',
      title: 'Sessions: attach, resume, session',
      description: 'Sessions auflisten, verbinden (Replay und live, auch nur lesend), fortsetzen, forken. IDs gehen als vollständige ID, eindeutiges Präfix oder „last“.',
      features: ['CLI-003', 'CLI-001'],
      frame: 'terminal',
      states: [
        { id: 'list', title: 'session list' },
        { id: 'attach', title: 'attach --read-only' },
        { id: 'ambiguous', title: 'Mehrdeutiges Präfix' },
        { id: 'fork', title: 'fork & resume' },
      ],
      component: CliSessions,
    },
    {
      id: 'cli-serve',
      title: 'serve, host & login',
      description:
        'Lokal startet `beton serve` nur auf 127.0.0.1. Ein zentraler Server, weitere Hosts und `beton login` sind optional; Hosts verbinden sich ausgehend und laufen als User-Dienst.',
      features: ['CLI-004', 'CLI-006', 'CLI-001'],
      frame: 'terminal',
      states: [
        { id: 'local', title: 'serve (lokal)' },
        { id: 'refused', title: '--bind 0.0.0.0 abgelehnt' },
        { id: 'pair', title: 'host pair' },
        { id: 'enable', title: 'host enable' },
        { id: 'login', title: 'login & profile' },
      ],
      component: CliServe,
    },
    {
      id: 'cli-setup',
      title: 'setup, doctor & diagnose',
      description:
        'Einrichtung erkennt die Vendor-CLIs und ihre Anmeldung und bietet fehlende CLIs mit exaktem Befehl an – nie still. `doctor` prüft die Umgebung, `diagnose` erzeugt ein Bundle ohne Secrets, das nichts hochlädt.',
      features: ['CLI-005', 'DIST-015', 'CLI-001'],
      frame: 'terminal',
      states: [
        { id: 'setup', title: 'setup (Installationsangebot)' },
        { id: 'noninteractive', title: 'setup --non-interactive' },
        { id: 'doctor', title: 'doctor' },
        { id: 'json', title: 'doctor --json' },
        { id: 'diagnose', title: 'diagnose' },
      ],
      component: CliSetup,
    },
    {
      id: 'cli-config',
      title: 'config',
      description: 'Effektive Konfiguration mit Herkunft je Schlüssel; ungültige Werte werden gegen das Schema abgelehnt, die Datei bleibt unverändert.',
      features: ['CLI-008'],
      frame: 'terminal',
      states: [
        { id: 'list', title: 'config list' },
        { id: 'invalid', title: 'Ungültiger Wert' },
        { id: 'set', title: 'set / get / edit' },
      ],
      component: CliConfig,
    },
    {
      id: 'cli-import',
      title: 'import & export',
      description: 'Chats von Claude Code, Codex und Gemini CLI übernehmen (nur lokal gelesen) und Sessions als JSONL exportieren und wieder einlesen.',
      features: ['CLI-007'],
      frame: 'terminal',
      states: [
        { id: 'pick', title: 'Auswahl' },
        { id: 'last', title: '--last 5' },
        { id: 'export', title: 'export / import' },
      ],
      component: CliImport,
    },
    {
      id: 'cli-agent',
      title: 'agent',
      description: 'Agent-Definitionen aus eingebaut, User und Projekt auflisten, gegen das Schema prüfen (Fehler mit Pfad) und neu anlegen.',
      features: ['CLI-009'],
      frame: 'terminal',
      states: [
        { id: 'list', title: 'agent list' },
        { id: 'validate', title: 'validate (Fehler)' },
        { id: 'new', title: 'new' },
      ],
      component: CliAgent,
    },
    {
      id: 'cli-usage',
      title: 'usage',
      description: 'Verbrauch nach Modell: Subscriptions zeigen Tokens und Abo-Fenster statt Euro, API-/lokale Nutzung getrennt mit Kosten – nie zusammengerechnet.',
      features: ['CLI-010'],
      frame: 'terminal',
      states: [
        { id: 'table', title: 'Tabelle' },
        { id: 'json', title: '--json' },
      ],
      component: CliUsage,
    },
    {
      id: 'cli-schedule',
      title: 'schedule',
      description: 'Zeitpläne anlegen, pausieren, sofort starten und die Lauf-Historie ansehen.',
      features: ['CLI-011'],
      frame: 'terminal',
      states: [
        { id: 'list', title: 'list & run-history' },
        { id: 'create', title: 'create' },
        { id: 'run-now', title: 'run-now --json' },
      ],
      component: CliSchedule,
    },
    {
      id: 'cli-plugin',
      title: 'plugin',
      description:
        'Plugins aus lokalem Pfad, Git oder Registry installieren. Vor Installation und vor Updates mit mehr Rechten zeigt beton die Berechtigungen und wartet auf deine Bestätigung.',
      features: ['CLI-012', 'PLG-005', 'PLG-008', 'PLG-010', 'CLI-001'],
      frame: 'terminal',
      states: [
        { id: 'install', title: 'install ./pfad' },
        { id: 'update', title: 'update (mehr Rechte)' },
        { id: 'list', title: 'list --json' },
      ],
      component: CliPlugin,
    },
    {
      id: 'cli-upgrade',
      title: 'upgrade & uninstall',
      description:
        'Updates nur auf Aufruf: `upgrade` erkennt die Installationsart, aktualisiert Skript-Installationen selbst (geprüft), nennt sonst den Paketmanager-Befehl. Auch offline per Datei. `uninstall --purge` fragt vorher. Offener Punkt: `--file` fehlt in DIST-016.',
      features: ['CLI-013', 'DIST-016', 'DIST-017', 'DIST-020'],
      frame: 'terminal',
      states: [
        { id: 'check', title: '--check' },
        { id: 'script', title: 'Selbst-Update' },
        { id: 'file', title: 'Update aus Datei' },
        { id: 'brew', title: 'Homebrew' },
        { id: 'busy', title: 'Turns laufen' },
        { id: 'uninstall', title: 'uninstall --purge' },
      ],
      component: CliUpgrade,
    },
    {
      id: 'cli-completion',
      title: 'Shell-Vervollständigung',
      description: 'Completion für bash, zsh, fish und PowerShell, mit Session-IDs samt Titel und Profilen. Ist der Server weg, bleibt die Liste leer statt die Shell zu blockieren.',
      features: ['CLI-014'],
      frame: 'terminal',
      states: [
        { id: 'zsh', title: 'zsh: attach <TAB>' },
        { id: 'offline', title: 'Server nicht erreichbar' },
      ],
      component: CliCompletion,
    },
    {
      id: 'cli-help',
      title: 'Kommandobaum & Konventionen',
      description: '`beton --help` mit dem Kommandobaum, die Trennung von stdout (Nutzdaten) und stderr (Diagnose), Farben nur im Terminal und die festen Exit-Codes.',
      features: ['CLI-001'],
      frame: 'terminal',
      states: [
        { id: 'help', title: '--help' },
        { id: 'conventions', title: 'stdout/stderr' },
        { id: 'exit', title: 'Exit-Codes' },
      ],
      component: CliHelp,
    },
    {
      id: 'cli-api',
      title: 'API-Referenz & SDKs',
      description:
        'REST-API mit OpenAPI 3.1 des laufenden Servers, Endpunkte nach Ressource, Beispiel-Request mit Zugangstoken sowie TypeScript- und Rust-SDK. Das Token ist ein beton-Token, kein Modell-API-Schlüssel.',
      features: ['API-001', 'API-002', 'API-004', 'API-005', 'RUN-019'],
      states: [
        { id: 'reference', title: 'Endpunkte' },
        { id: 'sdk', title: 'SDKs' },
      ],
      component: ApiReference,
    },
    {
      id: 'cli-sse',
      title: 'SSE-Stream für Skripte',
      description: 'Events einer Session als Server-Sent Events, ohne WebSocket. Jedes Event trägt seine seq als id; mit Last-Event-ID geht es nach Abbruch lückenlos weiter.',
      features: ['API-003'],
      frame: 'terminal',
      states: [
        { id: 'stream', title: 'from_seq=0' },
        { id: 'resume', title: 'Last-Event-ID' },
      ],
      component: CliSse,
    },
    {
      id: 'cli-dev',
      title: 'Entwickler-Werkzeuge',
      description:
        'Für Beitragende: Spec-Abdeckung je Meilenstein, Commit- und DCO-Prüfung wie in CI, Fake-Harness-Szenarien und Golden-Drift bei neuen Vendor-CLI-Versionen. Kein Produkt-UI, aber die Werkzeuge hinter den QA-Features.',
      features: [],
      frame: 'terminal',
      states: [
        { id: 'coverage', title: 'spec-coverage' },
        { id: 'checks', title: 'Lokale CI-Checks' },
        { id: 'fake', title: 'Fake-Harness' },
        { id: 'golden', title: 'Golden-Drift' },
      ],
      component: CliDev,
    },
  ],
  noUi: {
    'QA-001': { reason: 'Teststrategie des Projekts (Ebenen und Orte der Tests), beschrieben in AGENTS.md; kein Produkt-UI.', visibleIn: 'cli-dev' },
    'QA-002': { reason: 'Test-Binary beton-fake-cli simuliert die Vendor-Protokolle in Tests; Nutzer sehen es nicht.', visibleIn: 'cli-dev' },
    'QA-003': { reason: 'CI-Gate und Drift-Prozess für Golden-Transkripte; sichtbar nur im Entwickler-Terminal und im PR.', visibleIn: 'cli-dev' },
    'QA-004': { reason: 'CI-Gate für Policy-Testfälle; die Oberfläche `beton policy test` gehört zu POL-026.' },
    'QA-005': { reason: 'Sandbox-Escape-Suite läuft in der CI-Matrix (macOS, Linux, Windows); kein UI.' },
    'QA-006': { reason: 'Snapshot-Tests der generierten Schemas und des OpenAPI-Dokuments in CI; Wirkung: stabile API-Referenz.', visibleIn: 'cli-api' },
    'QA-007': { reason: 'E2E-Tests der Web-UI mit Playwright in CI; kein eigenes UI.' },
    'QA-008': { reason: 'E2E-Tests der Desktop-App über tauri-driver in CI; kein eigenes UI.' },
    'QA-009': { reason: 'Property- und Fuzz-Tests für Parser in CI; kein UI.' },
    'QA-010': { reason: 'CI-Pipeline mit Pflicht-Checks je PR; lokal mit denselben Befehlen ausführbar.', visibleIn: 'cli-dev' },
    'QA-011': { reason: 'Coverage-Floor als CI-Gate; lokal per cargo llvm-cov einsehbar.', visibleIn: 'cli-dev' },
    'QA-012': { reason: 'Benchmarks (criterion, Last-Harness) in CI; Wirkung sind die Performance-Budgets, kein UI.' },
    'QA-013': { reason: 'Prozessregel: TDD und menschliches Review über CODEOWNERS und Branch-Protection; kein UI.' },
    'QA-014': { reason: 'Arbeitsablauf mit Feature-IDs in Commits, PRs und Testnamen; sichtbar in spec-coverage und commit-lint.', visibleIn: 'cli-dev' },
    'QA-015': { reason: 'DCO-Check (Signed-off-by) in CI; lokal mit cargo xtask dco.', visibleIn: 'cli-dev' },
    'QA-016': { reason: 'Gemeinsame Contract-Suite für Harness-Adapter in CI; dieselbe Suite prüft Harness-Plugins.', visibleIn: 'plugins-dev' },
    'QA-017': { reason: 'Datenbank-Matrix (SQLite, Postgres) in CI; kein UI.' },
    'QA-018': {
      reason:
        'CI-Gate: Server, Clients und Fake-Harness bestehen die Demo-Szenarien in einem Netz-Namespace nur mit Loopback; ein zweiter Lauf weist nach, dass nichts nach außen verbindet.',
      visibleIn: 'cli-dev',
    },
    'QA-019': {
      reason:
        'Release-Prozess: Vor jedem Release prüft der Maintainer alle Wege mit echten Subscription-Logins (claude, codex, Gemini via ACP) und checkt das Protokoll ein.',
      visibleIn: 'harness-setup',
    },
  },
}
