# ADR-0011: Auth – lokal Token, zentral OIDC, Device-Pairing, PATs, Rollen; kein SCIM in v1

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton läuft lokal (Single-User) und zentral (Team). Omnigent hat mehrere Auth-Modi (Header-basiert als Default, eingebaute Accounts mit Passwort, OIDC, Device Grant, Client Credentials) – der Default "vertraut `X-Forwarded-Email`" ist ein Sicherheitsrisiko. Runner/Hosts und mobile Geräte müssen sich dauerhaft und widerrufbar anmelden; API-Clients brauchen Tokens mit Scopes.

## Betrachtete Optionen
1. **A — Eingebaute Accounts (Passwort) überall** — keine IdP-Abhängigkeit; Passwortverwaltung, Reset-Flows, Sicherheitsverantwortung.
2. **B — Lokal Token-Datei, zentral ausschließlich OIDC (+ optional Passkeys)** — kein eigenes Passwort-Handling, Unternehmens-IdPs nutzbar.
3. **C — Header-basiert hinter Reverse-Proxy** — einfach; gefährlich bei Fehlkonfiguration.
4. **D — Volles Enterprise-Paket inkl. SCIM** — Provisionierung; zu viel Aufwand für v1.

## Entscheidung
Option **B**:
- **Lokal:** kein Login; Bindung an `localhost` + Token in Datei (Rechte `0600`).
- **Zentral:** **OIDC** (Entra ID, Google, GitHub, Keycloak); optional eingebauter Passkey-/WebAuthn-Login (nicht zwingend v1). **Kein SCIM in v1** – der OIDC-Login legt den User an.
- **Runner/Geräte:** **Device-Pairing** per Code/QR → langlebiges, widerrufbares Gerätetoken im OS-Keychain.
- **API:** Personal Access Tokens und Service-Accounts mit Scopes.
- **Autorisierung:** Rollen pro Org/Team (Owner/Admin/Member/Viewer); Session-Freigaben *view* / *comment+approve* / *drive*.

## Konsequenzen
- Positiv: Kein Passwort-Speicher; sichere Defaults; Geräte einzeln widerrufbar.
- Positiv: Handy-Zugriff auf lokalen Server via Device-Pairing möglich (ADR-0003).
- Negativ / Risiken: Team-Betrieb setzt einen OIDC-IdP voraus; Deprovisionierung ohne SCIM nur über Login-Sperre/Admin-Aktion.
- Folgearbeiten: Token-Rotation, Revocation-Liste, CSWSH-Schutz (WS-Origin-Allowlist), sicherheitskritischer Code mit TDD (ADR-0031).

## Bezug
- Spec: docs/spec/05-security-identity.md (Prefix AUTH)
