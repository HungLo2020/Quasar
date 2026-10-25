use crate::layout::{LayoutStats, SharedLayout, TextLayout};
use crate::style::{StyleKey, TextStyle};
use crate::{TextError, TextResult};
use cosmic_text::Fallback;
use cosmic_text::FontSystem;
use cosmic_text::fontdb;
use cosmic_text::fontdb::{Database, Source};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use unicode_script::Script;

/// One font file registered with a [`FontStack`].
#[derive(Clone, Debug)]
pub struct FontData {
    /// Human-readable source name, usually the file name.
    pub name: String,
    pub bytes: Arc<[u8]>,
    /// Optional license identifier or notice shipped by the caller.
    pub license: Option<String>,
}

impl FontData {
    pub fn new(name: impl Into<String>, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            name: name.into(),
            bytes: bytes.into(),
            license: None,
        }
    }

    pub fn with_license(mut self, license: impl Into<String>) -> Self {
        self.license = Some(license.into());
        self
    }
}

/// A deterministic, caller-owned collection of font files.
///
/// `FontStack` never scans system fonts. This keeps shaping, fallback and tests
/// independent of the host. Call [`FontStack::add_path`] to load fonts from
/// disk, or register already-owned bytes with [`FontStack::add_font`].
#[derive(Clone, Debug, Default)]
pub struct FontStack {
    fonts: Vec<FontData>,
    default_family: Option<String>,
    default_monospace: Option<String>,
}

impl FontStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_font(&mut self, font: FontData) -> &mut Self {
        self.fonts.push(font);
        self
    }

    pub fn add_bytes(&mut self, name: impl Into<String>, bytes: impl Into<Arc<[u8]>>) -> &mut Self {
        self.fonts.push(FontData::new(name, bytes));
        self
    }

    pub fn add_path(&mut self, path: impl AsRef<Path>) -> TextResult<&mut Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .map_err(|error| TextError::Io(format!("{}: {error}", path.display())))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("font");
        self.fonts.push(FontData::new(name, bytes));
        Ok(self)
    }

    /// Sets the family used for `Family::SansSerif` and as the UI default.
    pub fn with_default_family(&mut self, family: impl Into<String>) -> &mut Self {
        self.default_family = Some(family.into());
        self
    }

    pub fn with_default_monospace(&mut self, family: impl Into<String>) -> &mut Self {
        self.default_monospace = Some(family.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    /// Builds a shaping engine with the given locale for font fallback and
    /// language-sensitive shaping.
    pub fn build(&self, locale: impl Into<String>) -> TextResult<TextEngine> {
        TextEngine::from_stack(self, locale.into())
    }
}

/// Metadata for one registered face.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontFace {
    pub family: String,
    pub post_script_name: String,
    pub weight: u16,
    pub italic: bool,
    pub monospaced: bool,
}

/// Font loading, shaping and cached text layout.
///
/// The engine owns a private `fontdb` database. No system font discovery occurs,
/// so the registered [`FontStack`] is the complete fallback universe.
pub struct TextEngine {
    font_system: FontSystem,
    faces: Vec<FontFace>,
    default_family: String,
    locale: String,
    layouts: HashMap<LayoutKey, SharedLayout>,
    layout_order: VecDeque<LayoutKey>,
    layout_capacity: usize,
    stats: LayoutStats,
}

