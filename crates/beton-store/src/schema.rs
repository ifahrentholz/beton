//! Schema-Introspektion und ER-Diagramm (DATA-001 AC1).
//!
//! `cargo xtask codegen` erzeugt daraus `docs/generated/er-diagram.md`; der Paritätstest
//! (DATA-003 AC1) vergleicht dieselbe Struktur mit den Postgres-Migrationen.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use sqlx::{Connection, Row, SqliteConnection};

use crate::error::Result;
use crate::migrate::MIGRATIONS;

/// Tabellen, Spalten, Fremdschlüssel und Indizes – dialektunabhängig normalisiert.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schema {
    pub tables: BTreeMap<String, Table>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    /// In Definitionsreihenfolge.
    pub columns: Vec<Column>,
    pub primary_key: Vec<String>,
    pub foreign_keys: Vec<ForeignKey>,
    pub indexes: BTreeMap<String, Index>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// Deklarierter Typ, z. B. `TEXT` (nur für das Diagramm, nicht für die Parität).
    pub ty: String,
    pub not_null: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ForeignKey {
    pub columns: Vec<String>,
    pub table: String,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub columns: Vec<String>,
    pub unique: bool,
}

impl Schema {
    /// Normalisiert für Vergleiche: Fremdschlüssel sortiert, Typen ausgeblendet.
    pub fn without_types(&self) -> Self {
        let mut out = self.clone();
        for table in out.tables.values_mut() {
            for c in &mut table.columns {
                c.ty.clear();
            }
            table.foreign_keys.sort();
        }
        out
    }

    /// ER-Diagramm in Mermaid-Syntax.
    pub fn to_mermaid(&self) -> String {
        let mut out = String::from("erDiagram\n");
        for (name, table) in &self.tables {
            let fk_cols: Vec<&String> =
                table.foreign_keys.iter().flat_map(|f| &f.columns).collect();
            let _ = writeln!(out, "    {name} {{");
            for c in &table.columns {
                let mut keys = Vec::new();
                if table.primary_key.contains(&c.name) {
                    keys.push("PK");
                }
                if fk_cols.contains(&&c.name) {
                    keys.push("FK");
                }
                let ty = if c.ty.is_empty() {
                    "ANY"
                } else {
                    c.ty.as_str()
                };
                let _ = write!(out, "        {ty} {}", c.name);
                if !keys.is_empty() {
                    let _ = write!(out, " {}", keys.join(","));
                }
                if !c.not_null && !table.primary_key.contains(&c.name) {
                    let _ = write!(out, " \"nullable\"");
                }
                out.push('\n');
            }
            out.push_str("    }\n");
        }
        for (name, table) in &self.tables {
            let mut fks = table.foreign_keys.clone();
            fks.sort();
            for fk in fks {
                let _ = writeln!(
                    out,
                    "    {} ||--o{{ {name} : \"{}\"",
                    fk.table,
                    fk.columns.join(", ")
                );
            }
        }
        out
    }
}

/// Schema, das die eingebetteten SQLite-Migrationen erzeugen (in einer In-Memory-Datenbank).
pub async fn sqlite_schema() -> Result<Schema> {
    let mut conn = SqliteConnection::connect("sqlite::memory:").await?;
    for m in MIGRATIONS {
        sqlx::raw_sql(m.sql).execute(&mut conn).await?;
    }
    let schema = introspect(&mut conn).await?;
    conn.close().await?;
    Ok(schema)
}

/// Liest das Schema einer SQLite-Datenbank (ohne interne und Verwaltungstabellen).
pub(crate) async fn introspect(conn: &mut SqliteConnection) -> Result<Schema> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
         AND name != 'schema_migrations' ORDER BY name",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut schema = Schema::default();
    for name in names {
        let mut table = Table::default();
        let mut pk: Vec<(i64, String)> = Vec::new();
        for row in sqlx::query("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?)")
            .bind(&name)
            .fetch_all(&mut *conn)
            .await?
        {
            let column: String = row.try_get("name")?;
            let pk_pos: i64 = row.try_get("pk")?;
            if pk_pos > 0 {
                pk.push((pk_pos, column.clone()));
            }
            table.columns.push(Column {
                name: column,
                ty: row.try_get("type")?,
                not_null: row.try_get::<i64, _>("notnull")? != 0 || pk_pos > 0,
            });
        }
        pk.sort();
        table.primary_key = pk.into_iter().map(|(_, c)| c).collect();

        let mut fks: BTreeMap<i64, ForeignKey> = BTreeMap::new();
        for row in sqlx::query(
            "SELECT id, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list(?) ORDER BY id, seq",
        )
        .bind(&name)
        .fetch_all(&mut *conn)
        .await?
        {
            let fk = fks.entry(row.try_get("id")?).or_insert_with(|| ForeignKey {
                columns: Vec::new(),
                table: String::new(),
                references: Vec::new(),
            });
            fk.table = row.try_get("table")?;
            fk.columns.push(row.try_get("from")?);
            fk.references.push(row.try_get("to")?);
        }
        table.foreign_keys = fks.into_values().collect();

        for row in sqlx::query("SELECT name, \"unique\", origin FROM pragma_index_list(?)")
            .bind(&name)
            .fetch_all(&mut *conn)
            .await?
        {
            // Nur explizit angelegte Indizes; PK-/UNIQUE-Autoindizes stecken im Tabellenschema.
            if row.try_get::<String, _>("origin")? != "c" {
                continue;
            }
            let index: String = row.try_get("name")?;
            let columns: Vec<String> =
                sqlx::query_scalar("SELECT name FROM pragma_index_info(?) ORDER BY seqno")
                    .bind(&index)
                    .fetch_all(&mut *conn)
                    .await?;
            table.indexes.insert(
                index,
                Index {
                    columns,
                    unique: row.try_get::<i64, _>("unique")? != 0,
                },
            );
        }
        schema.tables.insert(name, table);
    }
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn data_001_ac1_er_diagram_covers_all_tables() {
        let schema = sqlite_schema().await.unwrap();
        let mermaid = schema.to_mermaid();
        for table in schema.tables.keys() {
            assert!(mermaid.contains(&format!("    {table} {{")), "{table}");
        }
        assert!(mermaid.contains("users ||--o{ sessions : \"owner_id\""));
        assert!(mermaid.contains("TEXT id PK"));
    }

    #[tokio::test]
    async fn data_001_every_domain_table_carries_org_id() {
        let schema = sqlite_schema().await.unwrap();
        for (name, table) in &schema.tables {
            if name == "orgs" {
                continue;
            }
            assert!(
                table
                    .columns
                    .iter()
                    .any(|c| c.name == "org_id" && c.not_null),
                "{name} ohne org_id"
            );
        }
    }
}
