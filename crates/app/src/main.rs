use std::path::Path;

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use nibbet::{
    AppModel, ArchiveProject, DeleteProject, EditProject, GoBack, GoForward, NewProject, Selection,
    Shell, ShowPersonal, ShowWork, ToggleEditorMode, UnarchiveProject,
};
use nibbet_store::{LinkTarget, SettMigration, Settings};

actions!(nibbet, [Quit]);

fn main() {
    let settings_path = Settings::default_path();
    let home = std::env::var_os("HOME").unwrap_or_default();
    // Before loading, so an install from when the app was called sett opens
    // with its own settings and data.
    if settings_path.is_some() {
        match Settings::migrate_from_sett(Path::new(&home)) {
            Ok(SettMigration::NotNeeded) => {}
            Ok(SettMigration::SettingsOnly) => eprintln!("nibbet: carried sett's settings over"),
            Ok(SettMigration::Moved) => {
                eprintln!("nibbet: carried sett's settings over and moved ~/sett to ~/nibbet")
            }
            Ok(SettMigration::MoveFailed(err)) => {
                eprintln!("nibbet: carried sett's settings over; ~/sett stays put: {err}")
            }
            Err(err) => eprintln!("nibbet: couldn't carry sett's settings over: {err}"),
        }
    }
    let settings = match &settings_path {
        Some(path) => Settings::load(path, Path::new(&home)),
        None => Settings::defaults(Path::new(&home)),
    };

    application().with_assets(nibbet::AppAssets).run(move |cx| {
        init(cx);
        nibbet::init(cx);

        // Bind before set_menus: menus copy shortcuts from the keymap at call time.
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![
            Menu::new("nibbet").items([MenuItem::action("Quit nibbet", Quit)]),
            Menu::new("View").items([MenuItem::action("Toggle Raw Markdown", ToggleEditorMode)]),
            Menu::new("Go").items([
                MenuItem::action("Back", GoBack),
                MenuItem::action("Forward", GoForward),
            ]),
            Menu::new("Space").items([
                MenuItem::action("Personal", ShowPersonal),
                MenuItem::action("Work", ShowWork),
            ]),
            Menu::new("Project").items([
                MenuItem::action("New Project\u{2026}", NewProject),
                MenuItem::action("Edit Project\u{2026}", EditProject),
                MenuItem::separator(),
                MenuItem::action("Archive Project", ArchiveProject),
                MenuItem::action("Unarchive Project", UnarchiveProject),
                MenuItem::separator(),
                MenuItem::action("Delete Project\u{2026}", DeleteProject),
            ]),
        ]);

        let data_root = settings.data_root.clone();
        let model = match AppModel::open(settings.clone(), settings_path.clone(), cx) {
            Ok(model) => model,
            Err(err) => {
                eprintln!("nibbet: couldn't open {}: {err}", data_root.display());
                cx.quit();
                return;
            }
        };

        // Before the window opens, so the Shell's first sync already sees it.
        match arg_after("--appearance").as_deref() {
            Some("light") => cx.set_window_appearance(Some(WindowAppearance::Light)),
            Some("dark") => cx.set_window_appearance(Some(WindowAppearance::Dark)),
            Some(other) => eprintln!("nibbet: --appearance {other}: not light or dark"),
            None => {}
        }
        let bounds = Bounds::centered(None, size(px(1024.), px(720.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..TitleBar::window_options()
        };
        let shell_model = model.clone();
        open_window(options, cx, |window, cx| {
            cx.new(|cx| Shell::new(shell_model, window, cx))
        })
        .expect("failed to open window");
        if std::env::args().any(|arg| arg == "--insert-menu") {
            nibbet::open_insert_menu_on_load(cx);
        }
        // After the Shell exists, so it sees the open request.
        let screen = arg_after("--screen");
        let open = arg_after("--open");
        model.update(cx, |m, cx| match (screen.as_deref(), open) {
            (Some("templates"), Some(slug)) => m.open_template(&slug, cx),
            (screen, Some(slug)) => {
                if let Some(screen) = screen {
                    eprintln!("nibbet: --screen {screen} is ignored with --open");
                }
                match m.index().resolve(&slug) {
                    LinkTarget::Found { item, .. } => m.open_item(item, cx),
                    LinkTarget::Broken => eprintln!("nibbet: --open {slug}: no such Item"),
                }
            }
            (Some("journal"), None) => m.select(Selection::Journal, cx),
            (Some("notes"), None) => m.select(Selection::Notes, cx),
            (Some("templates"), None) => m.select(Selection::Templates, cx),
            (Some(other), None) => {
                eprintln!("nibbet: --screen {other}: not journal, notes or templates")
            }
            (None, None) => {}
        });
        cx.activate(true);
    });
}

/// The value after `flag` on the command line. `scripts/screenshot` passes
/// `--open SLUG` (an Item, or a Template with `--screen templates`) and
/// `--screen journal|notes|templates` to choose what the window shows, and
/// `--appearance light|dark` to override the system's appearance.
fn arg_after(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip_while(|arg| arg != flag);
    args.next()?;
    args.next()
}
