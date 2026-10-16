# ADR-0038: Vendor-Hosts in Stufe 1 als Passthrough, MITM nur für Model-API-Hosts bei `auth: api_key`

- **Status:** Akzeptiert
- **Datum:** 2026-10-16
- **Entscheider:** Coding-Agent (delegiert, AGENTS.md § 5); umgesetzt und security-reviewed in WP-45 und WP-46 (#168, #169, PRs #228, #291)

## Kontext
Nach ADR-0005 sieht beton Subscription-Tokens der Vendor-CLIs nie. Nach ADR-0007 und ADR-0024 läuft aller Netzverkehr der Sandbox über den Egress-Proxy, der TLS terminiert, Regeln prüft und Credentials für Platzhalter `bt_cred_*` einsetzt. Beides zusammen verlangt für die Vendor-CLI in Stufe 1 eine Ausnahme: Ihr eigener Login muss den Proxy verschlüsselt passieren. Mit `auth: api_key` (HAR-015) gilt das Gegenteil: Der Harness soll nur einen Platzhalter sehen (HAR-015 AC4), also muss der Proxy den echten Key einsetzen, was ohne TLS-Terminierung nicht geht. PRX-005 schloss Injection auf Passthrough-Verbindungen aus. Die tatsächlichen Hosts der CLIs waren *(Annahme)*; WP-45 hat sie gegen claude 2.1.296 und codex-cli 0.161.0 per lokalem CONNECT-Mitschnitt ermittelt.

## Betrachtete Optionen
1. **A — Vendor-Hosts immer per MITM** — einheitlich, aber beton sähe Subscription-Tokens im Klartext (Verstoß gegen ADR-0005).
2. **B — Vendor-Hosts immer Passthrough, echter API-Key im Env der CLI** — einfach, aber der Key läge in der Sandbox (Verstoß gegen HAR-015 AC4 und ADR-0024).
3. **C — Passthrough als Default, MITM nur für die Model-API-Hosts einer Session mit `auth: api_key`** — erfüllt beide Anforderungen; die CLI muss der beton-CA vertrauen.

## Entscheidung
Option **C**:
- Jeder Adapter deklariert seine Vendor-Hosts als exakte Hostnamen mit optionalem Port (keine Wildcards, keine IP-Literale). Stand WP-45: Claude `api.anthropic.com`, `platform.claude.com`, `claude.ai`, `mcp-proxy.anthropic.com`; Codex `chatgpt.com`, `ab.chatgpt.com`, `auth.openai.com`, `api.openai.com`. Telemetrie- und Auto-Update-Hosts bleiben gesperrt. Erweiterbar nur über die User-Konfiguration (`sandbox.harness.egress_passthrough`).
- Passthrough heißt: CONNECT ohne TLS-Terminierung; geprüft werden Host, Port, private IP (PRX-007) und dass die SNI dem CONNECT-Host entspricht. Aus Stufe 2 sind diese Hosts ohne eigene Regel nicht erreichbar.
- Mit `auth: api_key` laufen nur die **Model-API-Hosts** (Claude `api.anthropic.com`, Codex `api.openai.com`) dieser Session über MITM mit einer Stufe-1-Regel `* <host>/**`. Das Binding `harness:<id>` gilt nur für diese Hosts und injiziert nicht implizit. Login- und Token-Hosts bleiben Passthrough. Mit `auth: subscription` ändert sich nichts.
- Die Host-Listen werden pro Vendor-Release geprüft (QA-019).

## Konsequenzen
- Positiv: Subscription-Tokens werden nie entschlüsselt; ein API-Key gelangt nie in die Sandbox.
- Negativ / Risiken: Passthrough-Verbindungen sind für Pfad- und Methodenregeln undurchsichtig. Host-Listen veralten mit Vendor-Releases. Ob die echten CLIs der beton-CA über die CA-Variablen vertrauen (Node über `NODE_EXTRA_CA_CERTS`, Codex als Rust-Binary), ist noch mit eingeloggten CLIs zu prüfen (#260); Go-Programme unter macOS ignorieren `SSL_CERT_FILE` (PRX-008, scheitern geschlossen).
- Folgearbeiten: Checkliste #260; Host-Listen in die Release-Checkliste (QA-019, M3) aufnehmen.

## Bezug
- Spec: docs/spec/04-sandbox.md (PRX-005, PRX-006, PRX-008), docs/spec/01-harnesses.md (HAR-011, HAR-015)
- ADR-0005, ADR-0007, ADR-0024, ADR-0034
- Issues: #168, #169, #260
