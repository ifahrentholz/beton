<!-- Generiert von `cargo xtask codegen` aus beton_server::ProblemCode. Nicht von Hand ändern. -->

# Fehlercodes (RFC 9457)

Jede Fehlerantwort ist ein Problem-Objekt mit `type: urn:beton:problem:<code>`. Spezifikation: PROTO-011 in [06-data-sync-protocol.md](../spec/06-data-sync-protocol.md#proto-011--fehlerformat-rfc-9457).

| Code | Status | Titel |
| --- | --- | --- |
| `bad_request` | 400 | Ungültige Anfrage |
| `validation_failed` | 400 | Validierung fehlgeschlagen |
| `invalid_cursor` | 400 | Ungültiger Cursor |
| `invalid_limit` | 400 | Ungültiges Limit |
| `unknown_command` | 400 | Unbekanntes Kommando |
| `unauthorized` | 401 | Anmeldung erforderlich |
| `invalid_code` | 401 | Einmal-Code ungültig oder abgelaufen |
| `forbidden` | 403 | Keine Berechtigung |
| `host_not_allowed` | 403 | Host nicht erlaubt |
| `origin_not_allowed` | 403 | Origin nicht erlaubt |
| `loopback_only` | 403 | Nur über Loopback erreichbar |
| `path_outside_workspace` | 403 | Pfad außerhalb des Workspace |
| `not_found` | 404 | Nicht gefunden |
| `agent_not_found` | 404 | Agent nicht gefunden |
| `feature_disabled` | 404 | Funktion nicht aktiviert |
| `method_not_allowed` | 405 | Methode nicht erlaubt |
| `conflict` | 409 | Konflikt |
| `capability_unsupported` | 409 | Vom Harness nicht unterstützt |
| `no_active_turn` | 409 | Kein laufender Turn |
| `seq_conflict` | 409 | Sequenzkonflikt |
| `stale_epoch` | 409 | Veraltete Epoch |
| `seq_ahead` | 409 | from_seq liegt hinter head_seq |
| `not_a_git_repo` | 409 | Kein Git-Repository |
| `worktree_dirty` | 409 | Worktree hat uncommittete Änderungen |
| `worktree_unpushed` | 409 | Branch hat ungepushte Commits |
| `tombstoned` | 410 | Gelöscht |
| `precondition_failed` | 412 | Vorbedingung nicht erfüllt |
| `payload_too_large` | 413 | Anfrage zu groß |
| `unsupported_media_type` | 415 | Medientyp nicht unterstützt |
| `idempotency_key_reused` | 422 | Idempotency-Key mit anderem Inhalt wiederverwendet |
| `base_not_found` | 422 | Base-Branch nicht auflösbar |
| `agent_invalid` | 422 | Agent ungültig |
| `invalid_param` | 422 | Parameter ungültig |
| `params_required` | 422 | Pflichtparameter fehlen |
| `harness_incompatible` | 422 | Ziel-Harness passt nicht |
| `rate_limited` | 429 | Zu viele Anfragen |
| `internal` | 500 | Interner Fehler |
| `blob_corrupt` | 500 | Blob beschädigt |
| `unavailable` | 503 | Vorübergehend nicht verfügbar |
