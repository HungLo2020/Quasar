//! Developer consoles: a command grammar with completion, and an overlay.
//!
//! [`CommandSet`] holds commands written as syntax strings
//! (`give <item> [count]`): plain words are literals, `<kind>` is a
//! required argument and `[kind]` an optional one, where each kind is
//! registered with [`CommandSet::kind`] as a choice list (which may depend
//! on the game state and the arguments before it), a number range or any
//! word. [`CommandSet::parse`] validates a typed line and
//! [`CommandSet::suggest`] lists completions for the word being typed plus
//! the usage of every command the line still matches, in the style of
//! Minecraft's chat commands. The game decides what a matched command does.
//!
//! [`Console`] is the overlay state: a [`LineEditor`], scrolling log,
//! history (Up/Down), Tab completion and suggestion selection. It builds
//! its widgets into any [`Ui`] each frame ([`Console::build`]), so it works
//! in UIs that rebuild every frame.
//!
//! ```
//! use quasar_ui::console::{ArgKind, Choice, CommandSet};
//!
//! let mut commands = CommandSet::<Vec<&str>>::new();
//! commands
//!     .kind("item", ArgKind::choices(|items: &Vec<&str>, _| {
//!         items.iter().map(|i| Choice::new(*i, "")).collect()
//!     }))
//!     .kind("count", ArgKind::Number { min: 1, max: 99 })
//!     .command("give <item> [count]", "Add items");
//! let items = vec!["sword", "shield"];
//! let matched = commands.parse("give sword 3", &items).unwrap();
//! assert_eq!(matched.args, vec![Some("sword".to_string()), Some("3".to_string())]);
//! let suggestions = commands.suggest("give sh", &items);
//! assert_eq!(suggestions.items[0].word, "shield");
//! ```
use crate::{
    Color, Edges, LayoutItem, Length, LineEditor, NodeId, NodeStyle, PanelLayout, StackDirection,
    TextMeasure, TextStyle, Ui, UiInput, UiResult, UiText, Vec2, Widget, WidgetColors,
};
use std::collections::{BTreeMap, VecDeque};

/// One completion: the word to insert and a short description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub word: String,
    pub help: String,
}

impl Choice {
    pub fn new(word: impl Into<String>, help: impl Into<String>) -> Self {
        Self {
            word: word.into(),
            help: help.into(),
        }
    }
}

type ChoiceFn<C> = dyn Fn(&C, &[String]) -> Vec<Choice> + Send + Sync;

/// What an argument kind accepts.
pub enum ArgKind<C> {
    /// One of the listed words (case-insensitive). The closure receives the
    /// game context and the command's earlier argument values.
    Choices(Box<ChoiceFn<C>>),
    /// An integer in `min..=max`.
    Number { min: i64, max: i64 },
    /// Any single word.
    Word,
}

