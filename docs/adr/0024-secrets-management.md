# ADR-0024: Secrets – Keychain lokal, Envelope-Encryption zentral, Proxy-Injection, Audit

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton verwaltet API-Keys (Direkt-API-Harness, Gateways), Git-Provider-Tokens und weitere Credentials für Tools. Subscription-Credentials verwaltet beton bewusst nicht (ADR-0005). Agents dürfen Secrets nie im Klartext sehen (Prompt-Injection, Logs, Transkripte). Omnigent nutzt den OS-Keychain lokal, KMS/Vault-Transit zentral und einen Credential-Proxy mit Platzhaltern (`oa_cred_*`).

## Betrachtete Optionen
1. **A — Env-Variablen/Config-Dateien** — einfach; Klartext, Agent kann sie lesen.
2. **B — Lokal Keychain, zentral Envelope-Encryption mit Master-Key aus Env/Datei; Injection nur im Proxy** — sicher genug für v1, ohne externe Abhängigkeit.
3. **C — Zentral zwingend Vault/KMS** — Enterprise-Standard; schwere Abhängigkeit für kleine Teams.

## Entscheidung
Option **B**, Vault/KMS als v2-Plugin:
- **Lokal:** OS-Keychain (`keyring`-Crate); Fallback verschlüsselte Datei (Argon2-abgeleitete Passphrase).
- **Zentral:** Envelope-Encryption (AES-256-GCM, Data-Key pro Secret, Master-Key aus Env/Datei in v1; Vault/KMS als Plugin in v2).
- **Bindung** `(org, user|team, provider, account)`.
- **Agent sieht Secrets nie im Klartext:** nur Platzhalter `bt_cred_*`, der Egress-Proxy injiziert den echten Wert (ADR-0007).
- **Git-Provider:** OAuth-App für GitHub (inkl. Enterprise) und GitLab (inkl. self-hosted) pro User, PAT als Fallback.
- **Audit-Log** jeder Secret-Nutzung (wer, Session, Host) – nie der Wert.

## Konsequenzen
- Positiv: Secrets tauchen weder in Prompts noch in Logs/Transkripten auf; nachvollziehbare Nutzung.
- Negativ / Risiken: Master-Key aus Env/Datei ist der Single Point of Failure im zentralen Betrieb → Rotations- und Backup-Doku nötig.
- Negativ / Risiken: Keychain-Verfügbarkeit unter Linux (Secret Service) variiert → Fallback-Pfad testen.
- Folgearbeiten: Key-Rotation, Audit-Log-Retention, Plugin-Interface für KMS/Vault (v2).

## Bezug
- Spec: docs/spec/05-security-identity.md (Prefix SEC), docs/spec/04-sandbox.md (Prefix PRX)
