# 09 — Eingebetteter Browser & Inspect-Mode

Dieses Kapitel spezifiziert den **eingebetteten, agent-steuerbaren Browser** von beton (Crate `beton-browser`) gemäß ADR-0016: Chromium wird headless über das Chrome DevTools Protocol (CDP, via `chromiumoxide`) gesteuert, sein Bild per `Page.startScreencast` in ein Panel der Clients gestreamt und Nutzereingaben per `Input.dispatch*` zurückgespielt („Variante C“). Ein Umschalter wechselt auf ein sichtbares Chromium-Fenster („Variante B“) für Logins, OAuth und echte DevTools. Dazu kommen Agent-Tools (per MCP an jeden Harness), der **Inspect-Mode/Element-Picker**, der ausgewählte Elemente samt Kontext an den Agent schickt, sowie Sicherheitsregeln (Egress-Proxy, Policies, isoliertes Profil).

Abgrenzung: Darstellung im Workspace-Rail (Browser-Tab) ist WEB-008/DESK-001 (siehe 08-clients.md); Binärkanäle und Event-Format sind PROTO-007/PROTO-002 (siehe 06-data-sync-protocol.md); Policy-Auswertung ist POL-020/POL-004 (siehe 03-policies.md); Egress-Proxy ist PRX-011 (siehe 04-sandbox.md). Meilenstein ist durchgehend **M3** (Desktop & TUI, Release 0.1), Ausnahmen sind markiert.

## Konzepte & Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Browser-Instanz** | Ein Chromium-Prozess pro Session, gestartet auf dem Host des Runners (dort, wo Workspace und lokale Dev-Server liegen). |
| **Profil** | Eigenes `--user-data-dir` pro Session; Cookies/Storage sind von anderen Sessions und vom Alltags-Browser des Users getrennt. |
| **Modus `embedded`** | Headless-Chromium; Bild per Screencast im Browser-Panel, Eingaben per CDP. Default. |
| **Modus `window`** | Sichtbares Chromium-Fenster auf dem Desktop des Hosts (nur bei lokalem Host mit Display). |
| **Tab / Target** | CDP-Target vom Typ `page`; ein Tab ist „aktiv“ und wird gestreamt. |
| **Snapshot** | Accessibility-Tree-Auszug der aktiven Seite mit stabilen `ref`s (`e12`), über die Agent-Tools Elemente adressieren. |
| **Steuernder Client** | Client, der zuletzt Eingaben in den Browser geschickt hat; bestimmt Viewport-Größe und DPR. |
| **Picker** | Inspect-Mode auf Basis von `Overlay.setInspectMode`; liefert ein **Picker-Payload** an den Agent. |
| **Dev-Server** | Lokal lauschender HTTP-Server eines Workspace-Prozesses (z.B. Vite auf `localhost:5173`). |

## Design

### Architektur & Datenfluss

```
 Host (Runner-Maschine)                                   Server                Clients
┌────────────────────────────────────────────┐
│ Chromium (headless | window)               │
│   --user-data-dir=…/profiles/<ses>         │
│   --proxy-server=<egress-proxy>            │
│        ▲ CDP (WebSocket, lokal)            │
│ beton-browser (im Runner)                  │
│   ├─ Screencast  ── JPEG-Frames ──────────────► Binärkanal ──► Fan-out ──► Canvas (Web/Desktop)
│   ├─ Input-Bridge ◄── input.* ──────────────── WS ◄────────────────────── Maus/Tastatur
│   ├─ Agent-Tools (MCP: browser_*) ◄── Harness│
│   ├─ Picker (Overlay/DOM/CSS) ─── browser.picked (Event) ──────────────► Composer / Agent
│   └─ Policy-Hooks (navigate, submit, download, eval) ─► POL
└────────────────────────────────────────────┘
```

Frames sind ephemer und laufen über einen eigenen Binärkanal je Session (PROTO); im Event-Log landen nur Screenshots, die ein Agent-Tool oder der Picker explizit erzeugt (als Blob).

### Komponenten-Trait (Skizze)

```rust
#[async_trait::async_trait]
pub trait BrowserSession: Send + Sync {
    async fn ensure_started(&self, mode: BrowserMode) -> Result<()>;          // Embedded | Window
    async fn switch_mode(&self, to: BrowserMode) -> Result<ModeSwitchReport>; // Tabs/URLs/Scroll wiederhergestellt
    async fn tabs(&self) -> Result<Vec<TabInfo>>;
    async fn navigate(&self, tab: TabId, url: &Url, wait: WaitUntil) -> Result<NavResult>;
    async fn snapshot(&self, tab: TabId, opts: SnapshotOpts) -> Result<A11ySnapshot>;
    async fn act(&self, tab: TabId, action: Action) -> Result<ActionResult>;  // click/type/select/scroll/press
    async fn screenshot(&self, tab: TabId, opts: ShotOpts) -> Result<BlobRef>;
    async fn eval(&self, tab: TabId, expr: &str) -> Result<serde_json::Value>; // nur nach Policy-Freigabe
    async fn wait_for(&self, tab: TabId, cond: WaitCond, timeout: Duration) -> Result<()>;
    async fn start_pick(&self, tab: TabId, opts: PickOpts) -> Result<PickHandle>;
    fn frames(&self) -> FrameStream;                                           // für den Binärkanal
}
```

