<!-- Generiert von `cargo xtask codegen` aus crates/beton-store/migrations/sqlite. Nicht von Hand ändern. -->

# Datenmodell (lokal, SQLite)

Schema-Version 14. Spezifikation: DATA-001 in [06-data-sync-protocol.md](../spec/06-data-sync-protocol.md#data-001--datenmodell--entitäten).

```mermaid
erDiagram
    approvals {
        TEXT id PK
        TEXT org_id
        TEXT session_id PK,FK
        INTEGER requested_seq
        TEXT kind
        TEXT subject
        TEXT options
        TEXT expires_at
        TEXT on_timeout
        TEXT status
        TEXT decision "nullable"
        INTEGER resolved_seq "nullable"
    }
    audit_entries {
        TEXT id PK
        TEXT org_id
        TEXT at
        TEXT actor_id
        TEXT action
        TEXT target_kind
        TEXT target_id
        TEXT details
        INTEGER seq "nullable"
        TEXT session_id "nullable"
        TEXT host_id "nullable"
        INTEGER count
        TEXT prev_hash "nullable"
        TEXT hash "nullable"
    }
    audit_heads {
        TEXT org_id PK
        INTEGER seq
        TEXT hash
    }
    audit_usage_windows {
        TEXT org_id PK
        TEXT secret_ref PK
        TEXT session_id PK
        TEXT host_id PK
        TEXT actor_id
        TEXT purpose
        TEXT window_start
        TEXT last_at
        INTEGER count
    }
    blob_refs {
        TEXT org_id FK
        TEXT session_id PK,FK
        TEXT sha256 PK,FK
        TEXT created_at
    }
    blobs {
        TEXT org_id PK,FK
        TEXT sha256 PK
        INTEGER size
        TEXT created_at
    }
    event_raw {
        TEXT org_id
        TEXT session_id PK,FK
        INTEGER seq PK,FK
        TEXT raw
        TEXT created_at
    }
    events {
        TEXT org_id
        TEXT session_id PK,FK
        INTEGER seq PK
        TEXT id
        TEXT ts
        TEXT actor_kind
        TEXT actor_id "nullable"
        TEXT actor
        TEXT type
        TEXT payload "nullable"
        TEXT payload_ref "nullable"
        TEXT turn_id "nullable"
        TEXT causation_id "nullable"
        INTEGER epoch
        INTEGER redacted
    }
    idempotency_keys {
        TEXT org_id PK,FK
        TEXT key PK
        TEXT request_hash
        INTEGER status
        TEXT content_type
        BLOB body
        TEXT created_at
        TEXT location "nullable"
    }
    inbox_items {
        TEXT id PK
        TEXT org_id
        TEXT session_id PK,FK
        TEXT kind
        TEXT status
        TEXT approval_id "nullable"
        TEXT title
        TEXT summary
        TEXT detail
        INTEGER created_seq
        TEXT created_at
        INTEGER resolved_seq "nullable"
        TEXT resolved_at "nullable"
        TEXT resolution "nullable"
    }
    nodes {
        TEXT id PK
        TEXT org_id FK
        TEXT name
        TEXT created_at
    }
    orgs {
        TEXT id PK
        TEXT name
        TEXT created_at
    }
    policy_set_snapshots {
        TEXT org_id PK
        TEXT session_id PK,FK
        TEXT policy_set_hash PK
        TEXT manifest
        TEXT created_at
    }
    policy_shadow_daily {
        TEXT org_id PK
        TEXT session_id PK,FK
        TEXT day PK
        TEXT scope PK
        TEXT set_name PK
        TEXT rule_id PK
        TEXT verdict PK
        INTEGER hits
        INTEGER last_seq
        TEXT last_at
    }
    policy_user_state {
        TEXT org_id PK
        TEXT user_id PK
        TEXT scope PK
        TEXT day PK
        TEXT key PK
        TEXT value
        TEXT updated_at
    }
    search_docs {
        INTEGER id PK
        TEXT org_id
        TEXT session_id FK
        TEXT doc
        TEXT body
    }
    secrets {
        TEXT org_id PK
        TEXT secret_ref PK
        TEXT scope
        TEXT provider
        TEXT account
        TEXT name
        TEXT kind
        TEXT backend
        TEXT created_at
        TEXT updated_at
        TEXT last_used_at "nullable"
    }
    session_file_imports {
        TEXT session_id PK,FK
        TEXT org_id
        TEXT export_sha256
        TEXT source_session_id
        TEXT exported_at
        TEXT imported_at
    }
    session_imports {
        TEXT org_id PK
        TEXT host_id PK,FK
        TEXT harness PK
        TEXT vendor_session_id PK
        TEXT session_id FK
        TEXT imported_at
    }
    session_user_state {
        TEXT org_id
        TEXT session_id PK,FK
        TEXT user_id PK,FK
        INTEGER pinned
        INTEGER read_seq
        TEXT updated_at
    }
    sessions {
        TEXT id PK
        TEXT org_id FK
        TEXT owner_id FK
        TEXT project_id "nullable"
        TEXT parent_id FK "nullable"
        TEXT kind
        TEXT harness
        TEXT home_node_id FK
        INTEGER epoch
        INTEGER head_seq
        TEXT created_at
        TEXT updated_at
        TEXT title
        TEXT status
        INTEGER archived
        INTEGER cost_micro
        TEXT last_activity_at
        TEXT worktree_path "nullable"
        TEXT worktree_branch "nullable"
        TEXT worktree_base "nullable"
        TEXT worktree_base_sha "nullable"
        TEXT title_source
    }
    tombstones {
        TEXT org_id
        TEXT kind PK
        TEXT id PK
        TEXT owner_id
        TEXT deleted_at
        TEXT deleted_by
    }
    usage_daily {
        TEXT org_id
        TEXT session_id PK,FK
        TEXT day PK
        TEXT harness PK
        TEXT model PK
        INTEGER input_tokens
        INTEGER output_tokens
        INTEGER cache_read_tokens
        INTEGER cache_write_tokens
        INTEGER cost_micro
        INTEGER events
    }
    usage_records {
        TEXT org_id
        TEXT session_id PK,FK
        INTEGER seq PK
        TEXT ts
        TEXT day
        TEXT project_id "nullable"
        TEXT user_id
        TEXT harness
        TEXT model
        TEXT auth_source
        TEXT purpose "nullable"
        INTEGER input_tokens
        INTEGER output_tokens
        INTEGER cache_read_tokens
        INTEGER cache_write_tokens
        INTEGER cost_micro "nullable"
        TEXT cost_source
        TEXT pricing_version "nullable"
        INTEGER equivalent_micro "nullable"
    }
    usage_subscription_windows {
        TEXT org_id
        TEXT session_id PK,FK
        TEXT harness
        TEXT vendor PK
        TEXT window_name PK
        REAL used_pct
        TEXT resets_at
        INTEGER seq
        TEXT ts
    }
    users {
        TEXT id PK
        TEXT org_id FK
        TEXT display_name
        TEXT created_at
    }
    sessions ||--o{ approvals : "session_id"
    blobs ||--o{ blob_refs : "org_id, sha256"
    sessions ||--o{ blob_refs : "session_id"
    orgs ||--o{ blobs : "org_id"
    events ||--o{ event_raw : "session_id, seq"
    sessions ||--o{ events : "session_id"
    orgs ||--o{ idempotency_keys : "org_id"
    sessions ||--o{ inbox_items : "session_id"
    orgs ||--o{ nodes : "org_id"
    sessions ||--o{ policy_set_snapshots : "session_id"
    sessions ||--o{ policy_shadow_daily : "session_id"
    sessions ||--o{ search_docs : "session_id"
    sessions ||--o{ session_file_imports : "session_id"
    nodes ||--o{ session_imports : "host_id"
    sessions ||--o{ session_imports : "session_id"
    sessions ||--o{ session_user_state : "session_id"
    users ||--o{ session_user_state : "user_id"
    nodes ||--o{ sessions : "home_node_id"
    orgs ||--o{ sessions : "org_id"
    users ||--o{ sessions : "owner_id"
    sessions ||--o{ sessions : "parent_id"
    sessions ||--o{ usage_daily : "session_id"
    sessions ||--o{ usage_records : "session_id"
    sessions ||--o{ usage_subscription_windows : "session_id"
    orgs ||--o{ users : "org_id"
```
