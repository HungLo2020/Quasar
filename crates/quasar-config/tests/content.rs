//! Content tables: validation, cross-table references and reload safety.
#![cfg(all(feature = "toml", feature = "json"))]
use quasar_config::content::{
    ContentFormat, ContentRecord, ContentRegistry, ContentTable, Validator,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Weapon {
    id: String,
    damage: u32,
    rate: f64,
    ammo: String,
}
impl ContentRecord for Weapon {
    fn id(&self) -> &str {
        &self.id
    }
    fn validate(&self, v: &mut Validator) {
        v.range("damage", self.damage, 1, 100)
            .finite("rate", self.rate)
            .check(self.rate > 0.0, "rate", "must be positive")
            .reference("ammo", "ammo", &self.ammo);
    }
}

#[derive(Debug, Deserialize)]
struct Ammo {
    id: String,
    stack: u32,
}
impl ContentRecord for Ammo {
    fn id(&self) -> &str {
        &self.id
    }
}

const AMMO: &str = "[[ammo]]\nid = \"cell\"\nstack = 40\n[[ammo]]\nid = \"slug\"\nstack = 12\n";
const WEAPONS: &str = "[[weapons]]\nid = \"laser\"\ndamage = 3\nrate = 6.0\nammo = \"cell\"\n";

fn registry() -> ContentRegistry {
    let mut registry = ContentRegistry::new();
    registry
        .register::<Weapon>("weapons")
        .register::<Ammo>("ammo");
    registry
}

#[test]
fn every_problem_is_reported_with_its_record_and_field() {
    let bad = "[[weapons]]\nid = \"a\"\ndamage = 0\nrate = -1.0\nammo = \"cell\"\n\
               [[weapons]]\nid = \"a\"\ndamage = 5\nrate = 1.0\nammo = \"cell\"\n\
               [[weapons]]\nid = \"typo\"\ndamage = 5\nrate = 1.0\nammo = \"cell\"\nrange = 3\n";
    let errors = ContentTable::<Weapon>::parse("weapons", bad, ContentFormat::Toml)
        .unwrap_err()
        .0;
    let text: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(errors.len(), 4, "{text:#?}");
    assert!(text[0].starts_with("weapons[a].damage: 0 is outside 1..=100"));
    assert!(text[1].starts_with("weapons[a].rate: must be positive"));
    assert!(text[2].contains("duplicate id"));
    assert!(text[3].starts_with("weapons[typo]") && text[3].contains("range"));
    assert!(ContentTable::<Weapon>::parse("weapons", "[[guns]]\n", ContentFormat::Toml).is_err());
    let json = r#"{"ammo":[{"id":"cell","stack":40}]}"#;
    let table = ContentTable::<Ammo>::parse("ammo", json, ContentFormat::Json).unwrap();
    assert_eq!(table.get("cell").unwrap().stack, 40);
}

#[test]
fn registries_check_references_and_keep_old_content_on_bad_reloads() {
    let registry = registry();
    // A weapon referencing missing ammo fails as a whole.
    let missing = registry.load_all(&[("weapons", WEAPONS, ContentFormat::Toml)]);
    assert!(
        missing
            .unwrap_err()
            .to_string()
            .contains("unknown ammo id `cell`")
    );
    assert_eq!(registry.snapshot().revision(), 0);
    let revision = registry
        .load_all(&[
            ("ammo", AMMO, ContentFormat::Toml),
            ("weapons", WEAPONS, ContentFormat::Toml),
        ])
        .unwrap();
    let before = registry.snapshot();
    assert_eq!(before.revision(), revision);
    assert_eq!(
        before
            .table::<Weapon>("weapons")
            .unwrap()
            .get("laser")
            .unwrap()
            .damage,
        3
    );
    assert!(
        before.table::<Ammo>("weapons").is_none(),
        "wrong type is None"
    );
    // Removing ammo that a weapon still uses is rejected; content stays.
    let orphan = registry.reload_str(
        "ammo",
        "[[ammo]]\nid = \"slug\"\nstack = 1\n",
        ContentFormat::Toml,
    );
    assert!(orphan.is_err());
    assert_eq!(registry.snapshot().revision(), revision);
    // A good reload swaps in; old snapshots stay consistent.
    let stronger = WEAPONS.replace("damage = 3", "damage = 9");
    registry
        .reload_str("weapons", &stronger, ContentFormat::Toml)
        .unwrap();
    let after = registry.snapshot();
    assert!(after.revision() > before.revision());
    assert_eq!(
        after
            .table::<Weapon>("weapons")
            .unwrap()
            .get("laser")
            .unwrap()
            .damage,
        9
    );
    assert_eq!(
        before
            .table::<Weapon>("weapons")
            .unwrap()
            .get("laser")
            .unwrap()
            .damage,
        3
    );
}

#[test]
fn file_backed_tables_reload_when_their_paths_change() {
    let dir = tempfile::tempdir().unwrap();
    let ammo = dir.path().join("ammo.toml");
    let weapons = dir.path().join("weapons.toml");
    std::fs::write(&ammo, AMMO).unwrap();
    std::fs::write(&weapons, WEAPONS).unwrap();
    let mut registry = registry();
    registry
        .load_files(&[("ammo", &ammo), ("weapons", &weapons)])
        .unwrap();
    let unrelated = dir.path().join("readme.txt");
    assert_eq!(registry.reload_paths([unrelated.as_path()]).unwrap(), None);
    std::fs::write(&ammo, AMMO.replace("stack = 40", "stack = 50")).unwrap();
    let revision = registry.reload_paths([ammo.as_path()]).unwrap().unwrap();
    let snapshot = registry.snapshot();
    assert_eq!(snapshot.revision(), revision);
    assert_eq!(
        snapshot
            .table::<Ammo>("ammo")
            .unwrap()
            .get("cell")
            .unwrap()
            .stack,
        50
    );
    std::fs::write(&ammo, "not toml [").unwrap();
    assert!(registry.reload_paths([ammo.as_path()]).is_err());
    assert_eq!(
        registry.snapshot().revision(),
        revision,
        "broken edits keep content"
    );
}

#[test]
fn field_errors_and_dangling_references_are_reported_together() {
    let registry = registry();
    // One bad field (damage 0) and one dangling reference (ammo "rocket")
    // in different records, plus a reference from a valid record: all in one
    // pass.
    let weapons = "[[weapons]]\nid = \"laser\"\ndamage = 0\nrate = 6.0\nammo = \"cell\"\n\
                   [[weapons]]\nid = \"launcher\"\ndamage = 9\nrate = 1.0\nammo = \"rocket\"\n";
    let errors = registry
        .load_all(&[
            ("ammo", AMMO, ContentFormat::Toml),
            ("weapons", weapons, ContentFormat::Toml),
        ])
        .unwrap_err();
    let messages: Vec<String> = errors.0.iter().map(|e| e.to_string()).collect();
    assert_eq!(errors.0.len(), 2, "{messages:?}");
    assert!(
        messages
            .iter()
            .any(|m| m.contains("laser") && m.contains("damage"))
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("launcher") && m.contains("rocket"))
    );

    // A record that fails to deserialise still counts as an id, so a
    // reference to it is not reported as dangling on top of its own error.
    let ammo = "[[ammo]]\nid = \"cell\"\nstack = \"many\"\n";
    let errors = registry
        .load_all(&[
            ("ammo", ammo, ContentFormat::Toml),
            ("weapons", WEAPONS, ContentFormat::Toml),
        ])
        .unwrap_err();
    assert_eq!(errors.0.len(), 1, "{:?}", errors.0);
    assert_eq!(errors.0[0].table, "ammo");

    // A table whose document does not parse has unknown ids: references to
    // it are not guessed at.
    let errors = registry
        .load_all(&[
            ("ammo", "[[ammo]\nbroken", ContentFormat::Toml),
            ("weapons", weapons, ContentFormat::Toml),
        ])
        .unwrap_err();
    assert_eq!(errors.0.len(), 2, "{:?}", errors.0);
    assert!(errors.0.iter().any(|e| e.table == "ammo"));
    assert!(
        errors
            .0
            .iter()
            .any(|e| e.record.as_deref() == Some("laser"))
    );
    // Nothing was committed.
    assert_eq!(registry.snapshot().revision(), 0);
}