### Picker-Payload (Beispiel)

Das Payload wird als Event `browser.picked` (PROTO-002) geloggt und dem Agent als strukturierter Input-Block übergeben (Text + Bild-Attachment für den Crop).

```json
{
  "type": "browser.pick",
  "version": 1,
  "session_id": "ses_01JB7Q8K4Z3M2V",
  "tab": { "id": "tab_2", "url": "http://localhost:5173/settings/profile", "title": "Profil – Acme" },
  "viewport": { "width": 1280, "height": 800, "device_pixel_ratio": 2 },
  "comment": "Der Speichern-Button soll rechtsbündig neben Abbrechen stehen und primär gefärbt sein.",
  "elements": [
    {
      "index": 1,
      "ref": "e47",
      "selector": {
        "primary": "[data-testid=\"profile-save\"]",
        "strategy": "test_id",
        "alternatives": [
          "form#profile-form button[type=\"submit\"]",
          "role=button[name=\"Speichern\"]"
        ],
        "unique": true
      },
      "role": "button",
      "accessible_name": "Speichern",
      "outer_html": "<button data-testid=\"profile-save\" type=\"submit\" class=\"btn btn-secondary mt-4\">Speichern</button>",
      "outer_html_truncated": false,
      "bounding_box": { "x": 312, "y": 604, "width": 96, "height": 36 },
      "computed_styles": {
        "display": "inline-flex", "position": "static", "margin": "16px 0px 0px 0px",
        "padding": "8px 16px", "color": "rgb(17, 24, 39)", "background-color": "rgb(229, 231, 235)",
        "font-family": "Inter, sans-serif", "font-size": "14px", "font-weight": "500",
        "border-radius": "6px", "justify-self": "auto", "align-self": "auto"
      },
      "parent_layout": { "selector": "form#profile-form > div.actions", "display": "flex", "justify-content": "flex-start", "gap": "8px" },
      "screenshot": { "blob": "blob:sha256:9f2c…e1", "mime": "image/png", "crop": { "x": 296, "y": 588, "width": 128, "height": 68 }, "highlighted": true },
      "source": {
        "framework": "react",
        "component": "ProfileForm",
        "file": "src/features/profile/ProfileForm.tsx",
        "line": 88,
        "column": 9,
        "confidence": "sourcemap"
      }
    }
  ],
  "picked_by": { "user_id": "usr_ingo", "client": "desktop" },
  "picked_at": "2026-10-03T09:41:12.512Z"
}
```

`source` ist optional (BRW-016, Stretch) und fehlt, wenn keine Zuordnung möglich ist.

## Features

### BRW-001 — Chromium-Discovery
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** beton erkennt auf dem Host installierte Chromium-basierte Browser (Chrome, Chromium, Edge, Brave) und nutzt den ersten kompatiblen. Eine explizite Konfiguration hat Vorrang. Das Ergebnis erscheint in `beton doctor`.
- **Details:** Reihenfolge: `browser.executable` (Config) → `BETON_BROWSER_PATH` → bereits geladenes Chrome for Testing (BRW-002) → OS-Standardpfade (macOS `/Applications/*.app`, Linux `PATH`-Namen `google-chrome`, `chromium`, `chromium-browser`, `microsoft-edge`, Windows Registry `App Paths`). Mindestversion: Chromium 120 *(Annahme)*; Version per `--version` bzw. CDP `Browser.getVersion`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit gesetztem `browser.executable` wird ausschließlich dieser Pfad verwendet; ist er ungültig, schlägt der Start mit klarer Meldung fehl (kein Fallback).
  - [ ] AC2 — `beton doctor --json` enthält einen Check `browser.chromium` mit Pfad und Version oder `warn` mit Hinweis auf BRW-002.
  - [ ] AC3 — Ein Browser unter der Mindestversion wird übersprungen und im Doctor als `incompatible` gelistet.
- **Abhängigkeiten:** OBS-005 (siehe 11-platform-features.md)

