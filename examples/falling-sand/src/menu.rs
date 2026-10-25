//! Retro pixel-art menu stack for Falling Sand.
use crate::SandError;
use quasar_ui::{
    ActionId, Align, BitmapTextMeasure, Edges, Layer, LayoutItem, Length, PanelLayout, Rect,
    StackDirection, Theme, Ui, UiEvent, UiInput, UiText, Widget,
};
use settings::UiOptions;

use crate::settings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Screen {
    Main,
    Pause,
    Options,
    ConfirmQuitApp,
    ConfirmQuitToMenu,
    ConfirmRestart,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAction {
    None,
    Start,
    Resume,
    Restart,
    QuitToMenu,
    Quit,
    Apply(UiOptions),
}

pub struct Menu {
    ui: Ui,
    screen: Option<Screen>,
    return_screen: Screen,
    options: UiOptions,
    applied: UiOptions,
    fullscreen: Option<quasar_ui::NodeId>,
    brush: Option<quasar_ui::NodeId>,
    scale: Option<quasar_ui::NodeId>,
}

impl Menu {
    pub fn new(options: UiOptions) -> Self {
        let mut menu = Self {
            ui: Ui::new(Theme::retro()),
            screen: None,
            return_screen: Screen::Main,
            options,
            applied: options,
            fullscreen: None,
            brush: None,
            scale: None,
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
        self.ui = Ui::new(Theme::retro());
    }

    pub fn is_open(&self) -> bool {
        self.screen.is_some()
    }

    pub fn ui(&self) -> Option<&Ui> {
        self.is_open().then_some(&self.ui)
    }

    pub fn options(&self) -> UiOptions {
        self.options
    }

    pub fn set_options(&mut self, options: UiOptions) {
        self.options = options;
        self.applied = options;
    }

    pub fn update(
        &mut self,
        input: &UiInput,
        viewport: Rect,
        scale: f32,
        dt: f32,
    ) -> Result<MenuAction, SandError> {
        if !self.is_open() {
            return Ok(MenuAction::None);
        }
        let frame = self
            .ui
            .update(input, viewport, scale, Edges::ZERO, &BitmapTextMeasure, dt);
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
                        self.options = self.applied;
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
                let id = id.0;
                action = match (self.screen, id) {
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
                        let options = self.options;
                        self.applied = options;
                        self.screen = Some(self.return_screen);
                        self.rebuild();
                        MenuAction::Apply(options)
                    }
                    (Some(Screen::Options), 6) => {
                        self.options = self.applied;
                        self.screen = Some(self.return_screen);
                        self.rebuild();
                        MenuAction::None
                    }
                    (Some(Screen::Options), 7) => {
                        self.options = UiOptions::default();
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
        if let Some(id) = self.brush
            && let Some(node) = self.ui.node(id)
            && let Widget::Slider { value, .. } = &node.widget
        {
            self.options.brush_radius = *value;
        }
        if let Some(id) = self.scale
            && let Some(node) = self.ui.node(id)
            && let Widget::Slider { value, .. } = &node.widget
        {
            self.options.ui_scale = *value;
        }
    }

    fn rebuild(&mut self) {
        self.fullscreen = None;
        self.brush = None;
        self.scale = None;
        let Some(screen) = self.screen else {
            self.ui = Ui::new(Theme::retro());
            return;
        };
        let mut ui = Ui::new(Theme::retro());
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
            Screen::Options => 520.0,
            Screen::ConfirmQuitApp | Screen::ConfirmQuitToMenu | Screen::ConfirmRestart => 500.0,
            _ => 400.0,
        };
        let panel = ui
            .add_child(
                root,
                Widget::Panel {
                    layout: PanelLayout::Stack {
                        direction: StackDirection::Vertical,
                        gap: 6.0,
                    },
                    padding: Edges::all(18.0),
                    clip: true,
                    scrollable: false,
                },
                LayoutItem::size(Length::fixed(width), Length::Auto).with_align(Align::Center),
            )
            .unwrap();
        let title = match screen {
            Screen::Main => "FALLING SAND",
            Screen::Pause => "PAUSED",
            Screen::Options => "OPTIONS",
            Screen::ConfirmQuitApp => "QUIT FALLING SAND?",
            Screen::ConfirmQuitToMenu => "RETURN TO MAIN MENU?",
            Screen::ConfirmRestart => "RESTART SIMULATION?",
        };
        add_label(&mut ui, panel, title, quasar_ui::TextStyle::Heading);
        match screen {
            Screen::Main => {
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
                        LayoutItem::size(Length::Fill, Length::fixed(28.0)),
                    )
                    .unwrap();
                self.fullscreen = Some(fullscreen);
                let brush = ui
                    .add_child(
                        panel,
                        Widget::Slider {
                            label: UiText::literal("BRUSH RADIUS"),
                            value: self.options.brush_radius,
                            min: 1.0,
                            max: 16.0,
                            step: 1.0,
                            action: None,
                        },
                        LayoutItem::size(Length::Fill, Length::fixed(28.0)),
                    )
                    .unwrap();
                self.brush = Some(brush);
                let scale = ui
                    .add_child(
                        panel,
                        Widget::Slider {
                            label: UiText::literal("UI SCALE"),
                            value: self.options.ui_scale,
                            min: 0.75,
                            max: 2.0,
                            step: 0.25,
                            action: None,
                        },
                        LayoutItem::size(Length::Fill, Length::fixed(28.0)),
                    )
                    .unwrap();
                self.scale = Some(scale);
                add_label(
                    &mut ui,
                    panel,
                    "AUDIO: DEMO HAS NO AUDIO OUTPUT",
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
                    "THIS DISCARDS THE CURRENT SAND STATE.",
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
        LayoutItem::size(Length::Fill, Length::fixed(32.0)),
    )
    .unwrap();
}
