// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { WorktreeRequest } from "./WorktreeRequest";

export type CreateSessionRequest = { 
/**
 * Harness-ID, z. B. `claude` oder (nur mit `--dev`) `fake`. Mit `agent` optional: dann
 * ein Override von `executor.harness`, vermerkt in `agent.resolved.overrides` (AGT-004).
 */
target?: string, 
/**
 * Agent-Ref (AGT-003): Name (`pr-fixer`), Pfad (`./agents/x`) oder `builtin:<name>`. Der
 * Agent wird beim Start aufgelöst und als Snapshot festgehalten (`agent.resolved`).
 */
agent?: string, 
/**
 * Parameterwerte des Agents (AGT-010); Texte werden in den deklarierten Typ umgewandelt.
 * Ungültige Werte: 422 `invalid_param`; fehlende Pflichtwerte: 422 `params_required` mit
 * `errors[].pointer = /params/<name>`.
 */
params?: Record<string, unknown>, 
/**
 * Arbeitsverzeichnis (Projekt oder Worktree).
 */
cwd: string, title?: string, model?: string, 
/**
 * Reasoning-Effort (`low`, `medium`, `high`, `xhigh`); eine Stufe, die der Harness nicht
 * kennt, wird auf die nächstniedrigere gemappt (HAR-017).
 */
effort?: string, 
/**
 * Permission-Mode (`plan`, `default`, `accept_edits`, `yolo`; HAR-027). `yolo` braucht
 * Tool-Sandbox und Egress-Proxy, sonst `409 sandbox_required`.
 */
permission_mode?: string, 
/**
 * Harness-spezifische Optionen, z. B. `{"scenario": "…"}` beim Fake-Harness.
 */
harness_opts?: Record<string, unknown>, 
/**
 * Eigener `git worktree` mit eigenem Branch für die Session (SES-015). `cwd` muss dann in
 * einem Git-Repository liegen; die Session arbeitet im Worktree.
 */
worktree?: WorktreeRequest, };
