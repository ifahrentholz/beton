# 05 — Security & Identity

Dieses Kapitel spezifiziert **Authentisierung und Autorisierung** (Prefix `AUTH`) sowie **Secrets, Audit und Redaction** (Prefix `SEC`) für beide Betriebsarten: den lokalen Daemon ohne Login und den zentralen Team-Server mit OIDC. Dazu gehören Device-Pairing für Hosts, Runner und Geräte (Handy/PWA), Personal Access Tokens und Service-Accounts, das Rollenmodell Org/Team, die serverseitige Durchsetzung von Session-Freigaben, der lokale und zentrale Secret-Store mit Envelope-Encryption, Git-Provider-Verbindungen und ein Bedrohungsmodell (STRIDE-light) für das Gesamtsystem.

Grundlage sind ADR-0011 (Auth: Token lokal, OIDC/Device-Pairing/PATs zentral, kein SCIM) und ADR-0024 (Secrets: Keychain bzw. Envelope-Encryption, Platzhalter `bt_cred_*`, Audit), ergänzt durch ADR-0003 (gleicher Code lokal/zentral) und ADR-0007 (Proxy-Injection, PRX-006 in 04-sandbox.md). Die fachliche Seite der Session-Freigaben (Rollenmatrix, Share-Dialog) steht in COL-001/COL-002 (siehe 07-sessions-collaboration.md); hier wird ihre Durchsetzung spezifiziert. Meilensteine: lokaler Modus **M0**, Secrets/Audit/Redaction lokal **M2**, Team-Funktionen **M4**.

## Konzepte & Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Principal** | Handelnde Identität: `user`, `service_account`, `device` (im Namen eines Users), `host`, `runner`, `system`. |
| **Lokaler Modus** | `auth.mode: local` — kein Login; Zugriff nur über Loopback/Unix-Socket mit Token aus Datei; Single-User (`usr_local`). |
| **Zentraler Modus** | `auth.mode: oidc` — Login über OIDC-Provider; Multi-User, Orgs, Teams. |
| **Identität** | Externe Kennung `(issuer, sub)` eines OIDC-Providers; ein User kann mehrere haben. E-Mail ist nur Attribut. |
| **Org / Team** | Mandant bzw. Gruppe innerhalb einer Org; Rollen `owner`, `admin`, `member`, `viewer` je Ebene. |
| **Share** | Session-Freigabe mit Rolle `view`, `comment_approve` oder `drive` (COL-001, siehe 07-sessions-collaboration.md). |
| **Device-Token** | Langlebiges, widerrufbares Token eines gekoppelten Geräts (`bt_dev_…`), im OS-Keychain des Geräts. |
| **Runner-Token** | Kurzlebiges, session-gebundenes Token (`bt_run_…`), mit dem ein Runner seinen Tunnel öffnet. |
| **PAT** | Personal Access Token eines Users (`bt_pat_…`) mit Scopes und Ablaufdatum. |
| **Service-Account (SA)** | Nicht-menschlicher Principal einer Org mit Rollen und Tokens (`bt_sat_…`). |
| **Scope** | Rechte-Teilmenge eines Tokens; effektives Recht = Scope ∩ Rolle des Principals. |
| **SecretRef** | Adresse eines Secrets: `secret://<scope>/<provider>/<account>[#name]`, z. B. `secret://user/github.com/ifahrentholz`. |
| **Bindung** | Tupel `(org, user\|team, provider, account)`, an das ein Secret kryptografisch (AAD) gebunden ist. |
| **DEK / KEK** | Data-Encryption-Key pro Secret-Version / Key-Encryption-Key (Master-Key) zum Wrappen der DEKs. |
| **Secret-Lease** | Zeitlich begrenzte Auslieferung eines Secret-Werts an den Proxy eines Runners (nur im Speicher). |
| **Audit-Log** | Append-only-Protokoll sicherheitsrelevanter Aktionen mit Hash-Kette; enthält nie Secret-Werte. |

## Design

### Token-Formate

Alle beton-Tokens: `<prefix><40 Zeichen base62><6 Zeichen base62 CRC32>`; Prüfsumme erlaubt Secret-Scannern Erkennung ohne DB. Server speichern nur `SHA-256(token)`; Tokens werden genau einmal angezeigt. Tokens erscheinen nie in URLs (Ausnahme: Einmal-Codes mit ≤ 120 s Laufzeit).

| Prefix | Art | Laufzeit | Ablage beim Client |
| --- | --- | --- | --- |
| `bt_loc_` | Lokales Daemon-Token | bis Rotation | `~/.beton/auth/local.token` (0600) |
| `bt_dev_` | Device-Token (Host, CLI, Desktop, Handy) | unbegrenzt, Rotation alle 30 Tage | OS-Keychain |
| `bt_run_` | Runner-Token | 15 min, über Tunnel erneuerbar | nur Speicher |
| `bt_pat_` | Personal Access Token | Pflicht-Ablauf, max. 365 Tage (Default 90) | Sache des Users |
| `bt_sat_` | Service-Account-Token | Pflicht-Ablauf, max. 365 Tage | Sache des Betreibers |
| `bt_cred_` | **Kein Token:** Credential-Platzhalter des Proxys (PRX-006, siehe 04-sandbox.md) | Session | Sandbox-Env |

### Konfiguration (zentral)

```yaml
auth:
  mode: oidc                       # local | oidc
  public_url: https://beton.example.com
  default_org_role: member         # Rolle neu angelegter User
  bootstrap_owners: [alice@example.com]
  session: { idle_timeout: 14d, absolute_timeout: 30d }
  ws_allowed_origins: ["https://beton.example.com"]   # + Tauri-Origins implizit
  providers:
    - id: entra
      kind: oidc
      issuer: https://login.microsoftonline.com/<tenant-id>/v2.0
      client_id: 0000-…
      client_secret: { env: BETON_OIDC_ENTRA_SECRET }
      allowed_tenants: [<tenant-id>]
      trust_email: true            # Entra liefert kein email_verified
      allowed_domains: [example.com]
    - id: google
      kind: oidc
      issuer: https://accounts.google.com
      client_id: …
      client_secret: { file: /run/secrets/google_oidc }
      allowed_domains: [example.com]
    - id: github
      kind: github                 # OAuth2 + /user, /user/emails (GitHub ist kein OIDC-Provider)
      client_id: …
      client_secret: { env: BETON_GITHUB_LOGIN_SECRET }
      allowed_orgs: [example-org]
    - id: keycloak
      kind: oidc
      issuer: https://sso.example.com/realms/dev
      client_id: beton
      client_secret: { env: BETON_KC_SECRET }
secrets:
  master_keys: [{ id: k2026a, file: /run/secrets/beton_master_k2026a }]
  active_key: k2026a
```

### Rollenmatrix Org/Team

