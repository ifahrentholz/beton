# ADR-0018: Erweiterbarkeit – Crates (eingebaut), Out-of-Process JSON-RPC (Community), WASM (Policies), UI-Extensions v2

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Ein Meta-Harness lebt von Breite: Harnesses, Runner-Provider, Git-Provider, Policy-Erweiterungen. Omnigent nutzt Python-Entry-Points (In-Process-Plugins) und sandboxed iframes für UI-Extensions. In Rust sind In-Process-Plugins (dynamische Libraries) wegen fehlender stabiler ABI unpraktisch. Community-Beiträge sollen in beliebigen Sprachen möglich sein.

## Betrachtete Optionen
1. **A — Nur eingebaute Crates** — maximal integriert; jede Erweiterung braucht einen Upstream-PR.
2. **B — Dynamische Libraries (`cdylib`, C-ABI)** — In-Process, schnell; ABI-Fragilität, Absturz- und Sicherheitsrisiko.
3. **C — Out-of-Process-Plugins via JSON-RPC über stdio** — sprachunabhängig, isoliert, MCP-/ACP-Stil bekannt; IPC-Overhead.
4. **D — WASM-Komponenten für alles** — sicher, portabel; für Harness-/Runner-Plugins (Prozesse, Netz) zu einschränkend.

## Entscheidung
Mehrstufig:
- **Eingebaute Adapter als Crates:** Claude, Codex, ACP, Direkt-API, Docker, K8s, GitHub, GitLab.
- **Community-Harnesses, Runner- und Git-Provider als Out-of-Process-Plugins** (Option C): JSON-RPC über stdio, beliebige Sprache, Manifest + deklarierte Berechtigungen; Installation via `beton plugin install <name>` (Registry, Git oder Pfad). Neue Harnesses bevorzugt einfach via ACP.
- **Policy-Erweiterungen als WASM** (wasmtime + WIT).
- **UI-Extensions in v2** (sandboxed iframe + Message-Bridge).

## Konsequenzen
- Positiv: Community kann ohne Rust und ohne Upstream-Release erweitern; Plugin-Abstürze reißen den Core nicht mit.
- Positiv: WASM-Policies laufen deterministisch und sandboxed.
- Negativ / Risiken: Plugin-Protokolle müssen versioniert und stabil gehalten werden; Berechtigungsmodell für Plugins ist sicherheitsrelevant.
- Negativ / Risiken: Registry-Betrieb und Supply-Chain-Fragen (Signaturen) offen.
- Folgearbeiten: Plugin-Host in `beton-plugin`, Manifest-Schema, WIT-Interface für Policies, Plugin-SDK-Beispiele.

## Bezug
- Spec: docs/spec/10-runners-extensibility.md (Prefix PLG)
