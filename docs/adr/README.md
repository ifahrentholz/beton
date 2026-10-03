# Architecture Decision Records (ADRs)

Dieses Verzeichnis enthält die Architekturentscheidungen von **beton**. Ein ADR hält eine einzelne, bedeutsame Entscheidung fest: den Kontext, die betrachteten Optionen, die gewählte Lösung und deren Konsequenzen. ADRs sind die verbindliche Begründung hinter der Spec (`docs/spec/`) und dienen Menschen wie Coding-Agents als Referenz, *warum* etwas so gebaut ist.

## Neuen ADR anlegen

1. **Nächste freie Nummer** wählen (vierstellig, fortlaufend, z. B. `0032`). Nummern werden nie wiederverwendet.
2. [`template.md`](template.md) kopieren nach `NNNN-kebab-titel.md` (Titel kurz, Kleinbuchstaben, Bindestriche, keine Umlaute im Dateinamen).
3. Abschnitte *Kontext*, *Betrachtete Optionen*, *Entscheidung*, *Konsequenzen* und *Bezug* (Spec-Kapitel + Feature-Prefix) ausfüllen; knapp halten (ca. 30–70 Zeilen).
4. Mit Status **Vorgeschlagen** einreichen; nach Freigabe auf **Akzeptiert** setzen.
5. Den ADR in die Index-Tabelle unten eintragen.

Akzeptierte ADRs werden inhaltlich nicht mehr geändert. Ändert sich eine Entscheidung, entsteht ein **neuer** ADR; der alte erhält den Status *Ersetzt durch ADR-XXXX* (und der neue verweist auf den alten).

## Status-Werte

| Status | Bedeutung |
| --- | --- |
| Vorgeschlagen | Entwurf, noch nicht entschieden |
| Akzeptiert | Verbindlich; Spec und Code folgen dieser Entscheidung |
| Ersetzt durch ADR-XXXX | Nicht mehr gültig; der genannte ADR gilt stattdessen |

## Index