| Aktion | Owner | Admin | Member | Viewer |
| --- | :-: | :-: | :-: | :-: |
| Org löschen, Owner übertragen | ✔ | – | – | – |
| Auth-Provider, Master-Key-Rotation, Server-Einstellungen | ✔ | ✔ | – | – |
| Mitglieder, Teams, Service-Accounts verwalten | ✔ | ✔ | – | – |
| Org-Policies / Org-Secrets schreiben | ✔ | ✔ | – | – |
| Audit-Log der Org lesen | ✔ | ✔ | nur eigene Einträge | – |
| Sessions anlegen, Hosts koppeln, eigene PATs/Secrets | ✔ | ✔ | ✔ | – |
| Team-Ressourcen (Team-Policies/-Secrets/-Projekte) verwalten | Team-Owner/-Admin | Team-Owner/-Admin | – | – |
| Geteilte Sessions sehen (gemäß Share-Rolle) | ✔ | ✔ | ✔ | ✔ (max. `view`) |
| Session-**Inhalte** fremder, nicht geteilter Sessions | – *(Annahme)* | – *(Annahme)* | – | – |
| Metadaten aller Sessions (Owner, Titel, Kosten, Status), Stop/Übertragung | ✔ | ✔ | – | – |

Org-Rolle und Team-Rollen werden vereinigt; Viewer-Rolle deckelt jede Session-Freigabe auf `view`.

### Autorisierungsprüfung

```
Request ──► authenticate() ─► Principal{kind, id, org, scopes, device?}
        ──► authorize(principal, Action, Resource)
              1. Scope erlaubt Action?                      nein → 403 insufficient_scope
              2. Org-/Team-Rolle erlaubt Action?            nein → weiter mit 3 (nur Session-Ressourcen)
              3. Session: Owner? sonst max(Share user, Share team…) ≥ benötigte Rolle?
              4. Viewer-Deckel, sharing.mode (on|view_only|off)
        ──► Handler                                           Default: deny
```

Jede Route deklariert `Action` per Attribut-Makro; ein Test über die generierte Routentabelle schlägt fehl, wenn eine Route ohne Deklaration existiert (AUTH-015).

### Bedrohungsmodell (STRIDE-light, Gesamtsystem)

**Schutzgüter:** Quellcode/Workspaces; Code-Ausführung auf Hosts; Vendor-Subscription-Tokens (bleiben im Vendor-CLI); Secrets (Git-Tokens, API-Keys); Session-Inhalte (Prompts, Transkripte, Diffs); Policies und Budgets; Integrität von Audit-Log und Event-Log.
**Akteure:** (A1) bösartige Website im Browser des Users; (A2) anderer lokaler OS-User; (A3) prompt-injizierter Agent / bösartiges Repo; (A4) Insider mit Member-Rolle; (A5) Netzwerkangreifer zwischen Client/Host und Server; (A6) kompromittierter Host/Runner; (A7) gestohlenes Gerät/Token; (A8) bösartiges Plugin.
**Vertrauensgrenzen:** Browser ↔ Server · Client/Gerät ↔ Server (Netz) · Host/Runner ↔ Server (Tunnel) · Runner (Stufe 0) ↔ Sandbox (Stufe 1/2) · Proxy ↔ Internet · Server ↔ DB/Blob-Store · Plugin-Prozess ↔ beton.

| STRIDE | Bedrohung | Gegenmaßnahme | Feature |
| --- | --- | --- | --- |
| **S** | Website ruft lokalen Daemon per DNS-Rebinding/CSRF auf (A1) | Host-Allowlist, Origin-Prüfung, kein CORS, Token statt Ambient-Auth | AUTH-002, AUTH-003 |
| **S** | Anderer lokaler User nutzt den Daemon (A2) | Token-Datei 0600, Unix-Socket 0600, Rechteprüfung beim Start | AUTH-001 |
| **S** | Phishing des Device-Codes (Angreifer lässt Opfer fremdes Gerät freigeben) | Freigabeseite zeigt Gerätename, Art, IP, Scopes; Codes 10 min gültig; Rate-Limit | AUTH-008 |
| **S** | Gestohlenes Device-/PAT-Token (A7) | Widerruf mit sofortiger Trennung, Ablauf, Rotation, `last_seen`-Anzeige | AUTH-010, AUTH-012 |
| **T** | Manipulation von Events/Policies im Transit (A5) | TLS (rustls), WSS; Tunnel nur ausgehend mit Token | SEC-014, PROTO-015 |
| **T** | Ciphertext-Vertauschung zwischen Secrets in der DB | AES-GCM mit AAD = Bindung + ID + Version | SEC-005 |
| **T** | Agent schreibt in `~/.beton`, Git-Hooks, Shell-RCs (A3) | Sandbox-Masken und Schreib-Grants | SBX-003, SBX-004 |
| **R** | Abstreiten von Approvals, Secret-Nutzung, Rechteänderungen | Audit-Log mit Hash-Kette, `actor` in jedem Event | SEC-012 |
| **I** | Secrets im Agent-Kontext, in Logs oder Transkripten (A3) | Platzhalter + Proxy-Injection, Redaction vor Persistenz | SEC-013, PRX-006 |
| **I** | Insider liest fremde Sessions (A4) | Deny-by-default-Autorisierung, Shares, Admin ohne Inhaltszugriff | AUTH-014, AUTH-015 |
| **I** | Diebstahl der DB / Backups | Envelope-Encryption, Master-Key getrennt von DB | SEC-005, SEC-006 |
| **D** | Brute-Force auf Login, Pairing-Codes, Token-Endpunkte | Rate-Limits, Lockout pro User, Request-Größenlimits | SEC-014, AUTH-008 |
| **E** | `drive`-Share = Code-Ausführung auf Owner-Host (A4) | Warnhinweis, Sandbox, Policies, Credentials stets des Owners | AUTH-015, COL-002 |
| **E** | Kompromittierter Runner fordert fremde Secrets an (A6) | Leases nur für Bindungen des Session-Owners, Host-Label-Restriktion | SEC-008, AUTH-011 |
| **E** | Plugin missbraucht beton-Rechte (A8) | Out-of-Process, deklarierte Berechtigungen | PLG-008, PLG-009 |

**Restrisiken (bewusst akzeptiert):** Der zentrale Server ist vertrauenswürdiger Koordinator — wer ihn kompromittiert, kann Sessions auf verbundenen Hosts steuern (Hosts können per `host.accept_sessions_from` auf eigene User beschränkt werden). Root auf einem Host umgeht jede lokale Kontrolle. Ohne SCIM wird Deprovisionierung erst beim nächsten Login bzw. durch Admin-Aktion wirksam. Verlust des Master-Keys macht zentrale Secrets unwiederbringlich.

## Features

### AUTH-001 — Lokaler Modus: Loopback-Bindung & Token-Datei
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Im lokalen Modus bindet der Daemon ausschließlich an `127.0.0.1` und `::1` (konfigurierbarer Port) sowie auf Unix an einen Socket `~/.beton/run/beton.sock` (0600). Jeder Request braucht das lokale Token aus `~/.beton/auth/local.token` (256 bit, 0600, Verzeichnis 0700), das CLI, TUI und Desktop lesen. Andere lokale OS-User haben weder Datei- noch Socket-Zugriff.
- **Details:** Start bricht ab, wenn Datei/Verzeichnis Gruppen-/Fremdrechte haben oder einem anderen UID gehören (Windows: ACL nur aktueller User + SYSTEM). `beton auth rotate-local` erneuert das Token; laufende Clients reconnecten mit neuem Token. Nicht-Loopback-Bindung ist im lokalen Modus nur über AUTH-009 möglich.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Request ohne bzw. mit falschem Token an `127.0.0.1:<port>` erhält 401; mit Token aus der Datei 200.
  - [ ] AC2 — Ist `local.token` mit Modus 0644 angelegt, verweigert der Daemon den Start mit klarer Meldung.
  - [ ] AC3 — Ein zweiter OS-User auf derselben Maschine kann weder Token-Datei noch Socket öffnen (Linux/macOS-Integrationstest).
  - [ ] AC4 — `server.listen: 0.0.0.0:<port>` im lokalen Modus ohne Pairing-/TLS-Konfiguration wird beim Start abgelehnt.