impl std::fmt::Debug for TextEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextEngine")
            .field("faces", &self.faces)
            .field("default_family", &self.default_family)
            .field("locale", &self.locale)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl TextEngine {
    pub fn from_stack(stack: &FontStack, locale: String) -> TextResult<Self> {
        if stack.fonts.is_empty() {
            return Err(TextError::NoFonts);
        }
        let mut db = Database::new();
        for font in &stack.fonts {
            let source: Source = Source::Binary(Arc::new(FontBytes(font.bytes.clone())));
            let ids = db.load_font_source(source);
            if ids.is_empty() {
                return Err(TextError::InvalidFont(font.name.clone()));
            }
        }
        let faces: Vec<FontFace> = db
            .faces()
            .map(|face| FontFace {
                family: face
                    .families
                    .first()
                    .map(|(family, _)| family.clone())
                    .unwrap_or_else(|| "Unknown".into()),
                post_script_name: face.post_script_name.clone(),
                weight: face.weight.0,
                italic: !matches!(face.style, fontdb::Style::Normal),
                monospaced: face.monospaced,
            })
            .collect();
        let default_family = stack
            .default_family
            .clone()
            .or_else(|| faces.first().map(|face| face.family.clone()))
            .ok_or(TextError::NoFonts)?;
        db.set_sans_serif_family(default_family.clone());
        db.set_serif_family(default_family.clone());
        if let Some(monospace) = stack.default_monospace.clone().or_else(|| {
            faces
                .iter()
                .find(|face| face.monospaced)
                .map(|face| face.family.clone())
        }) {
            db.set_monospace_family(monospace);
        }
        let families: Vec<&'static str> = faces
            .iter()
            .map(|face| Box::leak(face.family.clone().into_boxed_str()) as &'static str)
            .collect();
        let fallback = RegisteredFallback {
            families: Box::leak(families.into_boxed_slice()),
        };
        let font_system =
            FontSystem::new_with_locale_and_db_and_fallback(locale.clone(), db, fallback);
        Ok(Self {
            font_system,
            faces,
            default_family,
            locale,
            layouts: HashMap::new(),
            layout_order: VecDeque::new(),
            layout_capacity: 512,
            stats: LayoutStats::default(),
        })
    }

    pub fn default_family(&self) -> &str {
        &self.default_family
    }

    pub fn locale(&self) -> &str {
        &self.locale
    }

    pub fn faces(&self) -> &[FontFace] {
        &self.faces
    }

    pub fn layout_stats(&self) -> LayoutStats {
        LayoutStats {
            cache_entries: self.layouts.len(),
            ..self.stats
        }
    }

    pub fn set_layout_cache_capacity(&mut self, capacity: usize) {
        self.layout_capacity = capacity.max(16);
        self.trim_layouts();
    }

    /// Shapes and caches text. `width` enables wrapping and alignment within
    /// that physical width; `None` measures the natural single-line width.
    pub fn layout(
        &mut self,
        text: &str,
        style: &TextStyle,
        width: Option<f32>,
    ) -> TextResult<SharedLayout> {
        self.stats.calls += 1;
        style.validate()?;
        if let Some(width) = width
            && (!width.is_finite() || width < 0.0)
        {
            return Err(TextError::InvalidLayout(format!(
                "layout width must be finite and non-negative, got {width}"
            )));
        }
        let key = LayoutKey {
            text: text.to_owned(),
            style: style.key(),
            width_bits: width.map(f32::to_bits),
        };
        if let Some(layout) = self.layouts.get(&key) {
            self.stats.cache_hits += 1;
            return Ok(layout.clone());
        }
        let layout = Arc::new(TextLayout::shape(
            &mut self.font_system,
            text,
            style,
            width,
        )?);
        if self.layouts.len() >= self.layout_capacity {
            self.trim_layouts();
        }
        self.layouts.insert(key.clone(), layout.clone());
        self.layout_order.push_back(key);
        Ok(layout)
    }

    /// Measures the natural (unwrapped) extent of text.
    pub fn measure(&mut self, text: &str, style: &TextStyle) -> TextResult<(f32, f32)> {
        let layout = self.layout(text, style, None)?;
        Ok((layout.width(), layout.height()))
    }

    pub(crate) fn font_system_mut(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    fn trim_layouts(&mut self) {
        while self.layouts.len() >= self.layout_capacity {
            let Some(key) = self.layout_order.pop_front() else {
                break;
            };
            self.layouts.remove(&key);
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct LayoutKey {
    text: String,
    style: StyleKey,
    width_bits: Option<u32>,
}

/// Deterministic fallback over the caller's registered faces. Cosine's
/// platform lists are not consulted, so a given `FontStack` produces the same
/// face order on every host.
struct RegisteredFallback {
    families: &'static [&'static str],
}

impl Fallback for RegisteredFallback {
    fn common_fallback(&self) -> &[&'static str] {
        self.families
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, _script: Script, _locale: &str) -> &[&'static str] {
        self.families
    }
}

struct FontBytes(Arc<[u8]>);

impl AsRef<[u8]> for FontBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for FontBytes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FontBytes")
            .field("len", &self.0.len())
            .finish()
    }
}