impl<C> ArgKind<C> {
    pub fn choices(list: impl Fn(&C, &[String]) -> Vec<Choice> + Send + Sync + 'static) -> Self {
        Self::Choices(Box::new(list))
    }
    /// A fixed list of words.
    pub fn words(words: &[&str]) -> Self
    where
        C: 'static,
    {
        let words: Vec<Choice> = words.iter().map(|w| Choice::new(*w, "")).collect();
        Self::Choices(Box::new(move |_, _| words.clone()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Slot {
    Literal(String),
    Arg { kind: String, optional: bool },
}

struct Command {
    syntax: String,
    help: String,
    slots: Vec<Slot>,
}

/// A matched command: its index (in [`CommandSet::command`] order), syntax
/// and one value per argument slot (`None` for missing optionals). Number
/// arguments are validated but returned as typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matched {
    pub command: usize,
    pub syntax: String,
    pub args: Vec<Option<String>>,
}

impl Matched {
    /// Argument `index` parsed as an integer.
    pub fn number(&self, index: usize) -> Option<i64> {
        self.args.get(index)?.as_deref()?.parse().ok()
    }
    pub fn word(&self, index: usize) -> Option<&str> {
        self.args.get(index)?.as_deref()
    }
}

/// Completions for the end of a line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Suggestions {
    /// Byte offset where the word being completed starts.
    pub from: usize,
    pub items: Vec<Choice>,
    /// Usage of every command the line still matches.
    pub usage: Vec<String>,
}

/// A command grammar. See the module docs.
pub struct CommandSet<C> {
    commands: Vec<Command>,
    kinds: BTreeMap<String, ArgKind<C>>,
    prefix: String,
}

impl<C> Default for CommandSet<C> {
    fn default() -> Self {
        Self::new()
    }
}

fn words(line: &str, prefix: &str) -> Vec<String> {
    line.trim_start()
        .strip_prefix(prefix)
        .unwrap_or(line.trim_start())
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

impl<C> CommandSet<C> {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            kinds: BTreeMap::new(),
            prefix: "/".into(),
        }
    }
    /// The optional leading character(s) of a command line (default `/`).
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }
    pub fn kind(&mut self, name: &str, kind: ArgKind<C>) -> &mut Self {
        self.kinds.insert(name.to_owned(), kind);
        self
    }
    /// Adds a command; returns `self` for chaining. Its index (for
    /// [`Matched::command`]) is the number of commands added before it.
    pub fn command(&mut self, syntax: &str, help: &str) -> &mut Self {
        let slots = syntax
            .split_whitespace()
            .map(|word| {
                if let Some(kind) = word.strip_prefix('<').and_then(|w| w.strip_suffix('>')) {
                    Slot::Arg {
                        kind: kind.to_owned(),
                        optional: false,
                    }
                } else if let Some(kind) = word.strip_prefix('[').and_then(|w| w.strip_suffix(']'))
                {
                    Slot::Arg {
                        kind: kind.to_owned(),
                        optional: true,
                    }
                } else {
                    Slot::Literal(word.to_lowercase())
                }
            })
            .collect();
        self.commands.push(Command {
            syntax: syntax.to_owned(),
            help: help.to_owned(),
            slots,
        });
        self
    }
    pub fn len(&self) -> usize {
        self.commands.len()
    }
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
    /// `/syntax — help` for every command.
    pub fn help(&self) -> Vec<String> {
        self.commands
            .iter()
            .map(|c| format!("{}{} — {}", self.prefix, c.syntax, c.help))
            .collect()
    }

    fn candidates(&self, slot: &Slot, ctx: &C, args: &[String]) -> Vec<Choice> {
        match slot {
            Slot::Literal(word) => vec![Choice::new(word.clone(), "")],
            Slot::Arg { kind, .. } => match self.kinds.get(kind) {
                Some(ArgKind::Choices(list)) => list(ctx, args),
                _ => Vec::new(),
            },
        }
    }

    /// The value `word` gives `slot`, or `None` when it does not fit.
    fn accept(&self, slot: &Slot, word: &str, ctx: &C, args: &[String]) -> Option<String> {
        match slot {
            Slot::Literal(literal) => literal.eq_ignore_ascii_case(word).then(|| literal.clone()),
            Slot::Arg { kind, .. } => match self.kinds.get(kind)? {
                ArgKind::Choices(list) => list(ctx, args)
                    .into_iter()
                    .find(|c| c.word.eq_ignore_ascii_case(word))
                    .map(|c| c.word),
                ArgKind::Number { min, max } => word
                    .parse::<i64>()
                    .ok()
                    .filter(|n| (min..=max).contains(&n))
                    .map(|_| word.to_owned()),
                ArgKind::Word => Some(word.to_owned()),
            },
        }
    }