- **Abhängigkeiten:** —
- **Referenz:** ADR-0011; Omnigent `local`-User auf Loopback (3.10)

### AUTH-002 — Schutz gegen DNS-Rebinding & CSRF
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Der Server (lokal wie zentral) akzeptiert nur Requests, deren `Host`-Header auf der Allowlist steht (lokal: `127.0.0.1:<port>`, `[::1]:<port>`, `localhost:<port>`, plus `server.allowed_hosts`; zentral: Host von `public_url`). Zustandsändernde Requests mit Cookie-Authentisierung müssen `Origin` aus der Allowlist und `Sec-Fetch-Site` `same-origin` oder `none` tragen. Es werden keine CORS-Header gesendet (außer konfiguriert), und Private-Network-Access-Preflights werden nicht freigegeben.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Request mit `Host: evil.test` (DNS-Rebinding-Szenario) auf den lokalen Port erhält 403 `host_not_allowed`, auch mit gültigem Cookie.
  - [ ] AC2 — Ein `POST` mit gültigem Cookie und `Origin: https://evil.test` erhält 403; derselbe Request mit `Authorization: Bearer` ohne Cookie wird nach Token-Prüfung verarbeitet.
  - [ ] AC3 — Eine Antwort auf einen CORS-Preflight von fremder Origin enthält kein `Access-Control-Allow-Origin` und kein `Access-Control-Allow-Private-Network`.
  - [ ] AC4 — Playwright-Test: Eine Seite auf `http://evil.localhost:<x>` kann per `fetch`/Form-POST keine Session im lokalen Daemon starten.
- **Abhängigkeiten:** AUTH-001

### AUTH-003 — WebSocket-Origin-Allowlist
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** WebSocket-Handshakes werden vor dem Upgrade gegen `auth.ws_allowed_origins` geprüft (Schutz gegen Cross-Site-WebSocket-Hijacking). Default: Origin der `public_url` bzw. lokale Loopback-Origins sowie die Tauri-Origins (`tauri://localhost`, `http://tauri.localhost`). Wildcards nur als Subdomain-Präfix (`https://*.example.com`). Handshakes ohne `Origin` sind nur mit `Authorization`-Header (nicht per Cookie) zulässig.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Handshake mit `Origin: https://evil.test` und gültigem Cookie wird mit 403 abgelehnt, bevor ein WS-Frame fließt.
  - [ ] AC2 — Handshake ohne `Origin`, mit Cookie und ohne `Authorization` wird abgelehnt; mit gültigem Bearer-Token akzeptiert (SDK-Fall).
  - [ ] AC3 — `https://*.example.com` erlaubt `https://a.example.com`, nicht `https://a.example.com.evil.test` und nicht `http://a.example.com`.
- **Abhängigkeiten:** AUTH-002, PROTO-004 (siehe 06-data-sync-protocol.md)
- **Referenz:** Omnigent `WebSocketOriginMiddleware`, `OMNIGENT_WS_ALLOWED_ORIGINS`

### AUTH-004 — Lokale Browser-Anmeldung per Einmal-Link
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Damit die Web-UI im normalen Browser ohne Token-Kopieren funktioniert, erzeugt `beton open [session]` (bzw. `beton run` beim ersten Start) über die authentisierte lokale API einen Einmal-Code (128 bit, 60 s, single-use) und öffnet `http://127.0.0.1:<port>/auth/local/redeem?code=…`. Der Server tauscht ihn gegen ein Session-Cookie (`HttpOnly`, `SameSite=Strict`) und leitet ohne Code in der URL weiter.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein zweites Einlösen desselben Codes oder ein Einlösen nach 60 s schlägt mit 401 fehl.
  - [ ] AC2 — Nach Einlösen enthält die Browser-History keine URL mit gültigem Code (Redirect ersetzt den Eintrag).
  - [ ] AC3 — Codes lassen sich nur mit lokalem Token erzeugen; der Endpunkt ist über eine Nicht-Loopback-Bindung nicht erreichbar.
- **Abhängigkeiten:** AUTH-001, AUTH-002

### AUTH-005 — OIDC-Login mit Just-in-Time-User-Anlage
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Im zentralen Modus melden sich User über konfigurierte Provider an (Entra ID, Google, GitHub, Keycloak; generisches OIDC). Authorization-Code-Flow mit PKCE (S256), `state` und `nonce`; das ID-Token wird über JWKS (Signatur, `iss`, `aud`, `exp`, `nonce`) geprüft. Beim ersten Login wird der User angelegt (kein SCIM), sofern Domain-, Tenant- bzw. Org-Regeln passen; Identitätsschlüssel ist `(issuer, sub)`, nie die E-Mail.
- **Details:** Zulassung: `allowed_domains` gegen verifizierte E-Mail (`email_verified=true` oder `trust_email: true`), `allowed_tenants` (Entra `tid`), `allowed_orgs` (GitHub-Mitgliedschaft via API, Scope `read:org`). Keine automatische Verknüpfung von Identitäten über gleiche E-Mail; ein eingeloggter User kann weitere Identitäten im Profil hinzufügen. Deaktivierte User werden beim Login abgewiesen. CLI-Login zentral über Device-Flow (AUTH-008, `kind: client`). OIDC ist nur im zentralen Modus nötig; der lokale Modus (AUTH-001) braucht keinen IdP und keinen externen Dienst. Ein selbst betriebener IdP (z. B. Keycloak) erlaubt auch den zentralen Betrieb ohne externe Server (ADR-0033).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Erster Login eines Users mit verifizierter `@example.com`-Adresse legt genau einen User mit `default_org_role` an; zweiter Login legt keinen weiteren an.
  - [ ] AC2 — Login mit `@other.test`, mit `email_verified=false` (ohne `trust_email`) oder aus fremdem Entra-Tenant wird abgelehnt und auditiert.
  - [ ] AC3 — Callback mit manipuliertem `state`, falschem `nonce` oder ID-Token mit fremdem `aud` wird abgelehnt (Tests gegen Mock-IdP).
  - [ ] AC4 — Ändert sich die E-Mail beim IdP, bleibt der User derselbe (gleiche `(iss, sub)`), die E-Mail wird aktualisiert.
  - [ ] AC5 — Ein deaktivierter User erhält beim Login 403 `user_disabled`.
- **Abhängigkeiten:** AUTH-007, DATA-001, DATA-004 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0011; Omnigent OIDC + `allowed_domains` (3.10)

### AUTH-006 — Org-Bootstrap & erster Owner
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Ein frisch gestarteter zentraler Server hat eine Default-Org. Owner wird, wer in `auth.bootstrap_owners` steht und sich zuerst anmeldet; alternativ druckt `beton serve` beim ersten Start einen Einmal-Setup-Code (stdout, 24 h gültig), mit dem der erste eingeloggte User Owner wird. Danach ist der Bootstrap-Pfad geschlossen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ohne Bootstrap-Konfiguration ist der erste User nach Login `member`, bis er den Setup-Code einlöst.
  - [ ] AC2 — Nach Vergabe des ersten Owners liefert der Setup-Endpunkt 410 und ein erneut gedruckter Code existiert nicht.
  - [ ] AC3 — Die Org kann nie ohne Owner sein: Entfernen/Herabstufen des letzten Owners wird mit 409 abgelehnt.
