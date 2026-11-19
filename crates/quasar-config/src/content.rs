//! Data-driven content tables with validation and reload-safe registries.
//!
//! A content table is a TOML (or JSON) file holding an array of records
//! under the table's name:
//!
//! ```toml
//! [[weapons]]
//! id = "laser"
//! damage = 3
//! ammo = "cell"
//! ```
//!
//! Records implement [`ContentRecord`]: an id, field checks through a
//! [`Validator`] and references to records of other tables. Loading reports
//! every problem at once (bad fields, duplicate ids, dangling references)
//! with the table, record and field it belongs to.
//!
//! A [`ContentRegistry`] owns every table of a game. It hands out cheap
//! [`ContentSnapshot`]s (`Arc`s) instead of living in a global, so a reload
//! ([`ContentRegistry::reload_str`], [`ContentRegistry::reload_paths`])
//! validates the new data against every other table first and only then
//! swaps it in; a broken edit keeps the previous content running. Systems
//! holding an older snapshot keep a consistent view until they fetch a new
//! one ([`ContentSnapshot::revision`] tells them when to).
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// One problem found while loading content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentError {
    pub table: String,
    /// Record id (or `#index` when the id is missing).
    pub record: Option<String>,
    pub field: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.table)?;
        if let Some(record) = &self.record {
            write!(f, "[{record}]")?;
        }
        if let Some(field) = &self.field {
            write!(f, ".{field}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// Every problem of one load, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentErrors(pub Vec<ContentError>);

impl std::fmt::Display for ContentErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}
impl std::error::Error for ContentErrors {}

/// Collects field errors and references for one record.
pub struct Validator {
    table: String,
    record: String,
    errors: Vec<ContentError>,
    references: Vec<(String, String, String)>,
}

impl Validator {
    fn new(table: &str, record: &str) -> Self {
        Self {
            table: table.to_owned(),
            record: record.to_owned(),
            errors: Vec::new(),
            references: Vec::new(),
        }
    }
    /// Records an error on `field` unless `ok`.
    pub fn check(&mut self, ok: bool, field: &str, message: impl Into<String>) -> &mut Self {
        if !ok {
            self.errors.push(ContentError {
                table: self.table.clone(),
                record: Some(self.record.clone()),
                field: Some(field.to_owned()),
                message: message.into(),
            });
        }
        self
    }
    /// `value` must lie in `min..=max`.
    pub fn range<T: PartialOrd + std::fmt::Display>(
        &mut self,
        field: &str,
        value: T,
        min: T,
        max: T,
    ) -> &mut Self {
        let ok = value >= min && value <= max;
        self.check(ok, field, format!("{value} is outside {min}..={max}"))
    }
    /// `value` must be finite (no NaN or infinity).
    pub fn finite(&mut self, field: &str, value: f64) -> &mut Self {
        self.check(value.is_finite(), field, "must be a finite number")
    }
    pub fn non_empty(&mut self, field: &str, value: &str) -> &mut Self {
        self.check(!value.trim().is_empty(), field, "must not be empty")
    }
    /// `id` must exist in table `table` once every table is loaded.
    pub fn reference(&mut self, field: &str, table: &str, id: &str) -> &mut Self {
        self.references
            .push((field.to_owned(), table.to_owned(), id.to_owned()));
        self
    }
    /// [`Self::reference`] for an optional id.
    pub fn optional_reference(&mut self, field: &str, table: &str, id: Option<&str>) -> &mut Self {
        if let Some(id) = id {
            self.reference(field, table, id);
        }
        self
    }
}

/// One record of a content table.
pub trait ContentRecord: serde::de::DeserializeOwned + Send + Sync + 'static {
    fn id(&self) -> &str;
    /// Field checks and references; the default accepts everything.
    fn validate(&self, _validator: &mut Validator) {}
}

/// File format of table sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentFormat {
    #[cfg(feature = "toml")]
    Toml,
    #[cfg(feature = "json")]
    Json,
}

impl ContentFormat {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            #[cfg(feature = "toml")]
            "toml" => Some(Self::Toml),
            #[cfg(feature = "json")]
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Validated records of one type, in file order, indexed by id.
#[derive(Debug)]
pub struct ContentTable<T> {
    name: String,
    records: Vec<T>,
    index: BTreeMap<String, usize>,
    references: Vec<(String, String, String, String)>,
}

impl<T: ContentRecord> ContentTable<T> {
    /// Parses and validates `source`. Cross-table references are checked by
    /// [`ContentRegistry`] (or [`Self::check_references`]).
    pub fn parse(name: &str, source: &str, format: ContentFormat) -> Result<Self, ContentErrors> {
        let outcome = Self::parse_outcome(name, source, format);
        match outcome.table {
            Some(table) => Ok(table),
            None => Err(ContentErrors(outcome.errors)),
        }
    }