    /// Matches `words` against one command: argument values, or how many
    /// words fitted before it failed and why.
    fn match_command(
        &self,
        command: &Command,
        words: &[String],
        ctx: &C,
    ) -> Result<Vec<Option<String>>, (usize, String)> {
        let usage = format!("Usage: {}{}", self.prefix, command.syntax);
        let mut args: Vec<String> = Vec::new();
        let mut values: Vec<Option<String>> = Vec::new();
        for (i, slot) in command.slots.iter().enumerate() {
            let Some(word) = words.get(i) else {
                match slot {
                    Slot::Arg { optional: true, .. } => {
                        values.push(None);
                        continue;
                    }
                    Slot::Arg { kind, .. } => return Err((i, format!("Missing {kind}. {usage}"))),
                    Slot::Literal(word) => return Err((i, format!("Missing '{word}'. {usage}"))),
                }
            };
            match self.accept(slot, word, ctx, &args) {
                Some(value) => {
                    if let Slot::Arg { .. } = slot {
                        args.push(value.clone());
                        values.push(Some(value));
                    }
                }
                None => {
                    let what = match slot {
                        Slot::Literal(literal) => format!("'{literal}'"),
                        Slot::Arg { kind, .. } => kind.clone(),
                    };
                    let message = match slot {
                        Slot::Arg { kind, .. }
                            if matches!(self.kinds.get(kind), Some(ArgKind::Number { .. })) =>
                        {
                            let Some(ArgKind::Number { min, max }) = self.kinds.get(kind) else {
                                unreachable!()
                            };
                            format!(
                                "{what} must be a number from {min} to {max}, not '{word}'. {usage}"
                            )
                        }
                        _ => format!("Unknown {what} '{word}'. {usage}"),
                    };
                    return Err((i, message));
                }
            }
        }
        if words.len() > command.slots.len() {
            return Err((command.slots.len(), format!("Too many words. {usage}")));
        }
        Ok(values)
    }

    /// Parses and validates a typed line (the prefix is optional). The
    /// error names the problem in the command that matched furthest.
    pub fn parse(&self, line: &str, ctx: &C) -> Result<Matched, String> {
        let words = words(line, &self.prefix);
        if words.is_empty() {
            return Err(format!("Type a command ({}help lists them)", self.prefix));
        }
        let mut best: Option<(usize, String)> = None;
        for (index, command) in self.commands.iter().enumerate() {
            match self.match_command(command, &words, ctx) {
                Ok(args) => {
                    return Ok(Matched {
                        command: index,
                        syntax: command.syntax.clone(),
                        args,
                    });
                }
                Err((matched, message)) => {
                    if best.as_ref().is_none_or(|(m, _)| matched > *m) {
                        best = Some((matched, message));
                    }
                }
            }
        }
        match best {
            Some((matched, message)) if matched > 0 => Err(message),
            _ => Err(format!(
                "Unknown command '{}' ({}help lists them)",
                words[0], self.prefix
            )),
        }
    }

    /// Completions for the word at the end of `line` (or the next word
    /// after a space) and the usage of every command it still matches.
    pub fn suggest(&self, line: &str, ctx: &C) -> Suggestions {
        let trailing = line.trim().is_empty()
            || line.ends_with(char::is_whitespace)
            || line.trim_end() == self.prefix;
        let all = words(line, &self.prefix);
        let (done, partial) = if trailing || all.is_empty() {
            (&all[..], String::new())
        } else {
            (&all[..all.len() - 1], all[all.len() - 1].clone())
        };
        let from = line.len() - if trailing { 0 } else { partial.len() };
        let partial = partial.to_lowercase();
        let mut out = Suggestions {
            from,
            ..Suggestions::default()
        };
        for command in &self.commands {
            let Some(slot) = command.slots.get(done.len()) else {
                continue;
            };
            let mut args: Vec<String> = Vec::new();
            let fits = done.iter().zip(&command.slots).all(|(word, slot)| {
                match self.accept(slot, word, ctx, &args) {
                    Some(value) => {
                        if let Slot::Arg { .. } = slot {
                            args.push(value);
                        }
                        true
                    }
                    None => false,
                }
            });
            if !fits {
                continue;
            }
            let options: Vec<Choice> = self
                .candidates(slot, ctx, &args)
                .into_iter()
                .filter(|c| c.word.to_lowercase().starts_with(&partial))
                .collect();
            let free = matches!(slot, Slot::Arg { kind, .. }
                if !matches!(self.kinds.get(kind), Some(ArgKind::Choices(_))));
            if options.is_empty() && !free {
                continue;
            }
            let usage = format!("{}{}", self.prefix, command.syntax);
            if !out.usage.contains(&usage) {
                out.usage.push(usage);
            }
            for option in options {
                if !out.items.iter().any(|c| c.word == option.word) {
                    out.items.push(option);
                }
            }
        }
        out
    }