- **Abhängigkeiten:** AUTH-005, AUTH-014

### AUTH-007 — Web-Sessions & Cookies
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Browser-Logins erhalten ein opakes Session-Cookie `__Host-beton_session` (`Secure`, `HttpOnly`, `SameSite=Lax`, `Path=/`), dessen SHA-256 serverseitig mit User, Gerät/User-Agent, Erstellzeit und letzter Nutzung gespeichert ist. Idle-Timeout 14 Tage, absolutes Timeout 30 Tage *(Annahme)*; Session-ID-Rotation bei Login und Rechteänderung. Keine JWTs für Browser-Sessions, damit Widerruf sofort wirkt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Logout invalidiert das Cookie serverseitig; ein Replay des alten Cookie-Werts erhält 401.
  - [ ] AC2 — „Alle Sitzungen abmelden“ im Profil trennt bestehende WS-Verbindungen des Users innerhalb von 5 s (Close-Code `4401`).
  - [ ] AC3 — Nach 14 Tagen Inaktivität bzw. 30 Tagen absolut ist eine erneute Anmeldung nötig (Test mit simulierter Uhr).
  - [ ] AC4 — Das Cookie wird nie über unverschlüsselte Verbindungen gesetzt; der Server verweigert den zentralen Modus ohne HTTPS-`public_url` (Ausnahme: `--insecure-dev`).
- **Abhängigkeiten:** SEC-014

### AUTH-008 — Device-Pairing per Code (geräteinitiiert)
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Hosts (`beton host`), CLI/TUI, Desktop-App und Runner-Images auf Headless-Systemen koppeln sich RFC-8628-ähnlich: Das Gerät fordert einen Code an, zeigt `user_code` und `verification_uri_complete` (auch als QR im Terminal), der eingeloggte User bestätigt im Browser, das Gerät pollt und erhält ein Device-Token, das es im OS-Keychain ablegt.
- **Details:** `POST /v1/auth/device/code {kind: host|client|runner_image, name, platform, requested_scopes}` → `{device_code (256 bit), user_code "BCDF-GHJK" (8 Zeichen aus 20er-Alphabet), verification_uri, verification_uri_complete, interval: 5, expires_in: 600}`; `POST /v1/auth/device/token` → `authorization_pending | slow_down | access_denied | expired_token | {access_token: bt_dev_…}`. Freigabeseite zeigt Art, Name, Plattform, anfragende IP und Scopes; Hosts benötigen Rolle ≥ Member. Max. 5 Fehleingaben von `user_code` pro User in 10 min.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton login https://beton.example.com` und `beton host --server …` durchlaufen den Flow und speichern das Token im Keychain, nicht in einer Klartextdatei.
  - [ ] AC2 — Pollen schneller als `interval` liefert `slow_down`; nach 600 s `expired_token`.
  - [ ] AC3 — Die sechste falsche `user_code`-Eingabe in 10 min wird mit 429 abgelehnt, auch wenn der Code korrekt wäre.
  - [ ] AC4 — Ein Viewer kann keinen Host freigeben (403); die Freigabe eines Hosts erzeugt ein Audit-Event `device.paired`.
- **Abhängigkeiten:** AUTH-007, SEC-012
- **Referenz:** ADR-0011; Omnigent Device Grant (3.10)

### AUTH-009 — Pairing per QR & Freigabe des lokalen Servers fürs Handy
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Für Handys/PWA startet der Flow in einer eingeloggten UI („Gerät koppeln“): Der Server erzeugt ein Pairing-Ticket (128 bit, 120 s, single-use) und zeigt einen QR mit `https://<host>/pair#t=<ticket>` (Fragment, damit es nicht in Server-Logs landet). Das Handy löst es mit Gerätename ein; erst nach Bestätigung auf dem auslösenden Gerät entsteht das Device-Token. Damit lässt sich auch ein **lokaler** Server über Tailscale/LAN fürs Handy freigeben: Nicht-Loopback-Bindung ist dann nur mit TLS (`tls: tailscale` oder Zertifikat/Schlüssel-Dateien) erlaubt, und über diese Bindung werden nur Device-Tokens/-Cookies akzeptiert, nie das lokale Token. Tailscale ist optional; die Freigabe im LAN mit eigenen Zertifikat-/Schlüssel-Dateien kommt ohne externen Dienst aus (ADR-0033).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein eingelöstes, aber auf dem Desktop nicht bestätigtes Ticket führt nach 120 s zu keinem Token.
  - [ ] AC2 — Lokaler Modus mit `server.listen` auf Tailscale-IP ohne TLS startet nicht.
  - [ ] AC3 — Über die Tailscale-Bindung wird ein Request mit `bt_loc_`-Token mit 401 abgelehnt; mit gekoppeltem Device-Token akzeptiert.
  - [ ] AC4 — Default-Scopes eines Handys: `sessions:read`, `sessions:write`, `sessions:approve`; Admin-Scopes sind nicht wählbar.
- **Abhängigkeiten:** AUTH-001, AUTH-008, WEB-013 (siehe 08-clients.md)

### AUTH-010 — Geräteverwaltung & Widerruf
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** User sehen ihre gekoppelten Geräte und Hosts (Name, Art, Plattform, erstellt, zuletzt gesehen, letzte IP, Scopes) und widerrufen sie; Admins können Geräte aller User widerrufen. Widerruf wirkt sofort auf offene Verbindungen. Device-Tokens rotieren automatisch alle 30 Tage (altes Token 24 h gültig).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach Widerruf eines Hosts schließt der Server dessen Tunnel und alle zugehörigen Runner-Tunnel innerhalb von 5 s; betroffene Sessions gehen auf `stopped` mit Grund `host_revoked`.
  - [ ] AC2 — Ein widerrufenes Token erhält bei jedem Endpunkt 401 `token_revoked`.
  - [ ] AC3 — Nach Rotation funktioniert das alte Token noch 24 h, danach 401; das neue liegt im Keychain des Geräts.
  - [ ] AC4 — `GET /v1/me/devices` zeigt `last_seen_at` mit max. 5 min Verzögerung.
- **Abhängigkeiten:** AUTH-008, SEC-012

### AUTH-011 — Host- & Runner-Credentials
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Hosts authentisieren den Tunnel mit ihrem Device-Token (Scope `hosts:connect`). Für jede Session mintet der Server ein Runner-Token (`bt_run_`, 15 min, an `session_id` und `host_id` gebunden), das der Host dem Runner per FD/stdin übergibt (nie Env/argv); der Runner erneuert es über den Tunnel. Lokal übergibt der Daemon Runner-Tokens auf dieselbe Weise. Hosts können per `host.accept_sessions_from` einschränken, wessen Sessions sie ausführen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Runner-Token für Session A öffnet keinen Tunnel für Session B (Close `4403`).
  - [ ] AC2 — Ein nicht erneuertes Runner-Token läuft nach 15 min ab; der Tunnel wird geschlossen, der Runner reconnectet nach Erneuerung über den Host.
  - [ ] AC3 — `ps`/`/proc/*/environ` des Runners enthalten kein Runner-Token.
  - [ ] AC4 — Ein Host mit `accept_sessions_from: [usr_alice]` lehnt einen Launch für eine Session von Bob mit `launch_rejected` ab.
