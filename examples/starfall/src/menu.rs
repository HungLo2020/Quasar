//! Modern 3D menu stack for Starfall.
use crate::world::GameError;
use quasar_ui::{
    ActionId, Align, Edges, Layer, LayoutItem, Length, PanelLayout, Rect, StackDirection,
    TextMeasure, Theme, Ui, UiEvent, UiInput, UiText, Widget,
};

impl Default for Options {
    fn default() -> Self {
        Self {
            fullscreen: false,
            volume: 0.6,
            sensitivity: 0.0026,
            player_name: "Pilot".into(),
        }
    }
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
            "# Starfall UI preferences (game-owned).\nversion = 1\n\n[ui]\nfullscreen = {}\nvolume = {:.3}\nsensitivity = {:.6}\nplayer_name = \"{}\"\n",
            self.fullscreen, self.volume, self.sensitivity, player_name
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Screen {
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
        };
        menu.rebuild();
        menu
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
            if matches!(event, UiEvent::Cancelled) {
                action = match self.screen {
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
    }

    fn rebuild(&mut self) {
        self.fullscreen = None;
        self.volume = None;
        self.sensitivity = None;
        self.player_name = None;
        let Some(screen) = self.screen else {
            self.ui = Ui::new(Theme::modern());
            return;
        };
        let mut ui = Ui::new(Theme::modern());
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
            Screen::Options => 620.0,
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
            Screen::ConfirmQuitApp => "QUIT STARFALL?",
            Screen::ConfirmQuitToMenu => "RETURN TO MAIN MENU?",
            Screen::ConfirmRestart => "RESTART RUN?",
        };
        add_label(&mut ui, panel, title, quasar_ui::TextStyle::Heading);
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
                add_button(&mut ui, panel, "QUIT", 3);
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
                            label: UiText::literal("FULLSCREEN"),
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
                            label: UiText::literal("MASTER VOLUME"),
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
                            label: UiText::literal("MOUSE SENSITIVITY"),
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
                            placeholder: UiText::literal("CALLSIGN"),
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
            }
            Screen::ConfirmQuitApp | Screen::ConfirmQuitToMenu | Screen::ConfirmRestart => {
                add_label(
                    &mut ui,
                    panel,
                    "CURRENT RUN PROGRESS WILL BE LOST.",
                    quasar_ui::TextStyle::Body,
                );
                add_button(&mut ui, panel, "YES", 8);
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

fn add_label(ui: &mut Ui, parent: quasar_ui::NodeId, text: &str, style: quasar_ui::TextStyle) {
    ui.add_child(
        parent,
        Widget::Label {
            text: UiText::literal(text),
            style,
        },
        LayoutItem::size(Length::Fill, Length::Auto),
    )
    .unwrap();
}

fn add_button(ui: &mut Ui, parent: quasar_ui::NodeId, text: &str, action: u64) {
    ui.add_child(
        parent,
        Widget::Button {
            text: UiText::literal(text),
            action: Some(ActionId::new(action)),
        },
        LayoutItem::size(Length::Fill, Length::fixed(36.0)),
    )
    .unwrap();
}
