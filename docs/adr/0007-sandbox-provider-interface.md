# ADR-0007: Sandbox – Provider-Interface, zweistufig, OS-Backends und Egress-/Credential-Proxy in Rust

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Ziel ist ein **sicherer YOLO-Mode**: Agents laufen unbeaufsichtigt, der Kernel erzwingt die Grenzen. Omnigents "Omnibox" nutzt bwrap+seccomp (Linux), Seatbelt (macOS) und unter Windows nur ein Job Object (keine FS-/Netz-Isolation). Native-TUI-Harnesses laufen dort faktisch unsandboxed. Der Egress-/Credential-Proxy ist ein Python-asyncio-MITM. Problem: Die Vendor-CLI muss ihre eigenen Credentials (Datei/Keychain) lesen dürfen, die von ihr ausgeführten Tools aber nicht.

## Betrachtete Optionen
1. **A — Nur Container (Docker/Podman)** — starke Isolation überall; schwergewichtig, Docker-Pflicht, schlechter lokaler UX.
2. **B — Nur OS-native Sandbox** — leichtgewichtig; je OS unterschiedlich stark, Windows schwach.
3. **C — Provider-Interface mit OS-Backends + Container, zweistufig, eigener Rust-Proxy** — flexibel, plattformübergreifend einheitliches Netzmodell; hoher Implementierungs- und Testaufwand.
4. **D — MicroVMs (Firecracker, libkrun)** — sehr stark; plattformabhängig, v1 zu aufwendig.

## Entscheidung
Option **C**. Crate `beton-sandbox` mit **Sandbox-Provider-Interface**:
- macOS: **Seatbelt** (`sandbox-exec`, SBPL, deny-default). Linux: **Landlock + seccomp** (+ Namespaces; bubblewrap optional). Windows: **Beta**, "best effort" (Restricted Token + Job Object, ggf. AppContainer). **Docker/Podman** als starke Option auf allen Plattformen.
- **Zweistufig:** (1) Harness-Prozess – die `claude`/`codex`-CLI darf eigene Credential-Dateien/Keychain lesen; (2) **Tool-Ausführung** (Bash etc.) in strengerer Sandbox ohne diesen Zugriff.
- **Egress-Proxy in Rust** (`beton-proxy`), plattformübergreifend identisch: TLS-MITM mit eigener CA, Allowlist nach Domain/Methode/Pfad (Default-Deny), private IPs blockiert (DNS-Rebinding-Schutz), **Credential-Injection**: Agent sieht nur Platzhalter `bt_cred_*`, der Proxy injiziert den echten Wert. Die OS-Sandbox sperrt jedes Netz außer zum Proxy.
- Sandbox verlangt, aber nicht verfügbar → **Fehler statt unsandboxed**. Environment deny-by-default.

## Konsequenzen
- Positiv: Einheitliches Sicherheitsmodell; Windows liegt über Omnigent-Niveau; YOLO-Mode wird vertretbar.
- Negativ / Risiken: Seatbelt ist von Apple deprecated/undokumentiert; Landlock erfordert neuere Kernel (ABI-Versionen prüfen); CA-Injection in alle Toolchains (Node, Python, Go, Rust, Java) ist lückenanfällig.
- Negativ / Risiken: Sicherheitskritischer Code → TDD, Sandbox-Escape-Tests pro OS in CI, menschliches Review (ADR-0031).
- Folgearbeiten: Non-HTTP-Traffic (SSH, raw TCP, DNS) explizit behandeln; `beton doctor` meldet Sandbox-Fähigkeiten; Windows-Beta in M5.

## Bezug
- Spec: docs/spec/04-sandbox.md (Prefix SBX/PRX)