- **Abhängigkeiten:** AUTH-008, PROTO-015 (siehe 06-data-sync-protocol.md), RUN-004 (siehe 10-runners-extensibility.md)

### AUTH-012 — Personal Access Tokens mit Scopes
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** User erzeugen PATs für API/SDK/Skripte mit Name, Scopes und Pflicht-Ablauf (Default 90, max. 365 Tage). Effektive Rechte sind Scope ∩ aktuelle Rechte des Users; verliert der User Rechte, verliert sie das Token sofort. PATs funktionieren auch im lokalen Modus (für Skripte gegen `localhost`).
- **Details:** Scopes: `sessions:read`, `sessions:write` (anlegen, Input, Queue, Interrupt im Rahmen der Share-Rolle), `sessions:approve`, `agents:read`, `agents:write`, `policies:read`, `policies:write`, `secrets:read_meta`, `secrets:write`, `usage:read`, `hosts:manage`, `schedules:write`, `webhooks:trigger`, `audit:read`, `admin`. Secret-**Werte** sind über keinen Scope lesbar. Gerätespezifische Scopes (nicht für PATs wählbar): `hosts:connect` (Host-Tunnel), `nodes:sync` (Sync eines lokalen Knotens, SYNC-001 in 06-data-sync-protocol.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein PAT mit nur `sessions:read` erhält bei `POST /v1/sessions` 403 `insufficient_scope`.
  - [ ] AC2 — Ein PAT mit `admin`-Scope eines Users, der zum Member herabgestuft wurde, erhält bei Admin-Endpunkten 403.
  - [ ] AC3 — PAT-Erstellung ohne Ablaufdatum oder mit > 365 Tagen wird mit 422 abgelehnt.
  - [ ] AC4 — Das Token ist nach Erstellung nicht erneut abrufbar; die DB enthält nur den Hash.
- **Abhängigkeiten:** AUTH-015, SEC-012

### AUTH-013 — Service-Accounts
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Admins legen Service-Accounts als eigene Principals der Org an (Name, Beschreibung, Org-Rolle, Team-Mitgliedschaften) und erzeugen dafür Tokens (`bt_sat_`, Scopes, Pflicht-Ablauf). Service-Accounts besitzen von ihnen angelegte Sessions, nutzen Secrets ihrer Bindung `(org, team, …)` und sind Grundlage für Webhook/API-getriggerte Async-Agents (ASY-007, siehe 02-agents.md). Sie können sich nicht interaktiv anmelden und keine Geräte koppeln.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein SA-Token startet über die API eine Session, deren Owner der SA ist; Sessions sind per Team-Share für Menschen sichtbar.
  - [ ] AC2 — SA-Tokens werden an Web-Login- und Pairing-Endpunkten abgelehnt.
  - [ ] AC3 — Deaktivieren des SA invalidiert alle seine Tokens sofort und stoppt keine laufenden Sessions ohne Bestätigung (Dialog bietet „Stoppen“ an).
- **Abhängigkeiten:** AUTH-012, AUTH-014

### AUTH-014 — Rollen Org/Team
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Rollen `owner`, `admin`, `member`, `viewer` existieren pro Org und pro Team; ihre Rechte folgen der Rollenmatrix (Design). Team-Rollen gelten für Team-Ressourcen (Team-Projekte, -Policies, -Secrets, Team-Shares). Policy-Hierarchie Org → Team → User nutzt diese Zugehörigkeit (POL-008, siehe 03-policies.md). Rollenänderungen wirken ohne Re-Login.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Member kann keine Org-Policy schreiben (403), ein Team-Admin kann Team-Policies seines Teams schreiben, nicht die eines anderen Teams.
  - [ ] AC2 — Herabstufung Admin → Member wirkt auf die nächste Anfrage derselben Browser-Session (kein Cache > 5 s).
  - [ ] AC3 — Ein Org-Admin kann Metadaten einer nicht geteilten Session sehen und sie stoppen, erhält aber auf `GET /v1/sessions/{id}/events` 403.
  - [ ] AC4 — Jede Rollenänderung erzeugt ein Audit-Event `role.changed {principal, scope, from, to, by}`.
- **Abhängigkeiten:** AUTH-005, SEC-012

### AUTH-015 — Autorisierungsdurchsetzung & Session-Freigaben
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Eine zentrale Funktion `authorize(principal, action, resource)` prüft jede REST-Route, jedes WS-Kommando und jede Kanalöffnung (Terminal, Browser, Audio) deny-by-default. Für Sessions gilt: Owner hat alle Rechte; sonst die stärkste Rolle aus direkten und Team-Shares (`view` < `comment_approve` < `drive`), gedeckelt durch Viewer-Rolle und `sharing.mode`. Credentials einer Session sind immer die des Owners — `drive`-Nutzer handeln mit Owner-Credentials (Warnhinweis, COL-002 in 07-sessions-collaboration.md). AUTH-015 ist Owner der Durchsetzung; Rollensemantik, Share-API und Rollenmatrix definiert COL-001.
- **Details:** Routen deklarieren ihre Action per Makro (`#[authz(SessionDrive)]`); fehlende Deklaration bricht den Build-Test. Widerruf eines Shares schließt betroffene Subscriptions (Close `4403`) bzw. Kanäle.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein generierter Test über die Routentabelle schlägt fehl, wenn eine Route keine `authz`-Deklaration hat.
  - [ ] AC2 — Ein `view`-User kann keinen Terminal-Kanal öffnen und keine Approval auflösen (je 403); `comment_approve` kann Approvals auflösen, aber keinen Input senden.
  - [ ] AC3 — Widerruf eines Team-Shares trennt alle Team-Mitglieder ohne andere Freigabe innerhalb von 2 s.
  - [ ] AC4 — Property-basierter Test: Für zufällige Kombinationen aus Rollen, Shares und Scopes stimmt `authorize` mit einer Referenztabelle überein.
- **Abhängigkeiten:** AUTH-012, AUTH-014, COL-001 (siehe 07-sessions-collaboration.md)

### AUTH-016 — Offboarding von Mitgliedern
- **Meilenstein:** M4 · **Priorität:** Should
- **Beschreibung:** Entfernt oder deaktiviert ein Admin einen User, werden transaktional Web-Sessions, PATs, Device-Tokens, Runner-Tokens laufender Sessions und dem User erteilte Shares widerrufen; seine Sessions werden gestoppt und wahlweise an einen anderen User übertragen oder archiviert; persönliche Secrets werden durch Löschen ihrer DEKs unlesbar gemacht (crypto-shredding).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach Entfernen hat der User keinen gültigen Token mehr (alle Endpunkte 401) und offene Verbindungen sind innerhalb von 5 s getrennt.
  - [ ] AC2 — Übertragene Sessions haben den neuen Owner; die alten Shares bleiben erhalten, der Vorgang ist auditiert.
  - [ ] AC3 — Persönliche Secrets des Users sind nach dem Vorgang nicht mehr entschlüsselbar (Test: DEK-Spalte leer, Lease-Anfrage scheitert).
- **Abhängigkeiten:** AUTH-010, AUTH-012, SEC-005

### AUTH-017 — Passkey-Login (WebAuthn)
- **Meilenstein:** M4 · **Priorität:** Could
- **Beschreibung:** Nach einem OIDC-Login können User Passkeys registrieren (`webauthn-rs`, User-Verification erforderlich) und sich damit ohne IdP-Roundtrip anmelden. Der IdP bleibt Quelle der Wahrheit: Passkey-Login prüft den User-Status, und spätestens alle 30 Tage ist ein OIDC-Login nötig.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Passkey-Login eines deaktivierten Users wird abgelehnt.
  - [ ] AC2 — 30 Tage nach dem letzten OIDC-Login verlangt der Passkey-Login zusätzlich einen OIDC-Login.
  - [ ] AC3 — Registrierung ohne User-Verification (`uv=false`) wird abgelehnt.
- **Abhängigkeiten:** AUTH-005, AUTH-007

### SEC-001 — Secret-Store-Abstraktion & SecretRefs
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `beton-secrets` definiert einen `SecretStore`-Trait (`put`, `get_for_use`, `delete`, `list_meta`) mit Backends Keychain, verschlüsselte Datei (lokal) und Envelope-DB (zentral). Secrets werden überall per SecretRef adressiert; Metadaten (Typ, Provider, Account, Zeitstempel, zuletzt genutzt) liegen in der DB, Werte nur im Backend. Der Wert ist nur über `get_for_use(ref, purpose)` erhältlich, das ausschließlich Proxy, Direkt-API-Harness und Git-Provider-Client aufrufen dürfen und das jeden Abruf auditiert.
- **Details:** SecretRef: `secret://<user|team:<slug>|org>/<provider>/<account>[#name]`; Provider z. B. `github.com`, `gitlab.example.com`, `anthropic`, `openai`, `npm`, `custom:<name>`. Lokal: `org=local`, `user=usr_local`. Typen: `token`, `basic` (User + Passwort), `oauth` (Access + Refresh + Ablauf).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `list_meta` liefert nie Werte; ein Unit-Test stellt sicher, dass `SecretValue` weder `Debug` noch `Serialize` mit Klartext implementiert (Ausgabe `***`).
  - [ ] AC2 — Ein Aufruf von `get_for_use` aus einem nicht freigegebenen Modul wird durch Sichtbarkeit (`pub(crate)`-Fassade + Capability-Typ) zur Compile-Zeit verhindert.
  - [ ] AC3 — Jeder `get_for_use`-Aufruf erzeugt einen Audit-Eintrag mit `purpose`, Session und Host.
  - [ ] AC4 — Werte werden nach Gebrauch genullt (`zeroize`), getestet über einen Drop-Hook im Test-Build.
- **Abhängigkeiten:** SEC-012
- **Referenz:** ADR-0024

### SEC-002 — Lokaler Store: OS-Keychain
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Lokal speichert beton Secret-Werte im OS-Keychain über die `keyring`-Crate (macOS Keychain, Windows Credential Manager, Linux Secret Service), Service-Name `beton`, Account = SecretRef. Ist kein Keychain verfügbar (Headless-Linux ohne Secret Service), wird der Datei-Fallback (SEC-003) genutzt — nie Klartext.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach `beton secrets add github` liegt der Wert im Keychain und in keiner Datei unter `~/.beton` (Grep-Test).
  - [ ] AC2 — Auf Linux ohne D-Bus-Session meldet `beton doctor` „Keychain nicht verfügbar → Datei-Store“ und SEC-003 greift.
  - [ ] AC3 — Löschen über `beton secrets rm` entfernt den Keychain-Eintrag und die Metadaten.
- **Abhängigkeiten:** SEC-001

### SEC-003 — Lokaler Fallback: verschlüsselte Datei (Argon2id)
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Der Fallback-Store `~/.beton/secrets/store.v1` (0600) ist eine mit AES-256-GCM verschlüsselte Datei; der Schlüssel wird per Argon2id aus einer Passphrase abgeleitet (m = 64 MiB, t = 3, p = 1, 16-Byte-Salt, Parameter im Header). Jede Änderung schreibt atomar (Temp-Datei, fsync, rename) mit neuem Nonce. Entsperren interaktiv (`beton secrets unlock`) oder über `BETON_SECRETS_PASSPHRASE_FILE`; gesperrt → Sessions, die Secrets brauchen, scheitern mit `secrets_locked`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Falsche Passphrase liefert `secrets_locked` ohne Hinweis, welcher Teil falsch war; die Datei bleibt unverändert.
  - [ ] AC2 — Ein einzelnes geflipptes Bit in der Datei führt zu einem Integritätsfehler statt zu falschen Werten.
  - [ ] AC3 — Passphrase-Wechsel verschlüsselt neu; die alte Passphrase funktioniert danach nicht mehr.
  - [ ] AC4 — Abbruch mitten im Schreiben (Kill-Test) hinterlässt entweder die alte oder die neue, nie eine kaputte Datei.
- **Abhängigkeiten:** SEC-001

### SEC-004 — Secret-Verwaltung über CLI & UI
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `beton secrets add|list|rm|test|rotate` und die Einstellungs-UI verwalten Secrets. Werte werden nur eingegeben (verdeckte Eingabe, stdin, Datei), nie angezeigt. `test` prüft ein Secret gegen den Provider (z. B. GitHub `/user`) über den Proxy-Pfad, ohne den Wert auszugeben.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton secrets list` zeigt Ref, Typ, Provider, Account, zuletzt genutzt — keine Werte, auch nicht gekürzt.
  - [ ] AC2 — `beton secrets add` mit Wert als Kommandozeilen-Argument wird abgelehnt (Hinweis auf stdin/Prompt), damit er nicht in Shell-History/`ps` landet.
  - [ ] AC3 — Die UI-API bietet keinen Endpunkt, der Werte zurückgibt (OpenAPI-Snapshot-Test auf Felder namens `value` in Responses).
- **Abhängigkeiten:** SEC-001, SEC-002, SEC-003

### SEC-005 — Zentrale Envelope-Encryption
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Zentral wird jeder Secret-Wert mit einem eigenen zufälligen DEK (256 bit, neu pro Version) per AES-256-GCM verschlüsselt; der DEK wird mit dem aktiven Master-Key (KEK) per AES-256-GCM gewrappt. Der Master-Key kommt aus Env (`BETON_MASTER_KEY_<ID>`, base64, 32 Byte) oder Datei (nur Owner-Rechte); er liegt nie in DB, Logs oder Diagnose-Bundles.
- **Details:** Spalten: `ciphertext, nonce, wrapped_dek, dek_nonce, kek_id, version`. AAD Wert: `secret_id ‖ version ‖ org_id ‖ scope ‖ provider ‖ account`; AAD DEK: `secret_id ‖ version ‖ kek_id`. Beim Start entschlüsselt der Server einen Prüfdatensatz pro konfiguriertem Key (Key-Check-Value); Fehlschlag → Start verweigert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Vertauscht man `ciphertext`/`wrapped_dek` zweier Secrets in der DB, schlägt die Entschlüsselung beider fehl (AAD-Test).
  - [ ] AC2 — Start mit falschem Master-Key wird mit `master_key_mismatch` verweigert; Start mit Key-Datei im Modus 0644 ebenso.
  - [ ] AC3 — Ein DB-Dump enthält keinen Klartextwert und keinen Master-Key (Grep auf Testwerte).
  - [ ] AC4 — Zwei Versionen desselben Secrets haben unterschiedliche DEKs und Nonces.
- **Abhängigkeiten:** SEC-001, DATA-004 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0024

### SEC-006 — Master-Key-Rotation
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Mehrere Master-Keys können parallel konfiguriert sein; neue Schreibvorgänge nutzen `active_key`. `beton admin secrets rewrap --to <id>` wrappt alle DEKs online, in Batches, idempotent und fortsetzbar neu (Werte werden nicht neu verschlüsselt). Ein Key darf erst entfernt werden, wenn kein DEK mehr auf ihn verweist; fehlt beim Start ein referenzierter Key, verweigert der Server den Start mit Anzahl betroffener Secrets.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach Rewrap von 10 000 Secrets referenziert keiner mehr den alten `kek_id`; alle sind weiterhin lesbar.
  - [ ] AC2 — Ein Abbruch des Rewrap (Kill nach 50 %) und Neustart führt zum vollständigen Ergebnis ohne Datenverlust.
  - [ ] AC3 — Entfernen des alten Keys aus der Konfiguration, solange noch Referenzen existieren, verhindert den Start mit Fehlermeldung inkl. Anzahl.
  - [ ] AC4 — Rotation und Rewrap erzeugen Audit-Events (`secrets.key_rotated`, `secrets.rewrapped`).
- **Abhängigkeiten:** SEC-005, SEC-012

### SEC-007 — Bindung & Auflösung von Secrets
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Jedes Secret ist an `(org, user|team, provider, account)` gebunden. Eine Session darf nur SecretRefs auflösen, deren Bindung zum **Session-Owner** passt: `user` = Owner selbst, `team:<slug>` = Owner ist Mitglied, `org` = Org des Owners. Policies können zusätzlich einschränken, welche Refs in welchen Projekten/Agents zulässig sind (POL, siehe 03-policies.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Session von Bob mit `secret://user/github.com/alice` in der Agent-Konfiguration startet nicht (`secret_binding_mismatch`), auch wenn Bob die Agent-YAML von Alice nutzt.
  - [ ] AC2 — Ein Team-Secret ist nach Austritt des Owners aus dem Team in seinen neuen Sessions nicht mehr auflösbar; laufende Leases werden widerrufen.
  - [ ] AC3 — Ein `drive`-Teilnehmer an Alices Session löst keine eigenen Secrets in diese Session auf; genutzt werden ausschließlich Alices Bindungen.
- **Abhängigkeiten:** SEC-005, AUTH-014

### SEC-008 — Secret-Auslieferung an Runner (Leases)
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Remote-Runner erhalten Secret-Werte nur für die Credential-Bindings ihrer Session, auf Anfrage über den authentisierten Tunnel, zeitlich begrenzt und ausschließlich im Speicher des Proxys. Der Server prüft Bindung (SEC-007), Policy und optional Host-Labels (`secrets.deliver_to`) und auditiert jede Gewährung. Änderungen oder Löschungen eines Secrets widerrufen aktive Leases.
- **Details:** Tunnel-Nachrichten (PROTO-015, siehe 06-data-sync-protocol.md): `secret.lease.request {session_id, ref, binding}` → `secret.lease.grant {lease_id, value, expires_at}` (TTL 1 h, erneuerbar) | `secret.lease.deny {reason}`; `secret.lease.revoke {lease_id}`. Runner: `mlock` soweit möglich, `zeroize` bei Ablauf. Lokal derselbe Pfad in-process.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Runner fordert ein Secret an, das in keinem Binding seiner Session steht → `secret.lease.deny`, Audit-Eintrag `credential.misuse`.
  - [ ] AC2 — Nach Löschen des Secrets erhält der Runner innerhalb von 5 s `secret.lease.revoke`; der nächste Proxy-Request mit dem Platzhalter liefert 502.
  - [ ] AC3 — Mit `secrets.deliver_to: {labels: [trusted]}` erhält ein Host ohne Label keine Leases.
  - [ ] AC4 — Weder Runner-Logs noch Core-Dumps im Test (`RLIMIT_CORE=0` gesetzt) enthalten den Wert.
- **Abhängigkeiten:** SEC-007, AUTH-011, PRX-006 (siehe 04-sandbox.md)

### SEC-009 — GitHub-Verbindung per OAuth-App (inkl. Enterprise Server)
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** User verbinden ihr GitHub- bzw. GitHub-Enterprise-Server-Konto über eine vom Betreiber registrierte OAuth-App (Web-Flow mit `state`; lokal Loopback-Redirect oder Device-Flow). Das Token wird als `secret://user/<github-host>/<login>` (Typ `oauth`) gespeichert und vom Proxy (`type: github`) sowie vom Git-Provider-Client (GIT-002, siehe 07-sessions-collaboration.md) genutzt. Mehrere Hosts (github.com + GHES) und mehrere Konten pro Host sind möglich. SEC-009 ist Owner von OAuth-Flow und Token-Speicherung; Nutzung im CR-Panel und Proxy-Bindings pro Session regelt GIT-004.
- **Details:** Konfiguration in `git.providers[]` (GIT-001) mit `kind: github, host, api_url, client_id, client_secret`; minimale Scopes `repo`, `read:org`, `read:user`, `user:email` (`workflow` optional). Inhaberschaft einer öffentlich nutzbaren OAuth-App für den lokalen Modus ist offen *(Annahme: wie Signing-Accounts zu klären; bis dahin PAT-Fallback)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach dem Flow existiert ein Secret mit Bindung an den User und Login-Namen; `beton secrets test` gegen `/user` ist erfolgreich.
  - [ ] AC2 — Callback mit fremdem `state` wird abgelehnt.
  - [ ] AC3 — Eine GHES-Verbindung nutzt `api_url` (`/api/v3`) und erzeugt ein Proxy-Binding für den GHES-Host, nicht für `github.com`.
  - [ ] AC4 — „Verbindung trennen“ widerruft das Token beim Provider (`DELETE /applications/{client_id}/token`) und löscht das Secret.
- **Abhängigkeiten:** SEC-007, GIT-001, GIT-004 (siehe 07-sessions-collaboration.md)

### SEC-010 — GitLab-Verbindung per OAuth (inkl. self-hosted)
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Analog SEC-009 für gitlab.com und self-hosted GitLab-Instanzen (pro Instanz eigene OAuth-Application). GitLab-Access-Tokens laufen ab; beton erneuert sie per Refresh-Token serverseitig vor Ablauf und aktualisiert laufende Leases.
- **Details:** Scopes `api` (bzw. `read_api` + `read_repository` + `write_repository`), `read_user`; PKCE wird genutzt. Refresh 5 min vor Ablauf; bei Refresh-Fehler wird der User per Inbox zur Neuverbindung aufgefordert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Token mit 2 h Laufzeit wird automatisch erneuert; ein laufender `git push` nach 3 h gelingt ohne Nutzeraktion.
  - [ ] AC2 — Für eine self-hosted Instanz mit eigener CA (`git.providers[].ca_file`, GIT-003) gelingt die Verbindung, ohne die TLS-Prüfung abzuschalten.
  - [ ] AC3 — Schlägt der Refresh fehl, entsteht ein Inbox-Eintrag „GitLab neu verbinden“ und Proxy-Requests liefern 502 `credential_expired`.
- **Abhängigkeiten:** SEC-007, GIT-001, GIT-004 (siehe 07-sessions-collaboration.md)

### SEC-011 — Git-PAT-Fallback
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Ohne OAuth-App (insbesondere lokal) hinterlegen User Personal Access Tokens von GitHub/GitLab als Secret (`beton secrets add github --host github.com`). Sie werden wie OAuth-Tokens per Proxy injiziert. `beton setup` erkennt vorhandene CLI-Logins (`gh`, `glab`) und **bietet** an, deren Token einmalig zu übernehmen (nie still). Nutzung im CR-Panel ab M4: GIT-004.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit hinterlegtem PAT funktionieren `git clone` eines privaten Repos und `gh pr list` in Stufe 2 nur über den Platzhalter.
  - [ ] AC2 — `beton setup` übernimmt ein `gh`-Token erst nach expliziter Bestätigung; ohne Bestätigung wird nichts gespeichert.
  - [ ] AC3 — `beton secrets test` meldet fehlende Scopes (z. B. `repo`) anhand der Provider-Antwort.
- **Abhängigkeiten:** SEC-002, PRX-006 (siehe 04-sandbox.md)

### SEC-012 — Audit-Log
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Sicherheitsrelevante Aktionen werden in einem Append-only-Audit-Log protokolliert: Secret erstellt/geändert/gelöscht/genutzt/Lease, Credential-Fehlgebrauch, Logins und Fehlschläge, Pairing/Widerruf, Token-Erstellung/-Widerruf, Rollen- und Share-Änderungen, Policy-Änderungen, Sandbox `backend: none`, CA-Rotation, Master-Key-Rotation. Jeder Eintrag enthält Zeit, Principal, Aktion, Ziel, Session/Host, IP/User-Agent (zentral) und Details — nie Secret-Werte. Einträge bilden eine Hash-Kette (`hash = SHA-256(prev_hash ‖ entry)`). Lokal ab M2, zentral mit Org-Bezug ab M4.
- **Details:** Hochfrequente Nutzung (Proxy) wird pro `(secret, session, host)` in 5-min-Fenstern mit Zähler aggregiert. Retention zentral Default 365 Tage, lokal 90 Tage *(Annahme)*. `beton audit list|export|verify`; REST `GET /v1/audit` (Scope `audit:read`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton audit verify` erkennt eine nachträglich in der DB geänderte oder gelöschte Zeile und nennt die erste ungültige Position.
  - [ ] AC2 — Test mit markiertem Secret-Wert: Der Wert kommt in keinem Audit-Eintrag vor, auch nicht bei Fehlermeldungen.
  - [ ] AC3 — 1 000 Proxy-Nutzungen eines Secrets innerhalb von 5 min erzeugen genau einen Eintrag mit `count: 1000`.
  - [ ] AC4 — Ein Member sieht über die API nur Einträge, deren Principal er selbst ist.
- **Abhängigkeiten:** DATA-001, DATA-002 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0024

### SEC-013 — Secret-Redaction in Logs, Events & Transkripten
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Bevor Event-Payloads (inkl. `raw`), Tool-Outputs, Logs (`tracing`-Layer), Exporte und Diagnose-Bundles persistiert oder versendet werden, durchlaufen sie eine Redaction-Pipeline: (1) exakter Abgleich gegen alle auf dem Knoten bekannten Secret-Werte (Aho-Corasick, inkl. Base64- und URL-kodierter Varianten), (2) Muster-Detektoren für verbreitete Token-Formate. Treffer werden durch `[REDACTED:<kind>]` ersetzt; `bt_cred_`-Platzhalter bleiben stehen (harmlos).
- **Details:** Detektoren: GitHub (`ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_`, `github_pat_`), GitLab (`glpat-`, `gloas-`), Anthropic (`sk-ant-`), OpenAI (`sk-` / `sk-proj-` mit Mindestlänge), AWS (`AKIA…`/`ASIA…` + Secret-Key-Kontext), Slack (`xox[abpr]-`), PEM-Private-Keys, beton-Tokens (`bt_pat_`, `bt_sat_`, `bt_dev_`, `bt_run_`, `bt_loc_` mit CRC-Prüfung), JWTs in `Authorization`-Kontext. Entropie-Heuristik ist abschaltbar und standardmäßig aus. Nachträglich entdeckte Leaks lassen sich per Payload-Redaktion entfernen (DATA-012, siehe 06-data-sync-protocol.md). SEC-013 ist Owner der Redaction-Pipeline; OBS-002 bindet sie in Logs/Traces ein.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Gibt ein Tool einen per `env.passthrough` durchgereichten echten Token-Wert aus, enthält das persistierte `tool.call.completed` nur `[REDACTED:…]`; Live-Clients erhalten ebenfalls die redigierte Fassung.
  - [ ] AC2 — Ein PEM-Private-Key in einer Agent-Nachricht wird vollständig (BEGIN bis END) ersetzt.
  - [ ] AC3 — Die Redaction verarbeitet ≥ 50 MB/s pro Kern bei 1 000 bekannten Werten (Benchmark in CI, Regression > 20 % schlägt fehl).
  - [ ] AC4 — `beton diagnose` enthält nach Redaction keinen der Test-Secret-Werte (Grep über das Bundle).
- **Abhängigkeiten:** SEC-001, PROTO-001, DATA-002 (siehe 06-data-sync-protocol.md), OBS-001 (siehe 11-platform-features.md)

### SEC-014 — Server-Härtung: TLS, Security-Header, Rate-Limits
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Der zentrale Server terminiert TLS selbst (rustls, TLS 1.2+, 1.3 bevorzugt) oder läuft hinter einem Reverse-Proxy; `X-Forwarded-*` wird nur von `server.trusted_proxies` (CIDRs) akzeptiert. Die Web-UI wird mit strikten Security-Headern ausgeliefert, nutzergenerierte Inhalte (Workspace-Dateien, Anhänge) nur mit isolierender CSP bzw. als Download. Auth-, Pairing- und Token-Endpunkte sind rate-limitiert.
- **Details:** CSP u. a. `default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; connect-src 'self' wss://<host>; frame-ancestors 'none'; object-src 'none'; base-uri 'none'`; dazu HSTS (bei HTTPS), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy: same-origin`, `Permissions-Policy` (Mikrofon nur `self`). Rate-Limits: Login/Callback/Pairing 10/min/IP, Token-Endpunkte 60/min/Principal; Request-Body-Limit 10 MiB außer Upload-Routen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Request mit `X-Forwarded-For` von einer IP außerhalb `trusted_proxies` wird mit der echten Peer-IP geloggt und rate-limitiert.
  - [ ] AC2 — Ein HTML-Anhang mit `<script>` wird nicht im Origin der App ausgeführt (`Content-Security-Policy: sandbox` bzw. `Content-Disposition: attachment`; E2E-Test).
  - [ ] AC3 — 11 Login-Versuche pro Minute von derselben IP: der elfte erhält 429 mit `Retry-After`.
  - [ ] AC4 — TLS-Scan (CI-Test mit `rustls`-Client) akzeptiert TLS 1.0/1.1 nicht.
- **Abhängigkeiten:** AUTH-007

## Nicht in v1

- **SCIM**-Provisionierung — v2.
- **KMS/Vault als Master-Key-Backend** (als Plugin) — v2.
- **Öffentliche, login-freie Share-Links** — v2.