    /// Parses as far as possible: even when some records fail, the ids and
    /// references of the records that did deserialise are kept so the
    /// registry can report dangling references in the same pass.
    fn parse_outcome(name: &str, source: &str, format: ContentFormat) -> ParseOutcome<T> {
        let document_failed = |errors: ContentErrors| ParseOutcome {
            table: None,
            ids: None,
            references: Vec::new(),
            errors: errors.0,
        };
        match Self::parse_records(name, source, format) {
            Ok(outcome) => outcome,
            Err(errors) => document_failed(errors),
        }
    }

    fn parse_records(
        name: &str,
        source: &str,
        format: ContentFormat,
    ) -> Result<ParseOutcome<T>, ContentErrors> {
        let fail = |message: String| {
            ContentErrors(vec![ContentError {
                table: name.to_owned(),
                record: None,
                field: None,
                message,
            }])
        };
        // Records are deserialised one at a time so one bad record does not
        // hide the others' errors.
        let raw: Vec<serde_json::Value> = match format {
            #[cfg(feature = "toml")]
            ContentFormat::Toml => {
                let mut doc: BTreeMap<String, toml::Value> =
                    toml::from_str(source).map_err(|e| fail(e.to_string()))?;
                let list = doc.remove(name).unwrap_or(toml::Value::Array(Vec::new()));
                if let Some(other) = doc.keys().next() {
                    return Err(fail(format!("unexpected top-level key `{other}`")));
                }
                let toml::Value::Array(list) = list else {
                    return Err(fail(format!("`{name}` must be an array of tables")));
                };
                list.into_iter()
                    .map(|v| serde_json::to_value(v).map_err(|e| fail(e.to_string())))
                    .collect::<Result<_, _>>()?
            }
            #[cfg(feature = "json")]
            ContentFormat::Json => {
                let mut doc: BTreeMap<String, serde_json::Value> =
                    serde_json::from_str(source).map_err(|e| fail(e.to_string()))?;
                let list = doc
                    .remove(name)
                    .unwrap_or(serde_json::Value::Array(Vec::new()));
                if let Some(other) = doc.keys().next() {
                    return Err(fail(format!("unexpected top-level key `{other}`")));
                }
                match list {
                    serde_json::Value::Array(list) => list,
                    _ => return Err(fail(format!("`{name}` must be an array"))),
                }
            }
        };
        let mut errors = Vec::new();
        let mut records = Vec::with_capacity(raw.len());
        let mut index = BTreeMap::new();
        let mut references = Vec::new();
        // Ids of every record, including ones that failed to deserialise
        // but carry a string id, so they are not reported as dangling.
        let mut ids = BTreeSet::new();
        for (position, value) in raw.into_iter().enumerate() {
            let raw_id = value.get("id").and_then(|v| v.as_str()).map(str::to_owned);
            if let Some(id) = &raw_id {
                ids.insert(id.clone());
            }
            let label = raw_id.unwrap_or_else(|| format!("#{position}"));
            let record: T = match serde_json::from_value(value) {
                Ok(record) => record,
                Err(e) => {
                    errors.push(ContentError {
                        table: name.to_owned(),
                        record: Some(label),
                        field: None,
                        message: e.to_string(),
                    });
                    continue;
                }
            };
            let id = record.id().to_owned();
            let mut validator = Validator::new(name, &id);
            validator.non_empty("id", &id);
            record.validate(&mut validator);
            errors.append(&mut validator.errors);
            if index.insert(id.clone(), records.len()).is_some() {
                errors.push(ContentError {
                    table: name.to_owned(),
                    record: Some(id.clone()),
                    field: Some("id".into()),
                    message: "duplicate id".into(),
                });
            }
            for (field, table, target) in validator.references {
                references.push((id.clone(), field, table, target));
            }
            ids.insert(id);
            records.push(record);
        }
        if errors.is_empty() {
            Ok(ParseOutcome {
                ids: Some(ids),
                references: references.clone(),
                table: Some(Self {
                    name: name.to_owned(),
                    records,
                    index,
                    references,
                }),
                errors,
            })
        } else {
            Ok(ParseOutcome {
                table: None,
                ids: Some(ids),
                references,
                errors,
            })
        }
    }

