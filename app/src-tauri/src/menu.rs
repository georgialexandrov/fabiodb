//! The native menu bar. Items carry the app's shortcuts so they're findable;
//! the page sees ⌘ keys first and handles them itself (preventDefault keeps
//! the menu from firing twice). A menu click is sent to the page as a
//! `menu` event with the item's id.

use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Runtime};

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let item = |id: &str, label: &str, shortcut: Option<&str>| MenuItem::with_id(app, id, label, true, shortcut);
    let about = AboutMetadata {
        name: Some("Fabio".into()),
        version: Some(app.package_info().version.to_string()),
        comments: Some("Fabio keeps watch over your databases. He whistles once when something’s wrong.".into()),
        ..Default::default()
    };

    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                "Fabio",
                true,
                &[
                    &PredefinedMenuItem::about(app, Some("About Fabio"), Some(about))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::show_all(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "File",
                true,
                &[
                    &item("new-connection", "New Connection…", Some("CmdOrCtrl+N"))?,
                    &item("open-sqlite", "Open SQLite File…", Some("CmdOrCtrl+O"))?,
                    &item("scan-folder", "Find Databases in Folder…", None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &item("new-query", "New Query", Some("CmdOrCtrl+T"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &item("close-tab", "Close Tab", Some("CmdOrCtrl+W"))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "View",
                true,
                &[
                    &item("palette", "Command Palette", Some("CmdOrCtrl+K"))?,
                    &item("switcher", "Switch Connection…", Some("CmdOrCtrl+Shift+K"))?,
                    &item("databases", "Switch Database…", Some("CmdOrCtrl+D"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &item("row-pane", "Row Details", Some("CmdOrCtrl+I"))?,
                    &item("theme", "Next Theme (System, Light, Dark)", Some("CmdOrCtrl+Shift+L"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::fullscreen(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Window",
                true,
                &[&PredefinedMenuItem::minimize(app, None)?, &PredefinedMenuItem::maximize(app, None)?],
            )?,
        ],
    )
}

pub fn forward<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let _ = app.emit("menu", id);
}
