//! String tables, fallback chains, formatting and localisation QA hooks.
use quasar_ui::{
    Edges, Layer, LayoutItem, Localization, MonospaceMeasure, Rect, StringTable, TextStyle, Theme,
    Ui, UiText, Widget,
};

const EN: &str = "
# English
menu.start = Start game
hud.ammo = Ammo: {count}
hud.enemies.one = {count} enemy left
hud.enemies.other = {count} enemies left
hud.literal = Use \\{braces\\} \\n here
";
const FR: &str = "
menu.start = Commencer la partie
hud.enemies.one = {count} ennemi restant
hud.enemies.other = {count} ennemis restants
";

#[test]
fn tables_parse_and_reject_malformed_lines() {
    let en = StringTable::parse("en", EN).unwrap();
    assert_eq!(en.len(), 5);
    assert_eq!(en.get("menu.start"), Some("Start game"));
    let duplicate = StringTable::parse("en", "a = 1\na = 2").unwrap_err();
    assert_eq!(duplicate.line, 2);
    assert!(StringTable::parse("en", "no equals sign").is_err());
    assert!(StringTable::parse("en", "bad key! = x").is_err());
    assert!(StringTable::parse("en", "k = bad \\q escape").is_err());
    let fr = StringTable::parse("fr", FR).unwrap();
    assert_eq!(fr.missing_from(&en), vec!["hud.ammo", "hud.literal"]);
}

#[test]
fn negotiation_falls_back_by_region_language_and_default() {
    let en = StringTable::parse("en", EN).unwrap();
    let fr = StringTable::parse("fr", FR).unwrap();
    let mut fr_ca = StringTable::new("fr-CA");
    fr_ca.insert("menu.start", "Démarrer");
    let available = [en, fr, fr_ca];
    let chain = Localization::negotiate("fr-CA", &available, "en");
    assert_eq!(chain.language(), Some("fr-CA"));
    assert_eq!(chain.tables().len(), 3);
    assert_eq!(chain.get("menu.start"), Some("Démarrer"));
    assert_eq!(
        chain.format_count("hud.enemies", "", 3, &[]),
        "3 ennemis restants"
    );
    assert_eq!(
        chain.format_count("hud.enemies", "", 1, &[]),
        "1 ennemi restant"
    );
    // Missing in French: English wins.
    assert_eq!(chain.format("hud.ammo", "", &[("count", "12")]), "Ammo: 12");
    // Unknown language: only the default.
    let de = Localization::negotiate("de-DE", &available, "en");
    assert_eq!(de.language(), Some("en"));
    // Missing arguments stay visible; escaped braces are literal.
    assert_eq!(de.format("hud.ammo", "", &[]), "Ammo: {count}");
    assert_eq!(de.format("hud.literal", "", &[]), "Use {braces} \n here");
    assert_eq!(
        de.format("nope", "Fallback {x}", &[("x", "1")]),
        "Fallback 1"
    );
}

#[test]
fn ui_installs_localizations_reports_missing_keys_and_reflows() {
    let en = StringTable::parse("en", EN).unwrap();
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(
        Widget::Panel {
            layout: quasar_ui::PanelLayout::Stack {
                direction: quasar_ui::StackDirection::Vertical,
                gap: 0.0,
            },
            padding: Edges::ZERO,
            clip: false,
            scrollable: false,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    let label = ui
        .add_child(
            root,
            Widget::label(UiText::keyed("menu.start", "START"), TextStyle::Body),
            LayoutItem::auto(),
        )
        .unwrap();
    ui.add_child(
        root,
        Widget::label(UiText::key("menu.untranslated"), TextStyle::Body),
        LayoutItem::auto(),
    )
    .unwrap();
    let width = |ui: &mut Ui| {
        ui.layout(
            Rect::new(0.0, 0.0, 800.0, 600.0),
            1.0,
            Edges::ZERO,
            &MonospaceMeasure,
        );
        ui.layout_entries()
            .iter()
            .find(|e| e.id == label)
            .unwrap()
            .rect
            .width
    };
    let fallback = width(&mut ui);
    let revision = ui.locale_revision();
    ui.set_localization(&Localization::new(vec![en.clone()]));
    assert!(ui.locale_revision() > revision);
    assert_eq!(ui.language(), Some("en"));
    let english = width(&mut ui);
    assert!(
        english > fallback,
        "longer text re-measures on the next layout"
    );
    assert_eq!(ui.missing_keys(), vec!["menu.untranslated"]);
    // Pseudo-localisation lengthens strings so overflow shows up in QA.
    ui.set_localization(&Localization::new(vec![StringTable::pseudo(&en)]));
    assert!(width(&mut ui) > english * 1.3);
    let pseudo = StringTable::pseudo(&en);
    assert!(pseudo.get("hud.ammo").unwrap().contains("{count}"));
}