    /// Dangling references, given every table's id set.
    pub fn check_references(&self, ids: &BTreeMap<String, BTreeSet<String>>) -> Vec<ContentError> {
        dangling(&self.name, &self.references, ids, &BTreeSet::new())
    }
}

/// `(record, field, table, target)` reference rows.
type References = Vec<(String, String, String, String)>;

struct ParseOutcome<T> {
    table: Option<ContentTable<T>>,
    /// `None` when the document itself failed to parse.
    ids: Option<BTreeSet<String>>,
    references: References,
    errors: Vec<ContentError>,
}

/// References whose target id is missing. Targets in `unknown` tables
/// (which failed to parse, so their ids are not known) are skipped.
fn dangling(
    name: &str,
    references: &References,
    ids: &BTreeMap<String, BTreeSet<String>>,
    unknown: &BTreeSet<String>,
) -> Vec<ContentError> {
    references
        .iter()
        .filter(|(_, _, table, target)| {
            !unknown.contains(table) && !ids.get(table).is_some_and(|set| set.contains(target))
        })
        .map(|(record, field, table, target)| ContentError {
            table: name.to_owned(),
            record: Some(record.clone()),
            field: Some(field.clone()),
            message: format!("unknown {table} id `{target}`"),
        })
        .collect()
}

impl<T> ContentTable<T> {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn get(&self, id: &str) -> Option<&T> {
        self.index.get(id).map(|&i| &self.records[i])
    }
    /// Position of `id` in file order (stable numeric handles).
    pub fn position(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }
    pub fn records(&self) -> &[T] {
        &self.records
    }
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(String::as_str)
    }
}

type Parsed = Arc<dyn Any + Send + Sync>;
type ParseFn = dyn Fn(&str, ContentFormat) -> Result<(Parsed, BTreeSet<String>, RefCheck), Failed>
    + Send
    + Sync;

/// A table that failed: its errors, plus (when the document parsed) the ids
/// and reference rows of the records that did.
struct Failed {
    errors: Vec<ContentError>,
    partial: Option<(BTreeSet<String>, References)>,
}
type RefCheck = Arc<
    dyn Fn(&BTreeMap<String, BTreeSet<String>>, &BTreeSet<String>) -> Vec<ContentError>
        + Send
        + Sync,
>;

#[derive(Clone)]
struct Entry {
    value: Parsed,
    ids: BTreeSet<String>,
    references: RefCheck,
}

/// An immutable view of every table at one revision.
#[derive(Clone)]
pub struct ContentSnapshot {
    revision: u64,
    tables: BTreeMap<String, Entry>,
}

impl ContentSnapshot {
    /// Increments on every successful load or reload.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// The table `name` as records of type `T`; `None` when missing or of
    /// another type.
    pub fn table<T: ContentRecord>(&self, name: &str) -> Option<&ContentTable<T>> {
        self.tables.get(name)?.value.downcast_ref()
    }
    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }
}

/// Every content table of a game; see the module docs.
pub struct ContentRegistry {
    parsers: BTreeMap<String, Arc<ParseFn>>,
    paths: BTreeMap<String, PathBuf>,
    current: RwLock<Arc<ContentSnapshot>>,
}

impl Default for ContentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentRegistry {
    pub fn new() -> Self {
        Self {
            parsers: BTreeMap::new(),
            paths: BTreeMap::new(),
            current: RwLock::new(Arc::new(ContentSnapshot {
                revision: 0,
                tables: BTreeMap::new(),
            })),
        }
    }

    /// Declares table `name` holding records of type `T`.
    pub fn register<T: ContentRecord>(&mut self, name: &str) -> &mut Self {
        let table = name.to_owned();
        let parse: Arc<ParseFn> = Arc::new(move |source, format| {
            let outcome = ContentTable::<T>::parse_outcome(&table, source, format);
            let Some(parsed) = outcome.table else {
                return Err(Failed {
                    errors: outcome.errors,
                    partial: outcome.ids.map(|ids| (ids, outcome.references)),
                });
            };
            let ids = parsed.ids().map(str::to_owned).collect();
            let parsed = Arc::new(parsed);
            let checker = parsed.clone();
            let references: RefCheck = Arc::new(move |ids, unknown| {
                dangling(&checker.name, &checker.references, ids, unknown)
            });
            Ok((parsed as Parsed, ids, references))
        });
        self.parsers.insert(name.to_owned(), parse);
        self
    }

    /// The current snapshot (cheap: an `Arc` clone).
    pub fn snapshot(&self) -> Arc<ContentSnapshot> {
        self.current.read().expect("content lock").clone()
    }

