// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { WorktreeRequest } from "./WorktreeRequest";

export type CreateSessionRequest = { 
/**
 * Harness-ID, z. B. `claude` oder (nur mit `--dev`) `fake`.
 */
target: string, 
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
