//! Modern 3D menu stack for Starfall.
use crate::world::GameError;
use quasar_input::{Bindings, Button, DeviceFamily, GamepadAxis, InputDevice};
use quasar_ui::{
    ActionId, Align, Color, ControlEntry, ControlsOutcome, ControlsScreen, Edges, LabelWrap, Layer,
    LayoutItem, Length, Localization, PanelLayout, Rect, RichSpan, StackDirection, StringTable,
    TextMeasure, Theme, Ui, UiEvent, UiInput, UiText, Widget, WidgetColors,
};

/// Menu strings. Keys come from the English text (`key_of`); English is
/// also the fallback, so a missing translation shows English.
const ENGLISH: &str = include_str!("../lang/en.lang");
const SPANISH: &str = include_str!("../lang/es.lang");

/// Languages offered in the options screen: (tag, native name).
pub const LANGUAGES: [(&str, &str); 2] = [("en", "English"), ("es", "Español")];

fn string_tables() -> Vec<StringTable> {
    vec![
        StringTable::parse("en", ENGLISH).expect("built-in English strings"),
        StringTable::parse("es", SPANISH).expect("built-in Spanish strings"),
    ]
}

/// Localisation chain for a language tag, falling back to English.
pub fn localization(language: &str) -> Localization {
    Localization::negotiate(language, &string_tables(), "en")
}

/// String-table key of a menu text: `menu.` + lower-case words joined by `_`.
pub fn key_of(text: &str) -> String {
    let words: String = text
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "menu.{}",
        words
            .split('_')
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join("_")
    )
}

fn keyed(text: &str) -> UiText {
    UiText::keyed(key_of(text), text)
}

/// Game actions listed on the controls screen.
pub fn control_entries() -> Vec<ControlEntry> {
    [
        ("forward", "THRUST"),
        ("strafe", "STRAFE"),
        ("turn", "TURN"),
        ("shoot", "FIRE"),
        ("restart", "RESTART"),
    ]
    .into_iter()
    .map(|(action, label)| ControlEntry::new(action, keyed(label)))
    .collect()
}

impl Default for Options {
    fn default() -> Self {
        Self {
            fullscreen: false,
            volume: 0.6,
            sensitivity: 0.0026,
            player_name: "Pilot".into(),
            language: "en".into(),
        }
    }
}

/// Saved controls, next to the UI preferences.
pub fn controls_path() -> std::path::PathBuf {
    Options::path().with_file_name("starfall-controls.toml")
}

