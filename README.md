# beton

> Ein Meta-Harness für KI-Coding-Agents, geschrieben in Rust.

beton ist eine gemeinsame Schicht über **Claude Code**, **Codex**, **ACP-Agents** (Gemini CLI, Goose, Qwen …) und eigene Agents:

- **Komposition:** Harnesses tauschen und kombinieren, ohne etwas umzubauen; Sessions über Harness-Grenzen forken; Agents in YAML definieren.
- **Kontrolle:** hierarchische Policies (Budgets, Approvals, Git-Guards) mit CEL, eine OS-Sandbox und ein Egress- bzw. Credential-Proxy. Damit ist auch YOLO-Mode sicher.
- **Collaboration:** dieselbe Live-Session aus der Desktop-App (Tauri), dem Browser, dem Terminal und vom Handy.
- **Subscriptions first:** Claude Pro/Max und ChatGPT Plus/Pro funktionieren über die offiziellen CLIs, nicht nur API-Keys.
- **Lokal-first:** ein einzelnes Binary, das ohne Server-Setup läuft und bei Bedarf als Team-Server skaliert.

> **Status:** Spezifikationsphase, es gibt noch keinen Code. Die Spec liegt unter [`docs/spec/`](docs/spec/00-overview.md), die Entscheidungen unter [`docs/adr/`](docs/adr/README.md).

## Dokumentation

| Dokument | Inhalt |
|---|---|
| [Überblick](docs/spec/00-overview.md) | Vision, Architektur, Glossar, Kapitelübersicht |
| [Roadmap](docs/spec/roadmap.md) | Meilensteine M0–M5 und Zuordnung der Features |
| [ADRs](docs/adr/README.md) | 31 Architekturentscheidungen |
| [Omnigent-Recherche](research/omnigent-features.md) | Feature-Inventar des Vorbilds |
| [AGENTS.md](AGENTS.md) | Arbeitsanweisungen für Coding-Agents |

## Lizenz

[Apache-2.0](LICENSE). Beiträge erfolgen unter dem [Developer Certificate of Origin](https://developercertificate.org/) (`git commit -s`), siehe [CONTRIBUTING.md](CONTRIBUTING.md).
