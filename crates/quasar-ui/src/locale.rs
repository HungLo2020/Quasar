//! Localisation: string tables, fallback chains, formatting and QA hooks.
//!
//! [`UiText::keyed`](crate::UiText::keyed) labels resolve through the
//! strings installed on a [`Ui`](crate::Ui). A [`Localization`] is an ordered
//! chain of [`StringTable`]s (for example `fr-CA`, then `fr`, then `en`);
//! [`Ui::set_localization`](crate::Ui::set_localization) installs the merged
//! result and bumps [`Ui::locale_revision`](crate::Ui::locale_revision) so a
//! game knows to re-format dynamic text such as scores. Widgets re-measure
//! on the next layout, so longer translations reflow automatically.
//!
//! Tables load from a small line format (no dependencies):
//!
//! ```text
//! # comment
//! menu.start = Start game
//! hud.ammo = Ammo: {count}
//! hud.enemies.one = {count} enemy left
//! hud.enemies.other = {count} enemies left
//! ```
//!
//! Values may use `\n`, `\t`, `\\` and `\{` escapes. `{name}` placeholders
//! are filled by [`Localization::format`]; [`Localization::format_count`]
//! picks a `.one` / `.other` (or `.zero`) variant first.
use std::collections::BTreeMap;

/// Parse error with a 1-based line number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocaleError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for LocaleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}
impl std::error::Error for LocaleError {}

/// Strings of one language.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StringTable {
    /// Language tag, for example `en` or `fr-CA`.
    pub language: String,
    entries: BTreeMap<String, String>,
}

impl StringTable {
    pub fn new(language: impl Into<String>) -> Self {
        Self {
            language: language.into(),
            entries: BTreeMap::new(),
        }
    }

    /// Parses the `key = value` line format (see the module docs).
    /// Duplicate keys and malformed lines are errors.
    pub fn parse(language: impl Into<String>, source: &str) -> Result<Self, LocaleError> {
        let mut table = Self::new(language);
        for (index, raw) in source.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                return Err(LocaleError {
                    line,
                    message: "expected `key = value`".into(),
                });
            };
            let key = key.trim();
            if key.is_empty()
                || !key
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'))
            {
                return Err(LocaleError {
                    line,
                    message: format!("invalid key {key:?}"),
                });
            }
            let value = unescape(value.trim()).map_err(|message| LocaleError { line, message })?;
            if table.entries.insert(key.to_owned(), value).is_some() {
                return Err(LocaleError {
                    line,
                    message: format!("duplicate key {key:?}"),
                });
            }
        }
        Ok(table)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.entries.insert(key.into(), value.into());
        self
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
    /// Keys of `reference` this table lacks (translation coverage checks).
    pub fn missing_from(&self, reference: &StringTable) -> Vec<String> {
        reference
            .entries
            .keys()
            .filter(|k| !self.entries.contains_key(*k))
            .cloned()
            .collect()
    }

    /// Pseudo-localisation of `base` for layout QA: accented letters and
    /// about 40% longer text, bracketed so clipped strings are obvious.
    /// Placeholders are kept intact.
    pub fn pseudo(base: &StringTable) -> Self {
        let mut table = Self::new("qps-ploc");
        for (key, value) in &base.entries {
            let mut out = String::from("[");
            let mut in_placeholder = false;
            let mut letters = 0usize;
            for c in value.chars() {
                match c {
                    '{' => in_placeholder = true,
                    '}' => in_placeholder = false,
                    _ => {}
                }
                if in_placeholder || c == '}' {
                    out.push(c);
                    continue;
                }
                out.push(accent(c));
                if c.is_alphabetic() {
                    letters += 1;
                }
            }
            for i in 0..letters.div_ceil(5) * 2 {
                out.push(if i % 2 == 0 { '~' } else { '·' });
            }
            out.push(']');
            table.entries.insert(key.clone(), out);
        }
        table
    }
}

fn accent(c: char) -> char {
    match c {
        'a' => 'á',
        'e' => 'é',
        'i' => 'í',
        'o' => 'ö',
        'u' => 'ü',
        'c' => 'ç',
        'n' => 'ñ',
        'y' => 'ý',
        'A' => 'Å',
        'E' => 'É',
        'I' => 'Î',
        'O' => 'Ø',
        'U' => 'Û',
        'C' => 'Ç',
        'N' => 'Ñ',
        other => other,
    }
}

fn unescape(value: &str) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('{') => out.push('\u{e000}'), // literal brace marker
            Some('}') => out.push('\u{e001}'),
            other => return Err(format!("unknown escape \\{}", other.unwrap_or(' '))),
        }
    }
    Ok(out)
}

/// An ordered fallback chain of string tables.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Localization {
    tables: Vec<StringTable>,
}

impl Localization {
    /// `tables` in priority order: the first table that has a key wins.
    pub fn new(tables: Vec<StringTable>) -> Self {
        Self { tables }
    }
    /// The highest-priority language tag.
    pub fn language(&self) -> Option<&str> {
        self.tables.first().map(|t| t.language.as_str())
    }
    pub fn tables(&self) -> &[StringTable] {
        &self.tables
    }
    /// Picks tables for `requested` (for example `fr-CA`) from `available`:
    /// the exact tag, then its language prefix, then `default` (for example
    /// `en`). Missing tags are skipped.
    pub fn negotiate(requested: &str, available: &[StringTable], default: &str) -> Self {
        let mut tags = vec![requested.to_owned()];
        if let Some((prefix, _)) = requested.split_once(['-', '_']) {
            tags.push(prefix.to_owned());
        }
        tags.push(default.to_owned());
        let mut tables = Vec::new();
        for tag in tags {
            if let Some(table) = available
                .iter()
                .find(|t| t.language.eq_ignore_ascii_case(&tag))
                && !tables
                    .iter()
                    .any(|t: &StringTable| t.language == table.language)
            {
                tables.push(table.clone());
            }
        }
        Self { tables }
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.tables.iter().find_map(|t| t.get(key))
    }
    /// Every key and its winning value.
    pub fn merged(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for table in self.tables.iter().rev() {
            for (k, v) in table.iter() {
                out.insert(k.to_owned(), display(v));
            }
        }
        out
    }
    /// `key`'s value (or `fallback`) with `{name}` placeholders replaced.
    /// Unknown placeholders stay visible so missing arguments are noticed.
    pub fn format(&self, key: &str, fallback: &str, args: &[(&str, &str)]) -> String {
        let template = self.get(key).unwrap_or(fallback);
        fill(template, args)
    }
    /// [`Self::format`] with a `count` argument, choosing `key.zero` (when
    /// `count` is 0 and it exists), `key.one` (count 1) or `key.other`, then
    /// plain `key`.
    pub fn format_count(
        &self,
        key: &str,
        fallback: &str,
        count: i64,
        args: &[(&str, &str)],
    ) -> String {
        let variant = match count {
            0 if self.get(&format!("{key}.zero")).is_some() => "zero",
            1 | -1 => "one",
            _ => "other",
        };
        let count_text = count.to_string();
        let mut all: Vec<(&str, &str)> = vec![("count", &count_text)];
        all.extend_from_slice(args);
        let template = self
            .get(&format!("{key}.{variant}"))
            .or_else(|| self.get(key))
            .unwrap_or(fallback);
        fill(template, &all)
    }
}

fn display(template: &str) -> String {
    template.replace('\u{e000}', "{").replace('\u{e001}', "}")
}

fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, value)) => out.push_str(value),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    display(&out)
}