impl Options {
    /// Game-owned preferences path, separate from the engine and config schema.
    pub fn path() -> std::path::PathBuf {
        if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
            return std::path::PathBuf::from(path)
                .join("quasar")
                .join("starfall-ui.toml");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home)
                .join(".config")
                .join("quasar")
                .join("starfall-ui.toml");
        }
        std::path::PathBuf::from("starfall-ui.toml")
    }

    pub fn load_default() -> Option<Self> {
        Self::load(&Self::path()).ok()
    }

    pub fn load(path: &std::path::Path) -> Result<Self, GameError> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| GameError::invalid(format!("read UI settings: {error}")))?;
        let mut fullscreen = None;
        let mut volume = None;
        let mut sensitivity = None;
        let mut player_name = None;
        let mut language = None;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "fullscreen" => fullscreen = Some(value == "true"),
                "volume" => volume = value.parse::<f32>().ok(),
                "sensitivity" => sensitivity = value.parse::<f32>().ok(),
                "language" => {
                    let tag = value.trim_matches('"');
                    if LANGUAGES.iter().any(|(t, _)| *t == tag) {
                        language = Some(tag.to_owned());
                    }
                }
                "player_name" => {
                    player_name = Some(
                        value
                            .trim_matches('"')
                            .replace("\\\"", "\"")
                            .replace("\\n", "\n"),
                    );
                }
                _ => {}
            }
        }
        let defaults = Self::default();
        Ok(Self {
            fullscreen: fullscreen.unwrap_or(defaults.fullscreen),
            volume: volume.unwrap_or(defaults.volume).clamp(0.0, 1.0),
            sensitivity: sensitivity
                .unwrap_or(defaults.sensitivity)
                .clamp(0.0001, 0.05),
            player_name: player_name.unwrap_or(defaults.player_name),
            language: language.unwrap_or(defaults.language),
        })
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), GameError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| GameError::invalid(format!("create UI settings dir: {error}")))?;
        }
        let player_name = self
            .player_name
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "");
        let text = format!(
            "# Starfall UI preferences (game-owned).\nversion = 1\n\n[ui]\nfullscreen = {}\nvolume = {:.3}\nsensitivity = {:.6}\nplayer_name = \"{}\"\nlanguage = \"{}\"\n",
            self.fullscreen, self.volume, self.sensitivity, player_name, self.language
        );
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, text)
            .map_err(|error| GameError::invalid(format!("write UI settings: {error}")))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| GameError::invalid(format!("replace UI settings: {error}")))?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub fullscreen: bool,
    pub volume: f32,
    pub sensitivity: f32,
    pub player_name: String,
    /// Menu language tag (see [`LANGUAGES`]).
    pub language: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Screen {
    Controls,
    Main,
    Pause,
    Options,
    ConfirmQuitApp,
    ConfirmQuitToMenu,
    ConfirmRestart,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuAction {
    None,
    Start,
    Resume,
    Restart,
    QuitToMenu,
    Quit,
    Apply(Options),
    /// The controls screen changed the game bindings.
    ApplyBindings(Bindings),
}

pub struct Menu {
    ui: Ui,
    screen: Option<Screen>,
    return_screen: Screen,
    options: Options,
    applied: Options,
    fullscreen: Option<quasar_ui::NodeId>,
    volume: Option<quasar_ui::NodeId>,
    sensitivity: Option<quasar_ui::NodeId>,
    player_name: Option<quasar_ui::NodeId>,
    language: Option<quasar_ui::NodeId>,
    controls: Option<ControlsScreen>,
    /// Which column the controls screen edits.
    controls_device: InputDevice,
    bindings: Bindings,
    default_bindings: Bindings,
}

impl Menu {
    pub fn new(options: Options) -> Self {
        let mut menu = Self {
            ui: Ui::new(Theme::modern()),
            screen: None,
            return_screen: Screen::Main,
            options: options.clone(),
            applied: options,
            fullscreen: None,
            volume: None,
            sensitivity: None,
            player_name: None,
            language: None,
            controls: None,
            controls_device: InputDevice::KeyboardMouse,
            bindings: Bindings::default(),
            default_bindings: Bindings::default(),
        };
        menu.rebuild();
        menu
    }

    /// Game bindings shown on the controls screen and their defaults.
    pub fn set_bindings(&mut self, bindings: Bindings, defaults: Bindings) {
        self.bindings = bindings.merged_with_defaults(&defaults);
        self.default_bindings = defaults;
    }

    pub fn bindings(&self) -> &Bindings {
        &self.bindings
    }

    /// Whether the controls screen waits for a button press.
    pub fn is_capturing(&self) -> bool {
        self.controls
            .as_ref()
            .is_some_and(ControlsScreen::is_capturing)
    }

    /// Offers this frame's pressed buttons to a running rebind.
    pub fn capture(&mut self, pressed: &[Button]) -> Result<MenuAction, GameError> {
        let Some(controls) = &mut self.controls else {
            return Ok(MenuAction::None);
        };
        match controls
            .capture(&mut self.ui, pressed)
            .map_err(|e| GameError::invalid(format!("controls: {e:?}")))?
        {
            ControlsOutcome::Rebound { .. } => {
                self.bindings = controls.bindings().clone();
                Ok(MenuAction::ApplyBindings(self.bindings.clone()))
            }
            _ => Ok(MenuAction::None),
        }
    }

    /// Offers the gamepad's processed axes to a running rebind of an
    /// analogue row (stick or trigger).
    pub fn capture_axes(&mut self, axes: &[(GamepadAxis, f32)]) -> Result<MenuAction, GameError> {
        let Some(controls) = &mut self.controls else {
            return Ok(MenuAction::None);
        };
        match controls
            .capture_axes(&mut self.ui, axes)
            .map_err(|e| GameError::invalid(format!("controls: {e:?}")))?
        {
            ControlsOutcome::Rebound { .. } => {
                self.bindings = controls.bindings().clone();
                Ok(MenuAction::ApplyBindings(self.bindings.clone()))
            }
            _ => Ok(MenuAction::None),
        }
    }

    pub fn open_main(&mut self) {
        self.screen = Some(Screen::Main);
        self.rebuild();
    }

    pub fn open_pause(&mut self) {
        self.screen = Some(Screen::Pause);
        self.rebuild();
    }

    pub fn close(&mut self) {
        self.screen = None;
        self.ui = Ui::new(Theme::modern());
    }

    pub fn is_open(&self) -> bool {
        self.screen.is_some()
    }

    pub fn ui(&self) -> Option<&Ui> {
        self.is_open().then_some(&self.ui)
    }

    pub fn options(&self) -> Options {
        self.options.clone()
    }

    pub fn update(
        &mut self,
        input: &UiInput,
        viewport: Rect,
        scale: f32,
        measure: &dyn TextMeasure,
        dt: f32,
    ) -> Result<MenuAction, GameError> {
        if !self.is_open() {
            return Ok(MenuAction::None);
        }
        let frame = self
            .ui
            .update(input, viewport, scale, Edges::ZERO, measure, dt);
        self.sync_options();
        let mut action = MenuAction::None;
        for event in &frame.events {
            if let Some(controls) = &mut self.controls {
                match controls
                    .handle_event(&mut self.ui, event)
                    .map_err(|e| GameError::invalid(format!("controls: {e:?}")))?
                {
                    ControlsOutcome::Reset => {
                        self.bindings = controls.bindings().clone();
                        action = MenuAction::ApplyBindings(self.bindings.clone());
                        break;
                    }
                    ControlsOutcome::None => {}
                    _ => continue,
                }
            }
            if matches!(event, UiEvent::Cancelled) {
                action = match self.screen {
                    Some(Screen::Controls) => {
                        if self.is_capturing() {
                            continue;
                        }
                        self.screen = Some(Screen::Options);
                        self.rebuild();
                        MenuAction::None
                    }
                    Some(Screen::Pause) => {
                        self.close();
                        MenuAction::Resume
                    }
                    Some(Screen::Options) => {
                        self.options = self.applied.clone();
                        self.screen = Some(self.return_screen);
                        self.rebuild();
                        MenuAction::None
                    }
                    Some(
                        Screen::ConfirmQuitApp | Screen::ConfirmQuitToMenu | Screen::ConfirmRestart,
                    ) => {
                        self.screen = Some(self.return_screen);
                        self.rebuild();
                        MenuAction::None
                    }
                    _ => MenuAction::None,
                };
                if action != MenuAction::None {
                    break;
                }
                continue;
            }
            if let UiEvent::Pressed(id) = event {
                action = match (self.screen, id.0) {
                    (Some(Screen::Main), 1) => {
                        self.close();
                        MenuAction::Start
                    }
                    (Some(Screen::Pause), 1) => {
                        self.close();
                        MenuAction::Resume
                    }
                    (Some(Screen::Main), 2) | (Some(Screen::Pause), 2) => {
                        self.return_screen = self.screen.unwrap_or(Screen::Main);
                        self.screen = Some(Screen::Options);
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::Main), 3) => {
                        self.return_screen = Screen::Main;
                        self.screen = Some(Screen::ConfirmQuitApp);
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::Pause), 3) => {
                        self.return_screen = Screen::Pause;
                        self.screen = Some(Screen::ConfirmQuitToMenu);
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::Pause), 4) => {
                        self.return_screen = Screen::Pause;
                        self.screen = Some(Screen::ConfirmRestart);
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::Options), 5) => {
                        let options = self.options.clone();
                        self.applied = options.clone();
                        self.screen = Some(self.return_screen);
                        self.rebuild();
                        MenuAction::Apply(options)
                    }
                    (Some(Screen::Options), 6) => {
                        self.options = self.applied.clone();
                        self.screen = Some(self.return_screen);
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::Options), 7) => {
                        self.options = Options::default();
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::Options), 10) => {
                        self.screen = Some(Screen::Controls);
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::Controls), 11) => {
                        self.screen = Some(Screen::Options);
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::Controls), 13) => {
                        self.controls_device = match self.controls_device {
                            InputDevice::KeyboardMouse => InputDevice::Gamepad,
                            InputDevice::Gamepad => InputDevice::KeyboardMouse,
                        };
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::ConfirmQuitApp), 8) => MenuAction::Quit,
                    (Some(Screen::ConfirmQuitToMenu), 8) => {
                        self.open_main();
                        MenuAction::QuitToMenu
                    }
                    (Some(Screen::ConfirmRestart), 8) => {
                        self.close();
                        MenuAction::Restart
                    }
                    (
                        Some(
                            Screen::ConfirmQuitApp
                            | Screen::ConfirmQuitToMenu
                            | Screen::ConfirmRestart,
                        ),
                        9,
                    ) => {
                        self.screen = Some(self.return_screen);
                        self.rebuild();
                        MenuAction::None
                    }
                    _ => MenuAction::None,
                };
                if action != MenuAction::None {
                    break;
                }
            }
        }
        Ok(action)
    }

    fn sync_options(&mut self) {
        if let Some(id) = self.fullscreen
            && let Some(node) = self.ui.node(id)
            && let Widget::Checkbox { checked, .. } = &node.widget
        {
            self.options.fullscreen = *checked;
        }
        if let Some(id) = self.volume
            && let Some(node) = self.ui.node(id)
            && let Widget::Slider { value, .. } = &node.widget
        {
            self.options.volume = *value;
        }
        if let Some(id) = self.sensitivity
            && let Some(node) = self.ui.node(id)
            && let Widget::Slider { value, .. } = &node.widget
        {
            self.options.sensitivity = *value;
        }
        if let Some(id) = self.player_name
            && let Some(node) = self.ui.node(id)
            && let Widget::TextInput { text, .. } = &node.widget
        {
            self.options.player_name.clone_from(text);
        }
        if let Some(id) = self.language
            && let Some(node) = self.ui.node(id)
            && let Widget::Select { selected, .. } = &node.widget
            && let Some((tag, _)) = LANGUAGES.get(*selected)
            && self.options.language != *tag
        {
            // Preview the language immediately; Cancel restores it.
            self.options.language = (*tag).to_owned();
            self.ui.set_localization(&localization(tag));
        }
    }

    fn rebuild(&mut self) {
        self.fullscreen = None;
        self.volume = None;
        self.sensitivity = None;
        self.player_name = None;
        self.language = None;
        self.controls = None;
        let Some(screen) = self.screen else {
            self.ui = Ui::new(Theme::modern());
            return;
        };
        let mut ui = Ui::new(Theme::modern());
        ui.set_localization(&localization(&self.options.language));
        let root = ui.add_root(
            Widget::Panel {
                layout: PanelLayout::Absolute,
                padding: Edges::ZERO,
                clip: true,
                scrollable: false,
            },
            LayoutItem::fill(),
            Layer::Normal,
            matches!(
                screen,
                Screen::ConfirmQuitApp | Screen::ConfirmQuitToMenu | Screen::ConfirmRestart
            ),
        );
        let width = match screen {
            Screen::Options | Screen::Controls => 620.0,
            Screen::ConfirmQuitApp | Screen::ConfirmQuitToMenu | Screen::ConfirmRestart => 560.0,
            _ => 460.0,
        };
        let panel = ui
            .add_child(
                root,
                Widget::Panel {
                    layout: PanelLayout::Stack {
                        direction: StackDirection::Vertical,
                        gap: 8.0,
                    },
                    padding: Edges::all(22.0),
                    clip: true,
                    scrollable: false,
                },
                LayoutItem::size(Length::fixed(width), Length::Auto).with_align(Align::Center),
            )
            .unwrap();
        let title = match screen {
            Screen::Main => "STARFALL",
            Screen::Pause => "PAUSED",
            Screen::Options => "OPTIONS",
            Screen::Controls => "CONTROLS",
            Screen::ConfirmQuitApp => "QUIT STARFALL?",
            Screen::ConfirmQuitToMenu => "RETURN TO MAIN MENU?",
            Screen::ConfirmRestart => "RESTART RUN?",
        };
        let heading = add_label(&mut ui, panel, title, quasar_ui::TextStyle::Heading);
        let accent = ui.theme().accent;
        ui.set_text_color(heading, Some(accent)).unwrap();
        match screen {
            Screen::Main => {
                add_label(
                    &mut ui,
                    panel,
                    "QUASAR · Ελληνικά · Русский · नमस्ते · مرحبا",
                    quasar_ui::TextStyle::Small,
                );
                add_button(&mut ui, panel, "START", 1);
                add_button(&mut ui, panel, "OPTIONS", 2);
                let quit = add_button(&mut ui, panel, "QUIT", 3);
                danger(&mut ui, quit);
            }
            Screen::Pause => {
                add_button(&mut ui, panel, "RESUME", 1);
                add_button(&mut ui, panel, "OPTIONS", 2);
                add_button(&mut ui, panel, "QUIT TO MENU", 3);
                add_button(&mut ui, panel, "RESTART", 4);
            }
            Screen::Options => {
                let fullscreen = ui
                    .add_child(
                        panel,
                        Widget::Checkbox {
                            label: keyed("FULLSCREEN"),
                            checked: self.options.fullscreen,
                            action: None,
                        },
                        LayoutItem::size(Length::Fill, Length::fixed(32.0)),
                    )
                    .unwrap();
                self.fullscreen = Some(fullscreen);
                let volume = ui
                    .add_child(
                        panel,
                        Widget::Slider {
                            label: keyed("MASTER VOLUME"),
                            value: self.options.volume,
                            min: 0.0,
                            max: 1.0,
                            step: 0.05,
                            action: None,
                        },
                        LayoutItem::size(Length::Fill, Length::fixed(30.0)),
                    )
                    .unwrap();
                self.volume = Some(volume);
                let sensitivity = ui
                    .add_child(
                        panel,
                        Widget::Slider {
                            label: keyed("MOUSE SENSITIVITY"),
                            value: self.options.sensitivity,
                            min: 0.0005,
                            max: 0.01,
                            step: 0.0001,
                            action: None,
                        },
                        LayoutItem::size(Length::Fill, Length::fixed(30.0)),
                    )
                    .unwrap();
                self.sensitivity = Some(sensitivity);
                let player_name = ui
                    .add_child(
                        panel,
                        Widget::TextInput {
                            text: self.options.player_name.clone(),
                            placeholder: keyed("CALLSIGN"),
                            cursor: self.options.player_name.chars().count(),
                            anchor: self.options.player_name.chars().count(),
                            max_len: 32,
                            password: false,
                            action: None,
                        },
                        LayoutItem::size(Length::Fill, Length::fixed(30.0)),
                    )
                    .unwrap();
                self.player_name = Some(player_name);
                add_label(
                    &mut ui,
                    panel,
                    "PILOT CALLSIGN · UNICODE / IME READY",
                    quasar_ui::TextStyle::Small,
                );
                add_label(
                    &mut ui,
                    panel,
                    "APPLY CHANGES TO THE RUNNING GAME.",
                    quasar_ui::TextStyle::Small,
                );
                add_button(&mut ui, panel, "APPLY", 5);
                add_button(&mut ui, panel, "CANCEL", 6);
                add_button(&mut ui, panel, "RESET DEFAULTS", 7);
                add_button(&mut ui, panel, "CONTROLS", 10);
                let selected = LANGUAGES
                    .iter()
                    .position(|(tag, _)| *tag == self.options.language)
                    .unwrap_or(0);
                let language = ui
                    .add_child(
                        panel,
                        Widget::Select {
                            label: keyed("LANGUAGE"),
                            options: LANGUAGES
                                .iter()
                                .map(|(_, name)| UiText::literal(*name))
                                .collect(),
                            selected,
                            action: None,
                        },
                        LayoutItem::size(Length::Fill, Length::fixed(30.0)),
                    )
                    .unwrap();
                self.language = Some(language);
            }
            Screen::Controls => {
                // The reset button matches the menu's full-width buttons.
                let mut controls = ControlsScreen::new(
                    control_entries(),
                    &self.bindings,
                    self.default_bindings.clone(),
                )
                .with_reset_item(LayoutItem::size(Length::Fill, Length::fixed(36.0)));
                controls
                    .build(&mut ui, panel, ActionId::new(12))
                    .expect("controls screen");
                if self.controls_device == InputDevice::Gamepad {
                    controls
                        .set_device(&mut ui, InputDevice::Gamepad, DeviceFamily::Xbox)
                        .expect("gamepad column");
                }
                // A prompt with an inline key cap.
                ui.add_child(
                    panel,
                    Widget::RichText {
                        spans: vec![
                            RichSpan::Text(keyed("SELECT A ROW, THEN PRESS A KEY.")),
                            RichSpan::Key(UiText::literal("ESC")),
                            RichSpan::Text(keyed("CANCELS.")),
                        ],
                        style: quasar_ui::TextStyle::Small,
                        wrap: LabelWrap::Word,
                        max_width: None,
                    },
                    LayoutItem::size(Length::Fill, Length::Auto),
                )
                .unwrap();
                add_button(
                    &mut ui,
                    panel,
                    match self.controls_device {
                        InputDevice::KeyboardMouse => "SHOW GAMEPAD",
                        InputDevice::Gamepad => "SHOW KEYBOARD",
                    },
                    13,
                );
                add_button(&mut ui, panel, "BACK", 11);
                self.controls = Some(controls);
            }
            Screen::ConfirmQuitApp | Screen::ConfirmQuitToMenu | Screen::ConfirmRestart => {
                add_label(
                    &mut ui,
                    panel,
                    "CURRENT RUN PROGRESS WILL BE LOST.",
                    quasar_ui::TextStyle::Body,
                );
                let yes = add_button(&mut ui, panel, "YES", 8);
                danger(&mut ui, yes);
                add_button(&mut ui, panel, "NO", 9);
            }
        }
        if matches!(
            screen,
            Screen::ConfirmQuitApp | Screen::ConfirmQuitToMenu | Screen::ConfirmRestart
        ) {
            ui.set_modal(Some(root));
        }
        self.ui = ui;
    }
}

fn add_label(
    ui: &mut Ui,
    parent: quasar_ui::NodeId,
    text: &str,
    style: quasar_ui::TextStyle,
) -> quasar_ui::NodeId {
    ui.add_child(
        parent,
        Widget::label(keyed(text), style),
        LayoutItem::size(Length::Fill, Length::Auto),
    )
    .unwrap()
}

fn add_button(
    ui: &mut Ui,
    parent: quasar_ui::NodeId,
    text: &str,
    action: u64,
) -> quasar_ui::NodeId {
    ui.add_child(
        parent,
        Widget::Button {
            text: keyed(text),
            action: Some(ActionId::new(action)),
        },
        LayoutItem::size(Length::Fill, Length::fixed(36.0)),
    )
    .unwrap()
}

/// Red tones for destructive choices (quit, confirm loss of progress).
fn danger(ui: &mut Ui, button: quasar_ui::NodeId) {
    let base = Color::rgb(92, 28, 34);
    let colors = WidgetColors::button(base, Color::rgb(140, 40, 48), Color::rgb(176, 52, 60))
        .with_text(Color::rgb(255, 214, 214));
    ui.set_colors(button, Some(colors)).unwrap();
}
