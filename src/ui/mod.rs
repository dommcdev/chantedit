//! The libadwaita front end.

mod editor;
mod files;
mod page;
mod render;
mod shortcuts;
mod sidebar;
mod window;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use window::Window;

pub const APP_ID: &str = "dev.dominic.ChantEdit";

const ACCELS: &[(&str, &[&str])] = &[
    ("win.open", &["<Control>o"]),
    ("win.save", &["<Control>s"]),
    ("win.save-as", &["<Control><Shift>s"]),
    ("win.export", &["<Control>e"]),
    ("win.export-as", &["<Control><Shift>e"]),
    (
        "win.next-file",
        &["<Alt>Page_Down", "<Control>bracketright"],
    ),
    ("win.prev-file", &["<Alt>Page_Up", "<Control>bracketleft"]),
    (
        "win.zoom-in",
        &["<Control>plus", "<Control>equal", "<Control>KP_Add"],
    ),
    ("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]),
    ("win.zoom-fit", &["<Control>0"]),
    ("win.toggle-guides", &["<Control>g"]),
    ("win.toggle-sidebar", &["F9"]),
    ("win.remove-line", &["<Control><Shift>Delete"]),
    ("win.shortcuts", &["F1", "<Control>question"]),
    ("app.quit", &["<Control>q"]),
];

pub fn run() -> glib::ExitCode {
    let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
    if std::env::var_os("CHANTEDIT_SCRIPT").is_some() {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();

    let window: Rc<RefCell<Option<Rc<Window>>>> = Rc::default();
    let ensure = {
        let window = window.clone();
        move |app: &adw::Application| -> Rc<Window> {
            let w = window
                .borrow_mut()
                .get_or_insert_with(|| Window::new(app))
                .clone();
            w.win.present();
            w
        }
    };

    app.connect_activate({
        let ensure = ensure.clone();
        move |app| {
            ensure(app);
        }
    });
    app.connect_open(move |app, files, _| {
        let w = ensure(app);
        if let Some(path) = files.first().and_then(|f| f.path()) {
            glib::spawn_future_local(w.open(path));
        }
    });

    let quit = gio::SimpleAction::new("quit", None);
    quit.connect_activate(move |_, _| {
        if let Some(w) = window.borrow().as_ref() {
            w.win.close();
        }
    });
    app.add_action(&quit);
    for (action, accels) in ACCELS {
        app.set_accels_for_action(action, accels);
    }
    app.run()
}
