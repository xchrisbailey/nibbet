use std::fs;

use nibbet_store::{EditorMode, Settings, Space};

#[test]
fn missing_settings_file_gives_defaults_under_home() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings::load(&dir.path().join("settings.toml"), dir.path());

    assert_eq!(settings.data_root, dir.path().join("nibbet"));
    assert_eq!(settings.editor_mode, EditorMode::Block);
    assert_eq!(settings.last_space, Space::Personal);
}

#[test]
fn settings_round_trip_and_bad_fields_fall_back_individually() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/settings.toml");

    let mut settings = Settings::load(&path, dir.path());
    settings.data_root = "/tmp/elsewhere".into();
    settings.editor_mode = EditorMode::Raw;
    settings.last_space = Space::Work;
    settings.save(&path).unwrap();
    assert_eq!(Settings::load(&path, dir.path()), settings);

    fs::write(
        &path,
        "last_space = \"play\"\neditor_mode = \"raw\"\nunknown = 1\n",
    )
    .unwrap();
    let loaded = Settings::load(&path, dir.path());
    assert_eq!(loaded.editor_mode, EditorMode::Raw);
    assert_eq!(loaded.last_space, Space::Personal);
    assert_eq!(loaded.data_root, dir.path().join("nibbet"));

    fs::write(&path, "this is = = not toml").unwrap();
    assert_eq!(
        Settings::load(&path, dir.path()).editor_mode,
        EditorMode::Block
    );
}

// ---- Carrying a sett install over to nibbet ----------------------------------

use std::path::Path;

use nibbet_store::SettMigration;

const SETT_SETTINGS: &str = "Library/Application Support/sett/settings.toml";

/// A data root an older build (called sett) wrote: its AGENTS.md block
/// carries sett's markers.
fn sett_data(root: &Path) {
    fs::create_dir_all(root.join("personal/notes")).unwrap();
    fs::write(root.join("personal/notes/plan.md"), "the plan\n").unwrap();
    fs::write(
        root.join("AGENTS.md"),
        "<!-- sett:begin -->\n# sett data\n<!-- sett:end -->\n",
    )
    .unwrap();
}

fn sett_settings(home: &Path, data_root: &Path) {
    let path = home.join(SETT_SETTINGS);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("data_root = {:?}\nlast_space = \"work\"\n", data_root),
    )
    .unwrap();
}

#[test]
fn a_sett_install_at_the_default_place_moves_to_nibbet() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    sett_data(&home.join("sett"));
    sett_settings(home, &home.join("sett"));

    let outcome = Settings::migrate_from_sett(home).unwrap();

    assert!(matches!(outcome, SettMigration::Moved), "{outcome:?}");
    assert!(!home.join("sett").exists());
    assert_eq!(
        fs::read_to_string(home.join("nibbet/personal/notes/plan.md")).unwrap(),
        "the plan\n"
    );
    let settings = Settings::load(&Settings::path_in(home), home);
    assert_eq!(settings.data_root, home.join("nibbet"));
    assert_eq!(settings.last_space, Space::Work);
    // The old settings file stays, and a second launch changes nothing.
    assert!(home.join(SETT_SETTINGS).is_file());
    assert!(matches!(
        Settings::migrate_from_sett(home).unwrap(),
        SettMigration::NotNeeded
    ));
}

/// Data the user put somewhere else stays there; only the settings come over.
#[test]
fn a_sett_data_root_elsewhere_stays_put() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let elsewhere = home.join("Dropbox/notes");
    sett_data(&elsewhere);
    sett_settings(home, &elsewhere);

    let outcome = Settings::migrate_from_sett(home).unwrap();

    assert!(
        matches!(outcome, SettMigration::SettingsOnly),
        "{outcome:?}"
    );
    assert!(elsewhere.join("personal/notes/plan.md").is_file());
    assert!(!home.join("nibbet").exists());
    let settings = Settings::load(&Settings::path_in(home), home);
    assert_eq!(settings.data_root, elsewhere);
}

/// `~/sett` moves only when it is sett's data and `~/nibbet` is free.
#[test]
fn a_sett_folder_is_moved_only_when_it_is_safe() {
    // A `~/sett` that isn't sett's, with no sett settings: nothing happens.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    fs::create_dir_all(home.join("sett")).unwrap();
    fs::write(home.join("sett/badger.txt"), "a sett is a badger's den\n").unwrap();
    let outcome = Settings::migrate_from_sett(home).unwrap();
    assert!(matches!(outcome, SettMigration::NotNeeded), "{outcome:?}");
    assert!(home.join("sett/badger.txt").is_file());
    assert!(!Settings::path_in(home).exists());

    // `~/nibbet` already exists: the settings come over, still on `~/sett`.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    sett_data(&home.join("sett"));
    sett_settings(home, &home.join("sett"));
    fs::create_dir_all(home.join("nibbet")).unwrap();
    let outcome = Settings::migrate_from_sett(home).unwrap();
    assert!(
        matches!(outcome, SettMigration::SettingsOnly),
        "{outcome:?}"
    );
    assert!(home.join("sett/personal/notes/plan.md").is_file());
    let settings = Settings::load(&Settings::path_in(home), home);
    assert_eq!(settings.data_root, home.join("sett"));
}