    /// `line` with suggestion `index` inserted and a trailing space.
    pub fn complete(&self, line: &str, ctx: &C, index: usize) -> Option<String> {
        let suggestions = self.suggest(line, ctx);
        let choice = suggestions.items.get(index)?;
        let mut out = line[..suggestions.from].to_owned();
        out.push_str(&choice.word);
        out.push(' ');
        Some(out)
    }
}

/// How a console log line is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    /// The command as typed.
    Echo,
    Info,
    Error,
}

/// What [`Console::handle`] produced this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleOutcome {
    None,
    /// A valid command was entered (already echoed and in history).
    Command(Matched),
    /// The console asked to close (Escape on an empty line).
    Close,
}

/// Overlay state: line, log, history and suggestion selection.
pub struct Console {
    pub line: LineEditor,
    log: VecDeque<(String, LogKind)>,
    history: Vec<String>,
    history_index: Option<usize>,
    selected: Option<usize>,
    max_log: usize,
    visible_log: usize,
    suggestions: Suggestions,
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

impl Console {
    pub fn new() -> Self {
        Self {
            line: LineEditor::new("").with_max_len(240),
            log: VecDeque::new(),
            history: Vec::new(),
            history_index: None,
            selected: None,
            max_log: 200,
            visible_log: 8,
            suggestions: Suggestions::default(),
        }
    }
    /// How many log lines [`Self::build`] shows.
    pub fn with_visible_log(mut self, lines: usize) -> Self {
        self.visible_log = lines;
        self
    }
    pub fn print(&mut self, text: impl Into<String>, kind: LogKind) {
        for line in text.into().lines() {
            self.log.push_back((line.to_owned(), kind));
        }
        while self.log.len() > self.max_log {
            self.log.pop_front();
        }
    }
    pub fn log(&self) -> impl Iterator<Item = (&str, LogKind)> {
        self.log.iter().map(|(t, k)| (t.as_str(), *k))
    }
    pub fn history(&self) -> &[String] {
        &self.history
    }
    pub fn suggestions(&self) -> &Suggestions {
        &self.suggestions
    }
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Applies one frame of input: editing, Up/Down through suggestions
    /// (or history when there are none), Tab to complete, Enter to run,
    /// Escape to close. Parse errors are printed to the log.
    pub fn handle<C>(
        &mut self,
        commands: &CommandSet<C>,
        ctx: &C,
        input: &UiInput,
        measure: &dyn TextMeasure,
    ) -> ConsoleOutcome {
        // Up/Down are navigation here, not cursor keys.
        let mut editing = input.clone();
        editing.up = false;
        editing.down = false;
        editing.next = false;
        editing.previous = false;
        editing.activate = false;
        editing.cancel = false;
        let edit = self.line.apply(&editing, measure);
        if edit.changed {
            self.selected = None;
            self.history_index = None;
        }
        self.suggestions = commands.suggest(self.line.text(), ctx);
        let count = self.suggestions.items.len();
        let vertical = i8::from(input.down) - i8::from(input.up);
        if vertical != 0 {
            let browsing = self.history_index.is_some() || self.line.text().trim().is_empty();
            if count > 0 && !browsing {
                self.selected = Some(match (self.selected, vertical > 0) {
                    (None, true) => 0,
                    (None, false) => count - 1,
                    (Some(i), true) => (i + 1) % count,
                    (Some(i), false) => (i + count - 1) % count,
                });
            } else if !self.history.is_empty() {
                let last = self.history.len() - 1;
                let index = match (self.history_index, vertical < 0) {
                    (None, true) => Some(last),
                    (None, false) => None,
                    (Some(i), true) => Some(i.saturating_sub(1)),
                    (Some(i), false) => (i < last).then_some(i + 1),
                };
                self.history_index = index;
                let text = index.map_or(String::new(), |i| self.history[i].clone());
                self.line.set_text(text);
                self.suggestions = commands.suggest(self.line.text(), ctx);
            }
        }
        if (input.next || input.previous) && count > 0 {
            let index = self.selected.unwrap_or(0).min(count - 1);
            if let Some(line) = commands.complete(self.line.text(), ctx, index) {
                self.line.set_text(line);
                self.selected = None;
                self.suggestions = commands.suggest(self.line.text(), ctx);
            }
        }
        if input.activate {
            if let Some(index) = self.selected.take()
                && let Some(line) = commands.complete(self.line.text(), ctx, index)
            {
                self.line.set_text(line);
                self.suggestions = commands.suggest(self.line.text(), ctx);
                return ConsoleOutcome::None;
            }
            let line = self.line.text().trim().to_owned();
            if line.is_empty() {
                return ConsoleOutcome::None;
            }
            self.print(line.clone(), LogKind::Echo);
            if self.history.last() != Some(&line) {
                self.history.push(line.clone());
            }
            self.history_index = None;
            self.line.clear();
            self.suggestions = commands.suggest("", ctx);
            return match commands.parse(&line, ctx) {
                Ok(matched) => ConsoleOutcome::Command(matched),
                Err(message) => {
                    self.print(message, LogKind::Error);
                    ConsoleOutcome::None
                }
            };
        }
        if input.cancel {
            if self.selected.take().is_some() {
                return ConsoleOutcome::None;
            }
            if self.line.text().is_empty() {
                return ConsoleOutcome::Close;
            }
            self.line.clear();
            self.suggestions = commands.suggest("", ctx);
        }
        ConsoleOutcome::None
    }

