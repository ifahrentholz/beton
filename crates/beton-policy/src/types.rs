//! CEL-Typen und die Deklaration der Kontext-Variablen je Phase (POL-004).
//!
//! Die Deklaration ist die Grundlage der Typprüfung beim Laden (POL-002 AC1): Jede Variable
//! und jedes Feld hat einen Typ; phasenspezifische Variablen sind nur in ihren Phasen
//! sichtbar. Quelle sind die Rust-Typen in [`crate::vars`] (POL-004 AC1); der Runner füllt
//! sie zur Laufzeit.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::LazyLock;

use crate::Phase;

/// Ein CEL-Typ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Bool,
    Int,
    Uint,
    Double,
    String,
    Bytes,
    Null,
    Timestamp,
    Duration,
    List(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    /// Ein deklariertes Objekt (z. B. `session`) mit festen Feldern.
    Object(&'static str),
    /// Dynamisch, erst zur Laufzeit geprüft (z. B. `tool.args`).
    Dyn,
}

impl Ty {
    pub fn list(t: Ty) -> Ty {
        Ty::List(Box::new(t))
    }

    pub fn map(k: Ty, v: Ty) -> Ty {
        Ty::Map(Box::new(k), Box::new(v))
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Int | Ty::Uint | Ty::Double)
    }

    /// Kann ein Wert dieses Typs dort stehen, wo `other` erwartet wird?
    pub fn assignable_to(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Dyn, _) | (_, Ty::Dyn) => true,
            (Ty::List(a), Ty::List(b)) => a.assignable_to(b),
            (Ty::Map(ka, va), Ty::Map(kb, vb)) => ka.assignable_to(kb) && va.assignable_to(vb),
            (a, b) => a == b,
        }
    }

    /// Gemeinsamer Typ zweier Zweige (Ternär, Listen-Literale); `Dyn`, wenn sie abweichen.
    pub fn join(&self, other: &Ty) -> Ty {
        match (self, other) {
            (a, b) if a == b => a.clone(),
            (Ty::Null, b) | (b, Ty::Null) => b.clone(),
            (Ty::List(a), Ty::List(b)) => Ty::list(a.join(b)),
            (Ty::Map(ka, va), Ty::Map(kb, vb)) => Ty::map(ka.join(kb), va.join(vb)),
            _ => Ty::Dyn,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Bool => f.write_str("bool"),
            Ty::Int => f.write_str("int"),
            Ty::Uint => f.write_str("uint"),
            Ty::Double => f.write_str("double"),
            Ty::String => f.write_str("string"),
            Ty::Bytes => f.write_str("bytes"),
            Ty::Null => f.write_str("null"),
            Ty::Timestamp => f.write_str("timestamp"),
            Ty::Duration => f.write_str("duration"),
            Ty::List(t) => write!(f, "list({t})"),
            Ty::Map(k, v) => write!(f, "map({k}, {v})"),
            Ty::Object(name) => f.write_str(name),
            Ty::Dyn => f.write_str("dyn"),
        }
    }
}

/// Eine Kontext-Variable der obersten Ebene.
#[derive(Debug, Clone)]
pub struct VarDecl {
    pub name: &'static str,
    pub ty: Ty,
    /// Phasen, in denen die Variable sichtbar ist; leer = alle Phasen.
    pub phases: Vec<Phase>,
    pub doc: String,
}

/// Eine Zeile der Referenztabelle: Pfad, Typ, Phasen, Beschreibung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclRow {
    pub path: String,
    pub ty: String,
    pub phases: Vec<Phase>,
    pub doc: String,
}

/// Alle Deklarationen: Variablen und Objekttypen.
#[derive(Debug, Default)]
pub struct Decls {
    pub vars: Vec<VarDecl>,
    pub objects: BTreeMap<&'static str, Vec<(&'static str, Ty)>>,
    /// Beschreibung je `objekt.feld`.
    docs: BTreeMap<String, String>,
}

impl Decls {
    /// Typ der Variable `name` in `phase`. `Err` mit den Phasen, in denen sie gilt.
    pub fn var(&self, name: &str, phase: Phase) -> Result<&Ty, Option<&[Phase]>> {
        let mut other = None;
        for v in self.vars.iter().filter(|v| v.name == name) {
            if v.phases.is_empty() || v.phases.contains(&phase) {
                return Ok(&v.ty);
            }
            other = Some(v.phases.as_slice());
        }
        Err(other)
    }

    /// Typ des Felds `field` im Objekt `object`.
    pub fn field(&self, object: &str, field: &str) -> Option<&Ty> {
        self.objects
            .get(object)?
            .iter()
            .find(|(n, _)| *n == field)
            .map(|(_, t)| t)
    }

    /// Alle Blätter (`session.cost_usd`, `start.sandbox.enabled`, `now`) in Deklarations-
    /// reihenfolge der Variablen, Felder alphabetisch (POL-004 AC1, Referenztabelle).
    pub fn rows(&self) -> Vec<DeclRow> {
        let mut out = Vec::new();
        for v in &self.vars {
            self.rows_of(v.name, &v.ty, &v.phases, &v.doc, &mut out, 0);
        }
        out
    }