    /// Loads several tables at once and validates references across them
    /// and the tables already loaded. Nothing changes on error.
    pub fn load_all(&self, sources: &[(&str, &str, ContentFormat)]) -> Result<u64, ContentErrors> {
        let base = self.snapshot();
        let mut tables = base.tables.clone();
        let mut errors = Vec::new();
        // Failed tables: their partial ids/references, or `None` when the
        // document did not parse (its ids are unknown).
        let mut failed: BTreeMap<String, Option<(BTreeSet<String>, References)>> = BTreeMap::new();
        for (name, source, format) in sources {
            let Some(parse) = self.parsers.get(*name) else {
                errors.push(ContentError {
                    table: (*name).to_owned(),
                    record: None,
                    field: None,
                    message: "table is not registered".into(),
                });
                continue;
            };
            match parse(source, *format) {
                Ok((value, ids, references)) => {
                    tables.insert(
                        (*name).to_owned(),
                        Entry {
                            value,
                            ids,
                            references,
                        },
                    );
                }
                Err(mut e) => {
                    errors.append(&mut e.errors);
                    tables.remove(*name);
                    failed.insert((*name).to_owned(), e.partial);
                }
            }
        }
        // Check references even when some records failed, so one pass
        // reports bad fields and dangling ids together.
        let mut ids: BTreeMap<String, BTreeSet<String>> = tables
            .iter()
            .map(|(name, entry)| (name.clone(), entry.ids.clone()))
            .collect();
        let mut unknown = BTreeSet::new();
        for (name, partial) in &failed {
            match partial {
                Some((partial_ids, _)) => {
                    ids.insert(name.clone(), partial_ids.clone());
                }
                None => {
                    unknown.insert(name.clone());
                }
            }
        }
        for entry in tables.values() {
            errors.extend((entry.references)(&ids, &unknown));
        }
        for (name, partial) in &failed {
            if let Some((_, references)) = partial {
                errors.extend(dangling(name, references, &ids, &unknown));
            }
        }
        if !errors.is_empty() {
            return Err(ContentErrors(errors));
        }
        let mut current = self.current.write().expect("content lock");
        let revision = current.revision + 1;
        *current = Arc::new(ContentSnapshot { revision, tables });
        Ok(revision)
    }

    /// Replaces one table from source text after full validation.
    pub fn reload_str(
        &self,
        name: &str,
        source: &str,
        format: ContentFormat,
    ) -> Result<u64, ContentErrors> {
        self.load_all(&[(name, source, format)])
    }

    /// Loads tables from files (format by extension) and remembers each
    /// path for [`Self::reload_paths`].
    pub fn load_files(&mut self, files: &[(&str, &Path)]) -> Result<u64, ContentErrors> {
        let mut sources = Vec::new();
        let mut errors = Vec::new();
        for (name, path) in files {
            match read(name, path) {
                Ok((text, format)) => sources.push(((*name).to_owned(), text, format)),
                Err(e) => errors.push(e),
            }
        }
        if !errors.is_empty() {
            return Err(ContentErrors(errors));
        }
        let borrowed: Vec<_> = sources
            .iter()
            .map(|(n, t, f)| (n.as_str(), t.as_str(), *f))
            .collect();
        let revision = self.load_all(&borrowed)?;
        for (name, path) in files {
            self.paths.insert((*name).to_owned(), path.to_path_buf());
        }
        Ok(revision)
    }

    /// Reloads every table whose file is in `changed` (for example the
    /// paths an asset watcher reported). Returns `Ok(None)` when no table
    /// was affected.
    pub fn reload_paths<'a>(
        &self,
        changed: impl IntoIterator<Item = &'a Path>,
    ) -> Result<Option<u64>, ContentErrors> {
        let changed: Vec<PathBuf> = changed
            .into_iter()
            .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
            .collect();
        let mut sources = Vec::new();
        let mut errors = Vec::new();
        for (name, path) in &self.paths {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if changed.contains(&canonical) {
                match read(name, path) {
                    Ok((text, format)) => sources.push((name.clone(), text, format)),
                    Err(e) => errors.push(e),
                }
            }
        }
        if !errors.is_empty() {
            return Err(ContentErrors(errors));
        }
        if sources.is_empty() {
            return Ok(None);
        }
        let borrowed: Vec<_> = sources
            .iter()
            .map(|(n, t, f)| (n.as_str(), t.as_str(), *f))
            .collect();
        self.load_all(&borrowed).map(Some)
    }
}

fn read(name: &str, path: &Path) -> Result<(String, ContentFormat), ContentError> {
    let error = |message: String| ContentError {
        table: name.to_owned(),
        record: None,
        field: None,
        message,
    };
    let format = ContentFormat::from_path(path).ok_or_else(|| {
        error(format!(
            "{}: content files must be .toml or .json",
            path.display()
        ))
    })?;
    let text =
        std::fs::read_to_string(path).map_err(|e| error(format!("{}: {e}", path.display())))?;
    Ok((text, format))
}