| Nr | Titel | Status | Link |
| --- | --- | --- | --- |
| 0001 | Produktziel – eigenständiges Produkt nach Omnigent-Vorbild, keine Kompatibilität | Akzeptiert | [0001-produktziel-eigenstaendiges-produkt.md](0001-produktziel-eigenstaendiges-produkt.md) |
| 0002 | Open Source als Vertriebsmodell | Akzeptiert | [0002-open-source-vertriebsmodell.md](0002-open-source-vertriebsmodell.md) |
| 0003 | Prozessmodell – ein Binary mit Modi, lokal = zentral nur Konfiguration | Akzeptiert | [0003-prozessmodell-ein-binary-mit-modi.md](0003-prozessmodell-ein-binary-mit-modi.md) |
| 0004 | Desktop-UI mit Tauri 2 | Akzeptiert | [0004-desktop-ui-mit-tauri-2.md](0004-desktop-ui-mit-tauri-2.md) |
| 0005 | Harness-Integration – ein Adapter-Interface mit drei Transporten; Subscription nur via offizielle CLI | Akzeptiert | [0005-harness-integration-adapter-drei-transporte.md](0005-harness-integration-adapter-drei-transporte.md) |
| 0006 | v1-Harness-Umfang | Akzeptiert | [0006-v1-harness-umfang.md](0006-v1-harness-umfang.md) |
| 0007 | Sandbox – Provider-Interface, zweistufig, OS-Backends und Egress-/Credential-Proxy in Rust | Akzeptiert | [0007-sandbox-provider-interface.md](0007-sandbox-provider-interface.md) |
| 0008 | Policies – YAML + CEL, eingebaute Regeltypen, hierarchisch, strengere gewinnt | Akzeptiert | [0008-policies-yaml-und-cel.md](0008-policies-yaml-und-cel.md) |
| 0009 | Storage – SQLite lokal + Postgres zentral, event-sourced, Blob-Store | Akzeptiert | [0009-storage-sqlite-postgres-event-sourced.md](0009-storage-sqlite-postgres-event-sourced.md) |
| 0010 | Sync – Single-Writer + Fork bei Divergenz, server-autoritative Policies, Budget-Leases | Akzeptiert | [0010-sync-single-writer-fork-budget-leases.md](0010-sync-single-writer-fork-budget-leases.md) |
| 0011 | Auth – lokal Token, zentral OIDC, Device-Pairing, PATs, Rollen; kein SCIM in v1 | Akzeptiert | [0011-auth-token-oidc-device-pairing.md](0011-auth-token-oidc-device-pairing.md) |
| 0012 | Agent-Definition – YAML + JSON-Schema, nur MCP für Tools, Skills, Cross-Harness-Sub-Agents, Built-ins als YAML | Akzeptiert | [0012-agent-definition-yaml-mcp-skills.md](0012-agent-definition-yaml-mcp-skills.md) |
| 0013 | Async-Agents, Timer und Schedules in v1; Webhooks nur via API; Approval ohne Zuschauer | Akzeptiert | [0013-async-agents-timer-schedules.md](0013-async-agents-timer-schedules.md) |
| 0014 | Collaboration-Scope v1 | Akzeptiert | [0014-collaboration-scope-v1.md](0014-collaboration-scope-v1.md) |
| 0015 | Client-Landschaft v1 | Akzeptiert | [0015-client-landschaft-v1.md](0015-client-landschaft-v1.md) |
| 0016 | Eingebetteter Browser – Chromium via CDP-Screencast, Umschalter auf sichtbares Fenster, Inspect-Mode → Agent | Akzeptiert | [0016-eingebetteter-browser-cdp-screencast.md](0016-eingebetteter-browser-cdp-screencast.md) |
| 0017 | Runner-Provider – lokal, Remote-Host, Docker/Podman, Kubernetes; Runner-Image; SaaS-Provider v2 | Akzeptiert | [0017-runner-provider.md](0017-runner-provider.md) |
| 0018 | Erweiterbarkeit – Crates (eingebaut), Out-of-Process JSON-RPC (Community), WASM (Policies), UI-Extensions v2 | Akzeptiert | [0018-erweiterbarkeit-crates-plugins-wasm.md](0018-erweiterbarkeit-crates-plugins-wasm.md) |
| 0019 | Eigenes Event-Modell + WebSocket mit Resume ab `seq`, Binärkanäle, Typen aus Rust generiert | Akzeptiert | [0019-event-modell-und-websocket-protokoll.md](0019-event-modell-und-websocket-protokoll.md) |
| 0020 | Frontend-Stack – React 19 + TypeScript + Vite + TanStack + Tailwind/shadcn + Zustand | Akzeptiert | [0020-frontend-stack-react.md](0020-frontend-stack-react.md) |
| 0021 | Umfang weiterer Plattform-Features in v1 | Akzeptiert | [0021-weitere-plattform-features-v1.md](0021-weitere-plattform-features-v1.md) |
| 0022 | Smart Routing auf v2 verschoben | Akzeptiert | [0022-smart-routing-auf-v2-verschoben.md](0022-smart-routing-auf-v2-verschoben.md) |
| 0023 | Spracheingabe – nur lokales Whisper im Server/Daemon, keine Cloud | Akzeptiert | [0023-spracheingabe-lokales-whisper.md](0023-spracheingabe-lokales-whisper.md) |
| 0024 | Secrets – Keychain lokal, Envelope-Encryption zentral, Proxy-Injection, Audit | Akzeptiert | [0024-secrets-management.md](0024-secrets-management.md) |
| 0025 | Telemetrie Opt-in, volle Betreiber-Observability | Akzeptiert | [0025-telemetrie-opt-in-und-observability.md](0025-telemetrie-opt-in-und-observability.md) |
| 0026 | Distribution, Signing und Updates | Akzeptiert | [0026-distribution-signing-updates.md](0026-distribution-signing-updates.md) |
| 0027 | Lizenz Apache-2.0 mit DCO | Akzeptiert | [0027-lizenz-apache-2-0-mit-dco.md](0027-lizenz-apache-2-0-mit-dco.md) |
| 0028 | Projektname "beton" | Akzeptiert | [0028-projektname-beton.md](0028-projektname-beton.md) |
| 0029 | Hosting auf persönlichem GitHub-Account `ifahrentholz` | Akzeptiert | [0029-hosting-persoenlicher-github-account.md](0029-hosting-persoenlicher-github-account.md) |
| 0030 | Roadmap M0–M5, erstes öffentliches Release nach M3, Solo-Entwicklung mit Coding-Agents | Akzeptiert | [0030-roadmap-meilensteine-m0-m5.md](0030-roadmap-meilensteine-m0-m5.md) |
| 0031 | Qualitätsstrategie für agent-getriebene Entwicklung | Akzeptiert | [0031-qualitaetsstrategie-agent-getriebene-entwicklung.md](0031-qualitaetsstrategie-agent-getriebene-entwicklung.md) |