    fn rows_of(
        &self,
        path: &str,
        ty: &Ty,
        phases: &[Phase],
        doc: &str,
        out: &mut Vec<DeclRow>,
        depth: usize,
    ) {
        if let Ty::Object(name) = ty
            && depth < 8
            && let Some(fields) = self.objects.get(name)
        {
            for (field, fty) in fields {
                let doc = self
                    .docs
                    .get(&format!("{name}.{field}"))
                    .cloned()
                    .unwrap_or_default();
                self.rows_of(
                    &format!("{path}.{field}"),
                    fty,
                    phases,
                    &doc,
                    out,
                    depth + 1,
                );
            }
            return;
        }
        out.push(DeclRow {
            path: path.to_owned(),
            ty: ty.to_string(),
            phases: phases.to_vec(),
            doc: doc.to_owned(),
        });
    }

    /// Leitet die Deklaration aus den Rust-Typen in [`crate::vars`] ab (über deren
    /// JSON-Schema): `string` mit `format: date-time` → `timestamp`, `number` → `double`,
    /// `integer` → `int`, Listen und Maps rekursiv, Strukturen → Objekte, `serde_json::Value`
    /// → `dyn`. `Option` (`null` im Schema) wird ignoriert: fehlende Felder sind `null`.
    pub fn from_vars(defs: &[crate::vars::VarDef]) -> Self {
        let mut generator = schemars::SchemaGenerator::default();
        let schemas: Vec<serde_json::Value> = defs
            .iter()
            .map(|d| (d.schema)(&mut generator).as_value().clone())
            .collect();
        let definitions = generator.definitions().clone();
        let mut decls = Decls::default();
        for (def, schema) in defs.iter().zip(&schemas) {
            let ty = decls.convert(schema, &definitions);
            decls.vars.push(VarDecl {
                name: def.name,
                ty,
                phases: def.phases.to_vec(),
                doc: def.doc.to_owned(),
            });
        }
        decls
    }

    fn convert(
        &mut self,
        schema: &serde_json::Value,
        definitions: &serde_json::Map<String, serde_json::Value>,
    ) -> Ty {
        use serde_json::Value as J;
        let J::Object(map) = schema else {
            return Ty::Dyn; // `true`: beliebiger Wert
        };
        if let Some(J::String(r)) = map.get("$ref") {
            let name = r.rsplit('/').next().unwrap_or_default();
            return match definitions.get(name) {
                Some(def) => self.object(name, def, definitions),
                None => Ty::Dyn,
            };
        }
        for key in ["anyOf", "oneOf"] {
            if let Some(J::Array(options)) = map.get(key) {
                let real: Vec<&J> = options
                    .iter()
                    .filter(|o| o.get("type") != Some(&J::String("null".into())))
                    .collect();
                return match real.as_slice() {
                    [one] => self.convert(one, definitions),
                    _ => Ty::Dyn,
                };
            }
        }
        let ty = match map.get("type") {
            Some(J::String(t)) => t.as_str(),
            Some(J::Array(types)) => types
                .iter()
                .filter_map(J::as_str)
                .find(|t| *t != "null")
                .unwrap_or("null"),
            _ => return Ty::Dyn,
        };
        match ty {
            "string" if map.get("format") == Some(&J::String("date-time".into())) => Ty::Timestamp,
            "string" => Ty::String,
            "number" => Ty::Double,
            "integer" => Ty::Int,
            "boolean" => Ty::Bool,
            "array" => Ty::list(
                map.get("items")
                    .map_or(Ty::Dyn, |i| self.convert(i, definitions)),
            ),
            "object" if map.contains_key("properties") => {
                // Inline-Objekt ohne Namen: als Map behandeln wäre ungenau; nicht vorgesehen.
                Ty::Dyn
            }
            "object" => Ty::map(
                Ty::String,
                map.get("additionalProperties")
                    .map_or(Ty::Dyn, |v| self.convert(v, definitions)),
            ),
            _ => Ty::Dyn,
        }
    }

    fn object(
        &mut self,
        name: &str,
        def: &serde_json::Value,
        definitions: &serde_json::Map<String, serde_json::Value>,
    ) -> Ty {
        // Objektnamen leben so lange wie das Programm (einmalig beim Aufbau von DECLS).
        let key: &'static str = match self.objects.keys().find(|k| **k == name) {
            Some(k) => k,
            None => Box::leak(name.to_owned().into_boxed_str()),
        };
        if self.objects.contains_key(key) {
            return Ty::Object(key);
        }
        self.objects.insert(key, Vec::new());
        let mut fields = Vec::new();
        if let Some(props) = def.get("properties").and_then(serde_json::Value::as_object) {
            for (field, schema) in props {
                let fname: &'static str = Box::leak(field.clone().into_boxed_str());
                let ty = self.convert(schema, definitions);
                if let Some(doc) = schema
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                {
                    self.docs.insert(format!("{key}.{field}"), doc.to_owned());
                }
                fields.push((fname, ty));
            }
        }
        self.objects.insert(key, fields);
        Ty::Object(key)
    }
}

/// Die Deklarationen aus POL-004 (03-policies.md), abgeleitet aus [`crate::vars`].
pub static DECLS: LazyLock<Decls> = LazyLock::new(|| Decls::from_vars(&crate::vars::var_defs()));
