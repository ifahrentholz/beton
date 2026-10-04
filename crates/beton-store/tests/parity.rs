//! DATA-003 AC1: Jede Migrationsnummer existiert in SQLite und Postgres, und beide Dialekte
//! ergeben äquivalente Schemas (Tabellen, Spalten, NOT NULL, Schlüssel, Indizes; Typen
//! dürfen sich unterscheiden).
//!
//! Postgres wird hier nicht ausgeführt (das kommt mit DATA-004 in M4), sondern die DDL
//! strukturell gelesen. Damit der Leser selbst stimmt, muss er für die SQLite-Dateien
//! exakt das Schema liefern, das SQLite nach den Migrationen tatsächlich hat.

#![allow(clippy::unwrap_used)] // Testcode: Panics sind hier die Fehlermeldung.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use beton_store::schema::{Column, ForeignKey, Index, Schema, Table, sqlite_schema};

fn migration_files(dialect: &str) -> BTreeMap<String, PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("migrations")
        .join(dialect);
    std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), p))
        .collect()
}

#[test]
fn data_003_ac1_every_migration_exists_in_both_dialects() {
    let sqlite = migration_files("sqlite");
    let postgres = migration_files("postgres");
    assert_eq!(
        sqlite.keys().collect::<Vec<_>>(),
        postgres.keys().collect::<Vec<_>>(),
        "Migrationen müssen in beiden Dialekten mit gleichem Namen existieren"
    );
    for (i, name) in sqlite.keys().enumerate() {
        let number: i64 = name[..4].parse().unwrap();
        assert_eq!(
            number,
            i as i64 + 1,
            "{name}: Nummerierung lückenlos ab 0001"
        );
        assert_eq!(&name[4..5], "_", "{name}: Format NNNN_<name>.sql");
    }
    assert_eq!(sqlite.len() as i64, beton_store::SCHEMA_VERSION);
}

#[tokio::test]
async fn data_003_ac1_schemas_are_equivalent() {
    let parse_all = |dialect: &str| {
        let mut schema = Schema::default();
        for path in migration_files(dialect).values() {
            apply_ddl(&mut schema, &std::fs::read_to_string(path).unwrap());
        }
        schema.without_types()
    };
    let sqlite_parsed = parse_all("sqlite");
    let actual = sqlite_schema().await.unwrap().without_types();
    assert_eq!(sqlite_parsed, actual, "DDL-Leser weicht von SQLite ab");
    let postgres_parsed = parse_all("postgres");
    for (name, table) in &actual.tables {
        assert_eq!(
            postgres_parsed.tables.get(name),
            Some(table),
            "Tabelle {name} unterscheidet sich zwischen SQLite und Postgres"
        );
    }
    assert_eq!(postgres_parsed, actual);
}

// ---------------------------------------------------------------------------
// Minimaler DDL-Leser für den Stil unserer Migrationen
// ---------------------------------------------------------------------------

fn apply_ddl(schema: &mut Schema, sql: &str) {
    let without_comments: String = sql
        .lines()
        .map(|l| l.split("--").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    for stmt in without_comments
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let words: Vec<&str> = stmt.split_whitespace().collect();
        // Volltext-Indizes sind dialektspezifisch (SES-012): SQLite FTS5, Postgres GIN.
        if words.starts_with(&["CREATE", "VIRTUAL", "TABLE"]) || stmt.contains(" USING GIN ") {
            continue;
        }
        match words.as_slice() {
            ["CREATE", "TABLE", name, ..] => {
                let body = between_parens(stmt);
                schema.tables.insert((*name).to_owned(), parse_table(body));
            }
            ["CREATE", "INDEX", name, "ON", table, ..]
            | ["CREATE", "UNIQUE", "INDEX", name, "ON", table, ..] => {
                let unique = words[1] == "UNIQUE";
                let index = Index {
                    columns: idents(between_parens(stmt)),
                    unique,
                };
                schema
                    .tables
                    .get_mut(*table)
                    .unwrap_or_else(|| panic!("Index auf unbekannte Tabelle {table}"))
                    .indexes
                    .insert((*name).to_owned(), index);
            }
            ["DROP", "TABLE", name] => {
                schema.tables.remove(*name);
            }
            ["ALTER", "TABLE", from, "RENAME", "TO", to] => {
                let table = schema
                    .tables
                    .remove(*from)
                    .unwrap_or_else(|| panic!("Umbenennen unbekannter Tabelle {from}"));
                schema.tables.insert((*to).to_owned(), table);
            }
            // Datenübernahme ändert das Schema nicht.
            ["INSERT", "INTO", ..] => {}
            _ => panic!("Nicht unterstützte Anweisung – Leser erweitern:\n{stmt}"),
        }
    }
}

fn parse_table(body: &str) -> Table {
    let mut table = Table::default();
    for item in split_top_level(body) {
        let words: Vec<&str> = item.split_whitespace().collect();
        match words.as_slice() {
            ["PRIMARY", "KEY", ..] => table.primary_key = idents(between_parens(item)),
            ["FOREIGN", "KEY", ..] => {
                let (cols, rest) = item.split_once("REFERENCES").unwrap();
                let rest = rest.trim();
                let target = rest.split_whitespace().next().unwrap().to_owned();
                table.foreign_keys.push(ForeignKey {
                    columns: idents(between_parens(cols)),
                    table: target,
                    references: idents(between_parens(rest)),
                });
            }
            ["CHECK", ..] => {}
            [name, ty, rest @ ..] => {
                let rest = rest.join(" ");
                let pk = rest.contains("PRIMARY KEY");
                if pk {
                    table.primary_key = vec![(*name).to_owned()];
                }
                if let Some((_, target)) = rest.split_once("REFERENCES") {
                    let target = target.trim();
                    table.foreign_keys.push(ForeignKey {
                        columns: vec![(*name).to_owned()],
                        table: target.split_whitespace().next().unwrap().to_owned(),
                        references: idents(between_parens(target)),
                    });
                }
                table.columns.push(Column {
                    name: (*name).to_owned(),
                    ty: (*ty).to_owned(),
                    not_null: pk || rest.contains("NOT NULL"),
                });
            }
            _ => panic!("Unbekannter Tabellenbestandteil: {item}"),
        }
    }
    // Spalten im Primärschlüssel sind immer NOT NULL.
    for c in &mut table.columns {
        if table.primary_key.contains(&c.name) {
            c.not_null = true;
        }
    }
    table
}

fn between_parens(s: &str) -> &str {
    let start = s.find('(').expect("Klammer fehlt");
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &s[start + 1..start + i];
                }
            }
            _ => {}
        }
    }
    panic!("Klammer nicht geschlossen: {s}")
}

fn split_top_level(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(body[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(body[start..].trim());
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

fn idents(list: &str) -> Vec<String> {
    list.split(',').map(|s| s.trim().to_owned()).collect()
}