### BRW-002 — Chrome for Testing auf Nachfrage
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Ist kein kompatibler Browser vorhanden, bietet beton den Download von **Chrome for Testing** (gepinnte Version, passend zu OS/Arch) an – nie still. Download nach `~/.beton/browser/cft/<version>/`, Prüfung per SHA-256 aus dem offiziellen Versions-Manifest.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ohne Browser zeigt der erste Browser-Start (UI oder Agent-Tool) eine Rückfrage mit Größe und Quelle; ohne Zustimmung wird nichts geladen und das Tool liefert `browser_unavailable`.
  - [ ] AC2 — Eine Datei mit falscher Prüfsumme wird verworfen und nicht ausgeführt.
  - [ ] AC3 — Im Skript-/Async-Modus ohne Zuschauer erfolgt der Download nur bei `browser.auto_download: true`.
- **Abhängigkeiten:** BRW-001

### BRW-003 — CDP-Lebenszyklus pro Session
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** `beton-browser` startet Chromium lazy beim ersten Bedarf (Panel öffnen oder Agent-Tool), hält die CDP-Verbindung via `chromiumoxide` und beendet die Instanz bei Session-Stop oder nach Leerlauf. Abstürze werden erkannt und die Instanz mit gleichem Profil neu gestartet.
- **Details:** Start-Flags: `--headless=new`, `--remote-debugging-pipe` *(Annahme: Pipe statt Port, damit kein lokaler Port offen ist)*, `--user-data-dir`, `--proxy-server`, `--no-first-run`, `--no-default-browser-check`, `--disable-background-networking`. Idle-Timeout `browser.idle_timeout` (Default 15 min ohne Viewer und ohne Tool-Aufruf). Events `browser.opened` / `browser.closed { reason: stopped|idle|crashed }` (PROTO-002).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Session ohne Browser-Nutzung startet keinen Chromium-Prozess.
  - [ ] AC2 — Nach `kill -9` des Browser-Prozesses ist er beim nächsten Tool-Aufruf mit gleichem Profil wieder verfügbar; die letzte URL des aktiven Tabs wird wiederhergestellt.
  - [ ] AC3 — Beim Stoppen der Session ist kein Chromium-Prozess der Session mehr aktiv (Prozessbaum-Test).
  - [ ] AC4 — Auf dem Host ist kein CDP-Port nach außen oder auf Loopback offen.
- **Abhängigkeiten:** RUN-003 (siehe 10-runners-extensibility.md), PROTO-002 (siehe 06-data-sync-protocol.md)

### BRW-004 — Isoliertes Profil pro Session
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Jede Session erhält ein eigenes Profilverzeichnis `~/.beton/browser/profiles/<session_id>/`. Es wird nie mit dem Alltagsprofil des Users oder anderen Sessions geteilt. Forks und Side-Chats starten mit leerem Profil *(Annahme)*. Löschen der Session löscht das Profil.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein in Session A gesetztes Cookie ist in Session B nicht vorhanden.
  - [ ] AC2 — Nach Session-Löschung existiert das Profilverzeichnis nicht mehr.
  - [ ] AC3 — Das Profilverzeichnis liegt außerhalb der Sandbox-Schreib-Roots der Tool-Ausführung (Agent kann Cookies nicht per Shell auslesen).
- **Abhängigkeiten:** BRW-003, SBX-003 (siehe 04-sandbox.md)