    /// Builds the overlay (log, suggestions, usage and the edit line with
    /// a measured caret) under `parent`. Returns the overlay panel.
    pub fn build(
        &self,
        ui: &mut Ui,
        parent: NodeId,
        measure: &dyn TextMeasure,
    ) -> UiResult<NodeId> {
        let theme = ui.theme().clone();
        let panel = ui.add_child(
            parent,
            Widget::Panel {
                layout: PanelLayout::Stack {
                    direction: StackDirection::Vertical,
                    gap: 2.0,
                },
                padding: Edges::all(theme.padding),
                clip: true,
                scrollable: false,
            },
            LayoutItem::size(Length::Fill, Length::Auto),
        )?;
        ui.set_style(panel, Some(NodeStyle::filled(theme.overlay)))?;
        let skip = self.log.len().saturating_sub(self.visible_log);
        for (text, kind) in self.log.iter().skip(skip) {
            let label = ui.add_child(
                panel,
                Widget::label(UiText::literal(text.clone()), TextStyle::Small),
                LayoutItem::size(Length::Fill, Length::Auto),
            )?;
            let color = match kind {
                LogKind::Echo => theme.dim_text,
                LogKind::Info => theme.text,
                LogKind::Error => theme.danger,
            };
            ui.set_text_color(label, Some(color))?;
        }
        // Edit line: text plus a measured caret in an absolute panel.
        let font = theme.font_size;
        let row = ui.add_child(
            panel,
            Widget::Panel {
                layout: PanelLayout::Absolute,
                padding: Edges::ZERO,
                clip: true,
                scrollable: false,
            },
            LayoutItem::size(Length::Fill, Length::fixed(theme.row_height)),
        )?;
        ui.set_style(row, Some(NodeStyle::filled(theme.track)))?;
        let pad = theme.padding;
        let mut text_item = LayoutItem::size(Length::Auto, Length::Fill);
        text_item.offset = Vec2::new(pad, 0.0);
        ui.add_child(
            row,
            Widget::label(
                UiText::literal(self.line.text().to_owned()),
                TextStyle::Body,
            ),
            text_item,
        )?;
        let scale = ui.scale().max(f32::EPSILON);
        let caret = self.line.caret_x(measure, TextStyle::Body, font * scale) / scale;
        let mut caret_item = LayoutItem::size(Length::fixed(1.5), Length::fixed(font * 1.2));
        caret_item.offset = Vec2::new(pad + caret, (theme.row_height - font * 1.2) * 0.5);
        let caret_node = ui.add_child(row, Widget::Spacer, caret_item)?;
        ui.set_style(caret_node, Some(NodeStyle::filled(theme.focus)))?;
        // Suggestions with the selection highlighted, then usage.
        for (index, choice) in self.suggestions.items.iter().take(8).enumerate() {
            let text = if choice.help.is_empty() {
                choice.word.clone()
            } else {
                format!("{}  {}", choice.word, choice.help)
            };
            let label = ui.add_child(
                panel,
                Widget::label(UiText::literal(text), TextStyle::Small),
                LayoutItem::size(Length::Fill, Length::Auto),
            )?;
            if Some(index) == self.selected {
                ui.set_style(label, Some(NodeStyle::filled(theme.selection)))?;
                ui.set_colors(label, Some(WidgetColors::text(theme.accent)))?;
            }
        }
        for usage in self.suggestions.usage.iter().take(4) {
            let label = ui.add_child(
                panel,
                Widget::label(UiText::literal(usage.clone()), TextStyle::Small),
                LayoutItem::size(Length::Fill, Length::Auto),
            )?;
            ui.set_text_color(label, Some(Color::lerp(theme.dim_text, theme.accent, 0.35)))?;
        }
        Ok(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Layer, MonospaceMeasure, Rect, Theme};

    struct Game {
        weapons: Vec<&'static str>,
        carried: Vec<&'static str>,
    }

    fn commands() -> CommandSet<Game> {
        let mut set = CommandSet::new();
        set.kind(
            "weapon",
            ArgKind::choices(|g: &Game, _| {
                g.weapons
                    .iter()
                    .map(|w| Choice::new(*w, "weapon"))
                    .collect()
            }),
        )
        .kind(
            "carried",
            ArgKind::choices(|g: &Game, _| g.carried.iter().map(|w| Choice::new(*w, "")).collect()),
        )
        .kind(
            "upgrade",
            // Depends on the weapon chosen before it.
            ArgKind::choices(|_, args| match args.first().map(String::as_str) {
                Some("laser") => vec![Choice::new("focus", ""), Choice::new("split", "")],
                Some(_) => vec![Choice::new("barrel", "")],
                None => Vec::new(),
            }),
        )
        .kind("count", ArgKind::Number { min: 1, max: 20 })
        .kind("rarity", ArgKind::words(&["common", "rare"]))
        .command("give weapon <weapon>", "Add a weapon")
        .command(
            "mod <carried> <upgrade> [rarity] [count]",
            "Upgrade a weapon",
        )
        .command("heal", "Refill health");
        set
    }

    fn game() -> Game {
        Game {
            weapons: vec!["laser", "launcher", "rifle"],
            carried: vec!["laser", "rifle"],
        }
    }

    #[test]
    fn parses_validates_and_reports_the_furthest_error() {
        let (set, g) = (commands(), game());
        let m = set.parse("/give weapon Laser", &g).unwrap();
        assert_eq!((m.command, m.word(0)), (0, Some("laser")));
        let m = set.parse("mod laser split rare 3", &g).unwrap();
        assert_eq!(m.args.len(), 4);
        assert_eq!(m.number(3), Some(3));
        let m = set.parse("mod rifle barrel", &g).unwrap();
        assert_eq!(
            m.args,
            vec![Some("rifle".into()), Some("barrel".into()), None, None]
        );
        assert!(
            set.parse("mod rifle focus", &g)
                .unwrap_err()
                .contains("Unknown upgrade 'focus'")
        );
        assert!(
            set.parse("mod laser focus rare 99", &g)
                .unwrap_err()
                .contains("from 1 to 20")
        );
        assert!(
            set.parse("give weapon", &g)
                .unwrap_err()
                .contains("Missing weapon")
        );
        assert!(set.parse("heal now", &g).unwrap_err().contains("Too many"));
        assert!(
            set.parse("dance", &g)
                .unwrap_err()
                .contains("Unknown command 'dance'")
        );
        assert_eq!(set.help().len(), 3);
    }

    #[test]
    fn suggestions_follow_the_typed_words() {
        let (set, g) = (commands(), game());
        let first = set.suggest("/", &g);
        let words: Vec<&str> = first.items.iter().map(|c| c.word.as_str()).collect();
        assert_eq!(words, ["give", "mod", "heal"]);
        let s = set.suggest("/give weapon la", &g);
        assert_eq!(s.from, "/give weapon ".len());
        let words: Vec<&str> = s.items.iter().map(|c| c.word.as_str()).collect();
        assert_eq!(words, ["laser", "launcher"]);
        assert_eq!(s.usage, ["/give weapon <weapon>"]);
        let s = set.suggest("mod laser ", &g);
        let words: Vec<&str> = s.items.iter().map(|c| c.word.as_str()).collect();
        assert_eq!(words, ["focus", "split"]);
        // A number slot lists no words but keeps the usage visible.
        let s = set.suggest("mod laser focus rare ", &g);
        assert!(s.items.is_empty());
        assert_eq!(s.usage.len(), 1);
        assert_eq!(
            set.complete("/give weapon la", &g, 1).unwrap(),
            "/give weapon launcher "
        );
    }

    #[test]
    fn the_console_completes_submits_and_recalls_history() {
        let (set, g) = (commands(), game());
        let m = &MonospaceMeasure;
        let mut console = Console::new();
        let type_text = |c: &mut Console, text: &str| {
            c.handle(
                &set,
                &g,
                &UiInput {
                    text: text.into(),
                    ..UiInput::default()
                },
                m,
            )
        };
        type_text(&mut console, "/give weapon ri");
        assert_eq!(console.suggestions().items[0].word, "rifle");
        console.handle(
            &set,
            &g,
            &UiInput {
                next: true,
                ..UiInput::default()
            },
            m,
        );
        assert_eq!(console.line.text(), "/give weapon rifle ");
        let out = console.handle(
            &set,
            &g,
            &UiInput {
                activate: true,
                ..UiInput::default()
            },
            m,
        );
        let ConsoleOutcome::Command(matched) = out else {
            panic!("{out:?}")
        };
        assert_eq!(matched.word(0), Some("rifle"));
        assert_eq!(console.line.text(), "");
        // Errors are logged, not returned.
        type_text(&mut console, "heal twice");
        let out = console.handle(
            &set,
            &g,
            &UiInput {
                activate: true,
                ..UiInput::default()
            },
            m,
        );
        assert_eq!(out, ConsoleOutcome::None);
        assert_eq!(console.log().last().unwrap().1, LogKind::Error);
        // Up with an empty line walks history (no suggestion is selected
        // because history navigation started).
        console.handle(
            &set,
            &g,
            &UiInput {
                up: true,
                ..UiInput::default()
            },
            m,
        );
        assert_eq!(console.line.text(), "heal twice");
        console.handle(
            &set,
            &g,
            &UiInput {
                cancel: true,
                ..UiInput::default()
            },
            m,
        );
        assert_eq!(console.line.text(), "");
        let out = console.handle(
            &set,
            &g,
            &UiInput {
                cancel: true,
                ..UiInput::default()
            },
            m,
        );
        assert_eq!(out, ConsoleOutcome::Close);
        // Down selects a suggestion; Enter accepts it instead of running.
        type_text(&mut console, "/he");
        console.handle(
            &set,
            &g,
            &UiInput {
                down: true,
                ..UiInput::default()
            },
            m,
        );
        assert_eq!(console.selected(), Some(0));
        console.handle(
            &set,
            &g,
            &UiInput {
                activate: true,
                ..UiInput::default()
            },
            m,
        );
        assert_eq!(console.line.text(), "/heal ");

        let mut ui = Ui::new(Theme::modern());
        let root = ui.add_root(
            Widget::Panel {
                layout: PanelLayout::Stack {
                    direction: StackDirection::Vertical,
                    gap: 0.0,
                },
                padding: Edges::ZERO,
                clip: false,
                scrollable: false,
            },
            LayoutItem::fill(),
            Layer::Overlay,
            false,
        );
        let panel = console.build(&mut ui, root, m).unwrap();
        ui.layout(Rect::new(0.0, 0.0, 640.0, 360.0), 1.0, Edges::ZERO, m);
        assert!(ui.node(panel).unwrap().children.len() >= 4);
    }
}