### BRW-005 — Screencast-Streaming ins Panel
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Der aktive Tab wird per `Page.startScreencast` (JPEG) erfasst; jedes Frame geht über den Binärkanal der Session an alle betrachtenden Clients und wird dort in ein `<canvas>` gezeichnet. Langsame Clients bekommen nur das jeweils neueste Frame (Drop statt Stau); `Page.screencastFrameAck` wird unabhängig von Clients sofort nach Übernahme in den Frame-Puffer gesendet.
- **Details:** Frame-Header (binär): `{tab_id, frame_seq, width, height, dpr, scroll_offset, timestamp}` + JPEG-Bytes. Ohne Viewer wird der Screencast pausiert (`Page.stopScreencast`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Lokal ist die Latenz vom Seitenwechsel bis zur Anzeige im Panel ≤ 150 ms (p95).
  - [ ] AC2 — Ein künstlich gedrosselter Client (100 KB/s) verzögert die Frames anderer Clients nicht (Test mit zwei Clients).
  - [ ] AC3 — Ohne verbundene Viewer sendet Chromium keine Screencast-Frames.
- **Abhängigkeiten:** PROTO-007 (siehe 06-data-sync-protocol.md), WEB-008 (siehe 08-clients.md)

### BRW-006 — Input-Weiterleitung
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Maus (Move, Klick, Doppelklick, Rad), Tastatur (inkl. Modifier, IME-Komposition, Shortcuts) und Zwischenablage-Paste aus dem Panel werden per `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent` bzw. `Input.insertText` an den Tab weitergereicht. Koordinaten werden von Canvas- in Seitenkoordinaten umgerechnet. Nur Clients mit Rolle `drive` (COL-001, ab M4; lokal der Owner) dürfen Eingaben senden.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Text in ein Formularfeld tippen, Tab-Fokuswechsel und `Enter` funktionieren identisch zu einem echten Browser (E2E gegen Testseite).
  - [ ] AC2 — Ein Klick bei HiDPI-Client (DPR 2) trifft das Element unter dem Cursor (Abweichung ≤ 1 CSS-Pixel).
  - [ ] AC3 — Eingaben eines Clients ohne `drive` werden serverseitig mit `403` verworfen.
  - [ ] AC4 — Browser-eigene Shortcuts des Clients (z.B. `⌘L`) werden nicht an die Seite weitergereicht, wenn das Panel sie selbst belegt (Adressleiste fokussieren).
- **Abhängigkeiten:** BRW-005, COL-001 (ab M4, siehe 07-sessions-collaboration.md)

### BRW-007 — Adaptive Framerate/Qualität & HiDPI
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Qualität und Bildrate passen sich an Viewport, DPR und gemessenen Durchsatz an. Der steuernde Client bestimmt Viewport und DPR (über `Emulation.setDeviceMetricsOverride`); andere Clients skalieren das Bild (Letterboxing).
- **Details:** Stufen *(Annahme)*: `high` (JPEG q 80, bis 30 fps), `medium` (q 60, 15 fps), `low` (q 40, 5 fps, `maxWidth` halbiert). Wechsel nach unten, wenn die Sende-Queue > 2 Frames für > 1 s; nach oben nach 5 s stabilem Durchsatz. DPR max. 2. Bei Tab im Hintergrund des Clients: 1 fps.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bei gedrosselter Verbindung wechselt die Stufe binnen 2 s auf `medium` bzw. `low`, ohne dass das Panel einfriert.
  - [ ] AC2 — Auf einem Retina-Client ist Text scharf (Frame-Breite = CSS-Breite × DPR).
  - [ ] AC3 — Wechselt der steuernde Client, übernimmt der Viewport dessen Größe innerhalb von 500 ms.
- **Abhängigkeiten:** BRW-005

### BRW-008 — Umschalter auf sichtbares Fenster
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Ein Umschalter wechselt zwischen `embedded` und `window`. Da Headless- und sichtbarer Modus nicht im laufenden Prozess wechseln können, startet beton Chromium mit **demselben Profil** neu und stellt Tabs (URLs, aktiver Tab, Scroll-Position) wieder her; Cookies, LocalStorage und IndexedDB bleiben über das Profil erhalten. Agent-Tools funktionieren in beiden Modi; der Screencast läuft auch im Fenster-Modus weiter.
- **Details:** Nicht erhalten: `sessionStorage`, ungespeicherte Formulareingaben, laufender JS-Zustand → der Umschalter warnt vorher. `window` ist nur verfügbar, wenn der Host lokal ist und ein Display hat (Desktop-App bzw. CLI auf dem Rechner des Users); sonst ist die Option deaktiviert mit Begründung. Typischer Zweck: Login/OAuth, echte DevTools.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach Login im Fenster-Modus und Zurückschalten ist der User im eingebetteten Modus weiterhin eingeloggt (Cookie-basierte Testseite).
  - [ ] AC2 — Drei offene Tabs sind nach dem Umschalten mit denselben URLs und demselben aktiven Tab vorhanden.
  - [ ] AC3 — Auf einem Remote-Host ist `window` nicht wählbar; die API liefert `409 window_mode_unavailable`.
- **Abhängigkeiten:** BRW-003, BRW-004

### BRW-009 — Browser-Panel: Tabs & Navigation
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Das Browser-Panel im Workspace-Rail zeigt Tab-Leiste, Adressleiste (mit Policy-Hinweis bei gesperrten Zielen), Zurück/Vor/Neu laden, Modus-Umschalter, Picker-Button und Indikator „Agent steuert“ während Agent-Aktionen. Links aus dem Chat können im Session-Browser geöffnet werden.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Navigiert der Agent, aktualisieren sich Adressleiste und Tab-Titel in allen Clients innerhalb von 500 ms.
  - [ ] AC2 — Während einer Agent-Aktion zeigt das Panel „Agent steuert“; User-Eingaben in dieser Zeit sind möglich und unterbrechen die Agent-Aktion nicht stillschweigend, sondern werden nach ihr zugestellt *(Annahme)*.
  - [ ] AC3 — Ein Chat-Link „Im Session-Browser öffnen“ öffnet einen neuen Tab im Session-Browser.
- **Abhängigkeiten:** BRW-005, WEB-008 (siehe 08-clients.md)

### BRW-010 — Agent-Tools (Basis)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Der Browser ist für jeden Harness über die System-Tools von `beton-mcp` steuerbar (AGT-007, siehe 02-agents.md). Elemente werden über `ref`s aus dem letzten Snapshot oder über Selektoren adressiert. Jede Aktion liefert ein kompaktes Ergebnis (neue URL, Fehler, optional neuer Snapshot-Ausschnitt).
- **Details:**

  | Tool | Parameter (Auszug) | Ergebnis |
  | --- | --- | --- |
  | `browser_navigate` | `url`, `tab?`, `wait_until: load\|domcontentloaded\|networkidle` | `url`, `status`, `title` |
  | `browser_snapshot` | `tab?`, `scope_ref?`, `max_nodes` (Default 500) | A11y-Tree als Text mit `[ref=eN]` |
  | `browser_click` | `ref\|selector`, `button?`, `click_count?`, `modifiers?` | `navigation?`, `dialog?` |
  | `browser_type` | `ref\|selector`, `text`, `submit?: bool`, `clear?: bool` | — |
  | `browser_select` | `ref\|selector`, `values[]` | ausgewählte Werte |
  | `browser_scroll` | `ref?`, `dx`, `dy` \| `to: top\|bottom` | Scroll-Position |
  | `browser_press` | `keys` (z.B. `"Control+A"`) | — |
  | `browser_screenshot` | `tab?`, `ref?`, `full_page?` | Bild (Blob, als Image-Content an das Modell) |
  | `browser_wait_for` | `text?\|ref?\|selector?\|url_matches?`, `state: visible\|hidden`, `timeout_ms` (≤ 30 000) | erfüllt/Timeout |
  | `browser_tabs` | `action: list\|new\|select\|close`, `tab?` | Tab-Liste |
  | `browser_console` | `since?`, `level?` | Konsolen- und Netzwerkfehler der Seite |
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Agent (Fake-Harness-Skript) loggt sich auf einer lokalen Testseite per `navigate` → `snapshot` → `type` → `click` ein; Ergebnis-URL stimmt.
  - [ ] AC2 — Ein veralteter `ref` (Seite neu geladen) liefert den Fehler `stale_ref` mit Hinweis, einen neuen Snapshot zu erstellen.
  - [ ] AC3 — Jeder Tool-Aufruf erzeugt `tool.call.*`-Events mit `tool.kind = browser` und durchläuft den Policy-Hook.
  - [ ] AC4 — Die Tools stehen Claude Code, Codex, ACP- und Direkt-API-Harness gleichermaßen zur Verfügung (Contract-Test je Harness mit Fake-Modell).
- **Abhängigkeiten:** BRW-003, BRW-011, AGT-007 (siehe 02-agents.md), POL-003, POL-005 (siehe 03-policies.md)
- **Referenz:** Omnigent `browser_navigate|snapshot|click|type|screenshot`

### BRW-011 — Accessibility-Snapshot mit stabilen refs
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** `browser_snapshot` erzeugt aus `Accessibility.getFullAXTree` (plus DOM-Infos für nicht-semantische klickbare Elemente) eine kompakte, eingerückte Textdarstellung mit Rolle, Name, Wert, Zustand und `ref`. `ref`s bleiben innerhalb eines Dokuments stabil (Mapping auf `backendNodeId`) und werden bei Navigation ungültig.
- **Details:** Beispielzeile: `- button "Speichern" [ref=e47] (disabled=false)`. Ignoriert: unsichtbare/`aria-hidden`-Knoten; Iframes gleicher Origin werden eingebettet, fremde als Platzhalter `- iframe "<origin>" [ref=eN]`. Kürzung bei `max_nodes` mit Hinweis `… N weitere Knoten (scope_ref verwenden)`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Snapshot einer Referenzseite entspricht dem Golden-Snapshot (Snapshot-Test).
  - [ ] AC2 — Zwei Snapshots ohne DOM-Änderung liefern identische `ref`s.
  - [ ] AC3 — Ein `div` mit Click-Handler ohne Rolle erscheint als `- clickable "<text>" [ref=…]`.
- **Abhängigkeiten:** BRW-003

### BRW-012 — `browser_eval` nur mit Policy
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Beliebiges JavaScript im Seitenkontext ist mächtig (Cookies, Tokens im DOM). `browser_eval` ist daher standardmäßig nicht registriert; aktiviert per Agent-Config (`tools.browser.eval: true`) und dann für jeden Aufruf durch den Policy-Hook mit Default `ask` geschützt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ohne Aktivierung ist `browser_eval` nicht in der Tool-Liste des Harness.
  - [ ] AC2 — Mit Aktivierung und ohne explizite Policy erzeugt jeder Aufruf eine Approval-Card mit dem vollständigen Ausdruck.
  - [ ] AC3 — Rückgabewerte > 64 KiB werden gekürzt und als gekürzt markiert.
- **Abhängigkeiten:** BRW-010, POL-003, POL-020 (siehe 03-policies.md)

### BRW-013 — Inspect-Mode / Element-Picker
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Der User aktiviert im Panel den Picker; beton schaltet `Overlay.setInspectMode` (`searchForNode`, Highlight-Config) ein. Mausbewegungen über dem Canvas werden als Mouse-Events weitergereicht, wodurch Chromium das Element-Highlight im Bild zeichnet. Ein Klick (`Overlay.inspectNodeRequested`) wählt das Element; ein Kommentarfeld öffnet sich; „Senden“ erzeugt das Picker-Payload und legt es in den Composer (Default) oder direkt in die Queue.
- **Details:** `Esc` beendet den Picker. Während des Pickers sind Klicks auf die Seite neutralisiert (keine Navigation). Optional kann der Agent per Tool `browser_request_pick {prompt}` den User um eine Auswahl bitten *(Annahme)*; das Ergebnis kommt als Tool-Result zurück.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Im Picker-Modus hebt das Panel das Element unter dem Cursor hervor; ein Klick löst keine Seitenaktion aus.
  - [ ] AC2 — Nach Auswahl und Kommentar enthält der Composer einen Picker-Block mit Vorschau-Crop.
  - [ ] AC3 — `Esc` verlässt den Picker ohne Payload; das Overlay ist entfernt.
- **Abhängigkeiten:** BRW-005, BRW-006

### BRW-014 — Picker-Payload an den Agent
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Für jedes gewählte Element erzeugt beton das Payload gemäß Design-Beispiel: stabiler Selektor, `outerHTML`-Ausschnitt, ausgewählte Computed Styles, Bounding-Box, Layout-Kontext des Elternelements, markierter Screenshot-Crop, Nutzerkommentar. Der Agent erhält eine Textfassung plus den Crop als Bild.
- **Details:** CDP-Aufrufe: `DOM.describeNode`, `DOM.getOuterHTML` (gekürzt auf 4 KiB, Kinder ab Tiefe 3 als `…`), `CSS.getComputedStyleForNode` (Whitelist von ~30 layout-/typo-relevanten Properties), `DOM.getBoxModel`, `Page.captureScreenshot` mit `clip` (+16 px Rand, Element umrandet). Selektor-Strategie in Reihenfolge: `data-testid`/`data-test`/`data-cy` → eindeutige, nicht generiert wirkende `id` → ARIA-Rolle + Name (`role=button[name=…]`) → kürzester eindeutiger CSS-Pfad mit `:nth-of-type`; Eindeutigkeit per `DOM.querySelectorAll` verifiziert. Generierte Klassen (Hash-Muster wie `css-1x2y3z`) werden nicht verwendet.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Das Payload validiert gegen das veröffentlichte JSON-Schema (`browser.pick` v1).
  - [ ] AC2 — `selector.primary` matcht auf der Seite genau ein Element; `unique: false` nur, wenn keine eindeutige Strategie existiert.
  - [ ] AC3 — `outer_html` ist ≤ 4 KiB; bei Kürzung ist `outer_html_truncated: true`.
  - [ ] AC4 — Der Crop zeigt das Element mit Markierung und wird als Blob gespeichert; das Event referenziert ihn per Hash.
- **Abhängigkeiten:** BRW-013, DATA-006, PROTO-002 (siehe 06-data-sync-protocol.md)

### BRW-015 — Mehrfachauswahl im Picker
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Mit gedrückter Umschalttaste werden mehrere Elemente nacheinander gewählt (nummerierte Marker im Panel); alle landen in einem Payload (`elements[]`) mit gemeinsamem Kommentar und optionalen Einzelkommentaren pro Element.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Drei per `⇧`+Klick gewählte Elemente erscheinen als `elements[0..2]` mit `index` 1–3 in Auswahlreihenfolge.
  - [ ] AC2 — Ein erneuter `⇧`+Klick auf ein gewähltes Element entfernt es aus der Auswahl.
- **Abhängigkeiten:** BRW-014

### BRW-016 — Quelldatei-Mapping (Stretch)
- **Meilenstein:** M3 · **Priorität:** Could
- **Beschreibung:** Für Seiten von lokalen Dev-Servern versucht beton, das gewählte Element einer Komponente und `datei:zeile` im Workspace zuzuordnen, damit der Agent direkt die richtige Stelle editiert. Ergebnis in `elements[].source` mit `confidence`.
- **Details:** Strategien in Reihenfolge: (1) explizite Attribute `data-beton-source="file:line:col"` (optionales Vite/Babel-Plugin von beton, *Annahme*); (2) Vue: `__vueParentComponent.type.__file`; Svelte: `__svelte_meta.loc`; (3) React: Fiber über `__reactFiber$*` → `_debugSource` (React ≤ 18) bzw. `_debugStack`/Owner-Stack (React 19) → Stack-Frame-URL; (4) Auflösung generierter Positionen über Sourcemaps (`Debugger.scriptParsed.sourceMapURL`). Pfade werden auf Workspace-relative Pfade normalisiert und nur zurückgegeben, wenn die Datei im Workspace existiert. `confidence: explicit | framework | sourcemap`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — In einem Vite-Vue-Beispielprojekt liefert ein Pick auf eine Komponente die korrekte `.vue`-Datei.
  - [ ] AC2 — In einem Vite-React-19-Beispielprojekt liefert ein Pick die richtige Datei und Zeile ±3.
  - [ ] AC3 — Pfade außerhalb des Workspaces werden nie zurückgegeben; ohne Zuordnung fehlt `source`.
- **Abhängigkeiten:** BRW-014, BRW-017

### BRW-017 — Erkennung lokaler Dev-Server
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** beton erkennt Dev-Server, die aus der Session heraus gestartet wurden (Agent-Tool-Calls oder Workspace-Terminals), und bietet „Im Session-Browser öffnen“ an. Für erkannte Ports wird automatisch eine Loopback-Ausnahme im Egress-Proxy für diese Session angelegt.
- **Details:** Quellen: (a) Terminal-/Tool-Output-Muster (`Local: http://localhost:5173`, `ready on http://127.0.0.1:3000`, …); (b) lauschende TCP-Sockets auf Loopback, deren Prozess Nachfahre des Runners ist (Prozessbaum, `/proc` bzw. `libproc`/`GetExtendedTcpTable`). Event `browser.devserver.detected {url, pid, source}` (PROTO-002). Ausnahme gilt nur für den Browser dieser Session und nur für `127.0.0.1`/`::1`/`localhost` mit dem erkannten Port.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `npm run dev` (Vite) im Session-Terminal erzeugt binnen 3 s einen Chip „localhost:5173 öffnen“.
  - [ ] AC2 — Ein lauschender Port eines fremden (nicht von der Session gestarteten) Prozesses wird nicht angeboten und ist im Session-Browser blockiert.
  - [ ] AC3 — Endet der Dev-Server-Prozess, wird die Loopback-Ausnahme entfernt.
- **Abhängigkeiten:** SES-019 (siehe 07-sessions-collaboration.md), PRX-007, PRX-011 (siehe 04-sandbox.md)

### BRW-018 — Browser-Traffic durch den Egress-Proxy
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Sämtlicher Browser-Traffic läuft über den Egress-Proxy der Session (Domain-/Methoden-/Pfad-Allowlist, Default-Deny, Block privater IPs außer erlaubten Dev-Server-Ports, Credential-Injection). Chromium vertraut der beton-CA nur in dieser Instanz, nicht systemweit. Implementierung (Proxy-Identität, Start-Flags, Zertifikatsvertrauen): Owner PRX-011; dieses Feature beschreibt die Browser-Sicht (Fehlerseite, Rückmeldung an den Agent).
- **Details:** `--proxy-server=http://127.0.0.1:<port>` (bzw. Unix-Socket-Relay laut PRX-001), `--proxy-bypass-list=<-loopback>` *(damit auch localhost über den Proxy läuft)*. CA-Vertrauen auf allen OS per `--ignore-certificate-errors-spki-list=<SPKI des Instanz-Leaf-Schlüssels>` (PRX-002, PRX-011) *(Annahme, in Sicherheits-Review zu verifizieren)*. Browser-spezifische Allowlist `sandbox.browser.egress_rules` (PRX-011) ergänzt die Session-Allowlist (z.B. für Docs-Seiten), unterliegt aber denselben Policy-Ebenen. WebSocket- und QUIC-Traffic: QUIC wird deaktiviert (`--disable-quic`), damit alles über den Proxy läuft.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Navigation zu einer nicht erlaubten Domain zeigt im Panel eine beton-Fehlerseite „Blockiert durch Egress-Policy“ mit Regelname; der Agent erhält `navigation_blocked`.
  - [ ] AC2 — Navigation zu `http://169.254.169.254/` und zu einer nicht erkannten LAN-IP scheitert.
  - [ ] AC3 — Eine TLS-Seite auf erlaubter Domain lädt ohne Zertifikatswarnung; im System-Trust-Store ist keine beton-CA installiert.
  - [ ] AC4 — Ein Request mit `bt_cred_*`-Platzhalter an den gebundenen Host erhält serverseitig den echten Wert (Proxy-Test), im Browser-Speicher steht nur der Platzhalter.
- **Abhängigkeiten:** PRX-006, PRX-011 (siehe 04-sandbox.md), BRW-003

### BRW-019 — Browser-Policies: Navigation, Formular-Submit, Downloads
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Browser-Aktionen sind Policy-Hook-Punkte (ADR-0008): Navigation (Allowlist), Formular-Submit (Default `ask` bei agent-ausgelösten Submits auf nicht-lokalen Origins), Downloads (Default `ask`), `eval` (BRW-012). Die Entscheidung gilt für Agent- und Seiten-initiierte Aktionen; User-Eingaben im Panel unterliegen nur Navigation/Downloads. BRW-019 ist Owner der Hook-Punkte im Browser (Submit-Erkennung, Download-Pausierung); Regeltyp-Semantik: POL-020, Variablen: POL-004.
- **Details:** CEL-Variablen verbindlich nach POL-004 (Phasen `browser_navigate` und `browser_action`; u. a. `browser.action`, `browser.url`, `browser.host`, `browser.initiator ∈ {agent, user, page}`, `browser.is_form_submit`, `browser.file_name`). Submit-Erkennung: (a) vor Agent-Aktionen (Klick auf Submit-Control oder `type` mit `submit`/Enter in Formular), (b) zusätzlich `Fetch.enable` für `Document`-Requests mit Methode ≠ GET → pausieren, prüfen, fortsetzen/abbrechen. Downloads: `Browser.setDownloadBehavior` → Session-Download-Verzeichnis außerhalb des Workspaces; Freigabe verschiebt die Datei nach `<workspace>/.beton/downloads/`.
  ```yaml
  rules:                             # Format POL-001
    - id: browser-no-external-submit
      on: browser_action
      when: browser.is_form_submit && !(browser.host in ["localhost", "127.0.0.1"])
      action: ask
    - id: browser-nav-allowlist
      on: browser_navigate
      when: browser.initiator == "agent" && !browser.url.matches("^https://(docs\\.rs|developer\\.mozilla\\.org)/")
      action: deny
  ```
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein agent-ausgelöster Submit auf einer externen Testseite erzeugt eine Approval-Card; bei `deny` wird kein Request gesendet (Proxy-Log leer).
  - [ ] AC2 — Ein seiten-initiierter POST (JS `form.submit()`) wird ebenfalls angehalten und geprüft.
  - [ ] AC3 — Ein Download landet erst nach Freigabe im Workspace; abgelehnte Downloads werden gelöscht.
  - [ ] AC4 — Jede Entscheidung erzeugt `policy.decision` mit `browser.*`-Kontext.
- **Abhängigkeiten:** POL-003, POL-004, POL-020 (siehe 03-policies.md), BRW-018

### BRW-020 — Sichtbarkeit für Co-Viewer
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Alle Clients, die eine Session betrachten (lokal mehrere Geräte/Fenster; ab M4 auch geteilte User), sehen denselben Browser-Stream live, inklusive Agent-Aktionen, Picker-Highlights und „Agent steuert“-Indikator. Steuern dürfen nur `drive`-Berechtigte (COL-001, ab M4); Viewer ohne Workspace-Freigabe sehen den Browser nicht *(Annahme: Browser zählt zum Workspace)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Desktop-App und Web-UI zeigen gleichzeitig denselben Browser-Stream; eine Navigation ist in beiden sichtbar.
  - [ ] AC2 — Ein `view`-User ohne `share.workspace_files` erhält keinen Browser-Kanal (`403` beim Abonnieren).
  - [ ] AC3 — Der Picker-Vorgang eines Users ist für andere Viewer als Highlight sichtbar, das Payload geht nur an die Session (nicht an Dritte).
- **Abhängigkeiten:** BRW-005, COL-001 (ab M4, siehe 07-sessions-collaboration.md)

### BRW-021 — Browser in Remote- und Container-Runnern
- **Meilenstein:** M5 · **Priorität:** Should
- **Beschreibung:** Läuft die Session auf einem Remote-Host oder im Docker-/K8s-Runner, startet `beton-browser` Chromium dort (im Runner-Image enthalten oder per BRW-002 nachgeladen); Frames und Input laufen durch den Runner-Tunnel. Nur Modus `embedded` ist verfügbar.
- **Akzeptanzkriterien:**
  - [ ] AC1 — In einer Docker-Runner-Session funktionieren `browser_navigate` und das Panel gegen einen im Container laufenden Dev-Server.
  - [ ] AC2 — Der Fenster-Umschalter ist deaktiviert mit Begründung „Remote-Host“.
- **Abhängigkeiten:** RUN-004, RUN-008, RUN-012 (siehe 10-runners-extensibility.md), BRW-008

## Nicht in v1

- Browser-Unterstützung für Nicht-Chromium-Engines (Firefox/WebKit) – nicht vorgesehen.
- Browser-Aktionen in nativen Mobile-Apps (native Apps sind v2; die PWA zeigt den Stream, Picker nur mit Maus/Trackpad sinnvoll).
- UI-Extensions, die den Browser-Tab erweitern (v2).
