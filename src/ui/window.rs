//! GABC GTK front end: native SVG chant with a source-anchored chord overlay.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib, graphene};

use super::page::PageView;
use chantedit::chant::{self, Svg};
use chantedit::gabc::Document;
use chantedit::gabc_editor::Editor;

const WIDTH: f64 = 720.0;

pub struct Window {
    pub win: adw::ApplicationWindow,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    scroller: gtk::ScrolledWindow,
    content: gtk::Box,
    stack: gtk::Stack,
    pages: RefCell<Vec<PageView>>,
    svgs: RefCell<Vec<Svg>>,
    padding: RefCell<Vec<f64>>,
    editor: RefCell<Option<Editor>>,
    path: RefCell<Option<PathBuf>>,
    zoom: Cell<f64>,
    guides: Cell<bool>,
    loading: Cell<bool>,
    may_close: Cell<bool>,
    weak: RefCell<Weak<Window>>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let win = adw::ApplicationWindow::builder()
            .application(app)
            .title("ChantEdit")
            .default_width(1100)
            .default_height(850)
            .build();
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new("ChantEdit", "");
        header.set_title_widget(Some(&title));
        for (icon, tooltip, action) in [
            ("document-open-symbolic", "Open GABC (Ctrl+O)", "win.open"),
            ("document-save-symbolic", "Save GABC (Ctrl+S)", "win.save"),
            (
                "go-previous-symbolic",
                "Previous chant (Alt+Page Up)",
                "win.prev-file",
            ),
            (
                "go-next-symbolic",
                "Next chant (Alt+Page Down)",
                "win.next-file",
            ),
        ] {
            header.pack_start(
                &gtk::Button::builder()
                    .icon_name(icon)
                    .tooltip_text(tooltip)
                    .action_name(action)
                    .build(),
            );
        }
        let menu = gio::Menu::new();
        for (label, action) in [
            ("Save As…", "win.save-as"),
            ("Zoom In", "win.zoom-in"),
            ("Zoom Out", "win.zoom-out"),
            ("Fit Width", "win.zoom-fit"),
            ("Show Neume Guides", "win.toggle-guides"),
            ("Keyboard Shortcuts", "win.shortcuts"),
            ("About ChantEdit", "win.about"),
        ] {
            menu.append(Some(label), Some(action));
        }
        header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .menu_model(&menu)
                .build(),
        );
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .margin_top(20)
            .margin_bottom(20)
            .margin_start(20)
            .margin_end(20)
            .halign(gtk::Align::Center)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&content)
            .focusable(true)
            .hexpand(true)
            .vexpand(true)
            .build();
        scroller.add_css_class("chant-score");
        let score_style = gtk::CssProvider::new();
        score_style.load_from_string(".chant-score { background-color: white; }");
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&scroller),
            &score_style,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let empty = adw::StatusPage::builder()
            .icon_name("folder-music-symbolic")
            .title("Open a Gregorian Chant Score")
            .description("Open a .gabc file, click a neume, and type a chord.")
            .child(
                &gtk::Button::builder()
                    .label("Open GABC…")
                    .action_name("win.open")
                    .halign(gtk::Align::Center)
                    .build(),
            )
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scroller, Some("score"));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let toolbar = adw::ToolbarView::builder().content(&toasts).build();
        toolbar.add_top_bar(&header);
        win.set_content(Some(&toolbar));
        let this = Rc::new(Self {
            win,
            title,
            toasts,
            scroller,
            content,
            stack,
            pages: RefCell::new(Vec::new()),
            svgs: RefCell::new(Vec::new()),
            padding: RefCell::new(Vec::new()),
            editor: RefCell::new(None),
            path: RefCell::new(None),
            zoom: Cell::new(1.25),
            guides: Cell::new(false),
            loading: Cell::new(false),
            may_close: Cell::new(false),
            weak: RefCell::new(Weak::new()),
        });
        this.weak.replace(Rc::downgrade(&this));
        this.actions();
        this.events();
        this
    }

    fn weak(&self) -> Weak<Self> {
        self.weak.borrow().clone()
    }

    fn toast(&self, message: &str) {
        self.toasts
            .add_toast(adw::Toast::new(&glib::markup_escape_text(message)));
    }

    fn spawn(
        &self,
        f: impl FnOnce(Rc<Self>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> + 'static,
    ) {
        let weak = self.weak();
        glib::spawn_future_local(async move {
            if let Some(w) = weak.upgrade() {
                f(w).await;
            }
        });
    }

    fn actions(self: &Rc<Self>) {
        for name in [
            "open",
            "save",
            "save-as",
            "next-file",
            "prev-file",
            "zoom-in",
            "zoom-out",
            "zoom-fit",
            "toggle-guides",
            "shortcuts",
            "about",
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.weak();
            action.connect_activate(move |_, _| {
                let Some(w) = weak.upgrade() else {
                    return;
                };
                match name {
                    "open" => w.spawn(|w| {
                        Box::pin(async move {
                            w.choose_open().await;
                        })
                    }),
                    "save" | "save-as" => w.spawn(move |w| {
                        Box::pin(async move {
                            w.save(name == "save-as").await;
                        })
                    }),
                    "next-file" => w.sibling(1),
                    "prev-file" => w.sibling(-1),
                    "zoom-in" => w.set_zoom(w.zoom.get() * 1.15),
                    "zoom-out" => w.set_zoom(w.zoom.get() / 1.15),
                    "zoom-fit" => w.fit_width(),
                    "toggle-guides" => {
                        w.guides.set(!w.guides.get());
                        w.refresh();
                    }
                    "shortcuts" => w.help(),
                    "about" => adw::AboutDialog::builder()
                        .application_name("ChantEdit")
                        .application_icon(super::APP_ID)
                        .version(env!("CARGO_PKG_VERSION"))
                        .developer_name("Dominic")
                        .comments(
                            "Add chords to GABC chant scores. Offline preview powered by Exsurge.",
                        )
                        .license_type(gtk::License::Gpl30)
                        .build()
                        .present(Some(&w.win)),
                    _ => {}
                }
            });
            self.win.add_action(&action);
        }
    }

    fn events(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.weak();
        keys.connect_key_pressed(move |_, key, _, state| {
            weak.upgrade().map_or(glib::Propagation::Proceed, |w| {
                if w.key(key, state) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            })
        });
        self.win.add_controller(keys);
        let weak = self.weak();
        self.win.connect_close_request(move |_| {
            let Some(w) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if w.may_close.get() {
                return glib::Propagation::Proceed;
            }
            if w.loading.get() {
                return glib::Propagation::Stop;
            }
            if w.editor
                .borrow()
                .as_ref()
                .is_none_or(|ed| !ed.doc.is_dirty())
            {
                return glib::Propagation::Proceed;
            }
            w.spawn(|w| {
                Box::pin(async move {
                    if w.confirm_discard().await {
                        w.may_close.set(true);
                        w.win.close();
                    }
                })
            });
            glib::Propagation::Stop
        });
        let pinch = gtk::GestureZoom::new();
        let initial = Rc::new(Cell::new(1.0));
        let weak = self.weak();
        let start = initial.clone();
        pinch.connect_begin(move |_, _| {
            if let Some(w) = weak.upgrade() {
                start.set(w.zoom.get());
            }
        });
        let weak = self.weak();
        pinch.connect_scale_changed(move |_, scale| {
            if let Some(w) = weak.upgrade() {
                w.set_zoom(initial.get() * scale);
            }
        });
        self.scroller.add_controller(pinch);
    }

    fn edit(&self, operation: impl FnOnce(&mut Editor)) {
        if let Some(ed) = self.editor.borrow_mut().as_mut() {
            operation(ed);
        }
        self.refresh();
    }

    fn key(&self, key: gdk::Key, state: gdk::ModifierType) -> bool {
        use gdk::Key as K;
        if self.loading.get()
            || self.win.visible_dialog().is_some()
            || self.editor.borrow().is_none()
        {
            return false;
        }
        if gtk::prelude::GtkWindowExt::focus(&self.win).is_none_or(|f| {
            f != *self.scroller.upcast_ref::<gtk::Widget>() && !f.is_ancestor(&self.scroller)
        }) {
            return false;
        }
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let direction = if matches!(key, K::Left | K::KP_Left | K::Up | K::KP_Up) {
            -1
        } else {
            1
        };
        let step = f64::from(direction) * if shift { 5.0 } else { 0.5 };
        match key {
            K::Left | K::Right | K::KP_Left | K::KP_Right if !alt => self.edit(|ed| {
                if ctrl {
                    ed.nudge(step, 0.0);
                } else {
                    ed.move_anchor(direction);
                }
            }),
            K::Up | K::Down | K::KP_Up | K::KP_Down if !alt => self.edit(|ed| {
                if ctrl {
                    ed.nudge(0.0, -step);
                } else {
                    ed.move_staff(direction);
                }
            }),
            K::Tab | K::KP_Tab | K::ISO_Left_Tab if !ctrl && !alt => self.edit(|ed| {
                ed.tab(if shift || key == K::ISO_Left_Tab {
                    -1
                } else {
                    1
                })
            }),
            K::Return | K::KP_Enter if !ctrl => self.edit(Editor::finish),
            K::BackSpace if !ctrl => self.edit(Editor::backspace),
            K::Delete | K::KP_Delete if !ctrl => self.edit(Editor::delete),
            K::F2 => self.edit(Editor::start_edit),
            K::Escape => self.edit(|ed| {
                if ed.editing {
                    ed.finish();
                } else {
                    ed.selected = None;
                }
            }),
            K::Home => self.edit(|ed| ed.select(0)),
            K::End => self.edit(|ed| ed.select(ed.preview.positions.len() - 1)),
            K::z | K::Z if ctrl => self.edit(if shift { Editor::redo } else { Editor::undo }),
            K::y | K::Y if ctrl => self.edit(Editor::redo),
            K::r | K::R if ctrl => self.edit(Editor::reset_offsets),
            K::Page_Up | K::Page_Down if !alt => {
                let adj = self.scroller.vadjustment();
                adj.set_value(
                    adj.value() + adj.page_size() * if key == K::Page_Up { -0.85 } else { 0.85 },
                );
            }
            _ if !ctrl && !alt => {
                let Some(ch) = key.to_unicode().filter(|c| !c.is_control()) else {
                    return false;
                };
                self.edit(|ed| ed.type_text(&ch.to_string()));
            }
            _ => return false,
        }
        if matches!(
            key,
            K::Tab | K::ISO_Left_Tab | K::Left | K::Right | K::Up | K::Down | K::Home | K::End
        ) {
            self.reveal();
        }
        true
    }

    fn refresh(&self) {
        let state = self.editor.borrow();
        let Some(ed) = state.as_ref() else {
            return;
        };
        let filename = self
            .path
            .borrow()
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ed.doc.name.clone());
        let title = format!("{}{}", if ed.doc.is_dirty() { "• " } else { "" }, filename);
        self.title.set_title(&title);
        self.win.set_title(Some(&format!("{title} — ChantEdit")));
        let mut padding = self.padding.borrow_mut();
        for (page, view) in self.pages.borrow().iter().enumerate() {
            let top = ed
                .placed
                .iter()
                .filter(|p| p.page == page)
                .map(|p| p.top)
                .fold(8.0, f64::min);
            padding[page] = (8.0 - top).max(0.0);
            view.set_page_size(
                ed.preview.pages[page].width,
                ed.preview.pages[page].height + padding[page],
            );
            view.queue_draw();
        }
    }

    fn draw(&self, page: usize, snapshot: &gtk::Snapshot) {
        let state = self.editor.borrow();
        let Some(ed) = state.as_ref() else {
            return;
        };
        let spec = &ed.preview.pages[page];
        let padding = self.padding.borrow()[page];
        let zoom = self.zoom.get();
        let cr = snapshot.append_cairo(&graphene::Rect::new(
            0.0,
            0.0,
            (spec.width * zoom) as f32,
            ((spec.height + padding) * zoom) as f32,
        ));
        cr.scale(zoom, zoom);
        cr.translate(0.0, padding);
        let _ = self.svgs.borrow()[page].draw(&cr, spec.width, spec.height);
        let accent = adw::StyleManager::default().accent_color_rgba();
        let set_accent = |alpha| {
            cr.set_source_rgba(
                f64::from(accent.red()),
                f64::from(accent.green()),
                f64::from(accent.blue()),
                alpha,
            )
        };
        if self.guides.get() {
            set_accent(0.3);
            cr.set_line_width(0.7 / zoom);
            for p in ed.preview.positions.iter().filter(|p| p.page == page) {
                cr.move_to(p.x, p.y - 8.0);
                cr.line_to(p.x, p.note_y + 8.0);
            }
            let _ = cr.stroke();
        }
        for placed in ed.placed.iter().filter(|p| p.page == page) {
            if ed.selected == Some(placed.chord) {
                set_accent(0.2);
                cr.rectangle(
                    placed.x - 2.0,
                    placed.top - 2.0,
                    placed.width + 4.0,
                    placed.bottom - placed.top + 4.0,
                );
                let _ = cr.fill();
            }
            cr.set_source_rgb(0.0, 0.0, 0.0);
            ed.font.draw(
                &cr,
                &ed.doc.chords[placed.chord].text,
                placed.x + placed.width / 2.0,
                placed.baseline,
            );
        }
        let cursor = ed.position();
        if cursor.page == page && ed.selected.is_none() {
            set_accent(0.9);
            cr.set_line_width(1.5 / zoom);
            cr.move_to(cursor.x, cursor.y - 9.0);
            cr.line_to(cursor.x, cursor.y + 2.0);
            let _ = cr.stroke();
        }
    }

    fn build_pages(self: &Rc<Self>) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        self.pages.borrow_mut().clear();
        let state = self.editor.borrow();
        let ed = state.as_ref().unwrap();
        self.padding.replace(vec![0.0; ed.preview.pages.len()]);
        for (page, spec) in ed.preview.pages.iter().enumerate() {
            let view = PageView::new(spec.width, spec.height, self.zoom.get());
            let weak = self.weak();
            view.set_painter(move |snapshot| {
                if let Some(w) = weak.upgrade() {
                    w.draw(page, snapshot);
                }
            });
            self.page_events(&view, page);
            self.content.append(&view);
            self.pages.borrow_mut().push(view);
        }
    }

    fn page_events(self: &Rc<Self>, view: &PageView, page: usize) {
        let drag = gtk::GestureDrag::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        let start = Rc::new(Cell::new((0.0, 0.0)));
        let previous = Rc::new(Cell::new((0.0, 0.0)));
        let dragging = Rc::new(Cell::new(false));
        let fine = Rc::new(Cell::new(false));
        let weak = self.weak();
        let (st, prev, active, mode) = (
            start.clone(),
            previous.clone(),
            dragging.clone(),
            fine.clone(),
        );
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let x = x / w.zoom.get();
            let y = y / w.zoom.get() - w.padding.borrow()[page];
            active.set(
                w.editor
                    .borrow()
                    .as_ref()
                    .is_some_and(|ed| ed.hit(page, x, y).is_some()),
            );
            mode.set(
                gesture
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK),
            );
            st.set((x, y));
            prev.set((0.0, 0.0));
            w.scroller.grab_focus();
            w.edit(|ed| ed.click(page, x, y));
        });
        let weak = self.weak();
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(w) = weak.upgrade().filter(|_| dragging.get()) else {
                return;
            };
            let (dx, dy) = (dx / w.zoom.get(), dy / w.zoom.get());
            if fine.get() {
                let (px, py) = previous.replace((dx, dy));
                w.edit(|ed| ed.nudge(dx - px, py - dy));
            } else {
                w.edit(|ed| ed.drag_to(page, start.get().0 + dx));
            }
        });
        view.add_controller(drag);
        let double = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        let weak = self.weak();
        double.connect_pressed(move |_, count, x, y| {
            if count != 2 {
                return;
            }
            if let Some(w) = weak.upgrade() {
                let y = y / w.zoom.get() - w.padding.borrow()[page];
                w.edit(|ed| {
                    ed.click(page, x / w.zoom.get(), y);
                    ed.start_edit();
                });
            }
        });
        view.add_controller(double);
    }

    fn set_zoom(&self, zoom: f64) {
        self.zoom.set(zoom.clamp(0.3, 5.0));
        for view in self.pages.borrow().iter() {
            view.set_zoom(self.zoom.get());
        }
    }

    fn fit_width(&self) {
        let width = self.scroller.width();
        if width > 60 {
            self.set_zoom((f64::from(width) - 60.0) / WIDTH);
        }
    }

    fn reveal(&self) {
        let state = self.editor.borrow();
        let Some(ed) = state.as_ref() else {
            return;
        };
        let page = ed.position().page;
        let y = 20.0
            + self
                .pages
                .borrow()
                .iter()
                .take(page)
                .map(|p| p.page_size().1 * self.zoom.get())
                .sum::<f64>();
        let adj = self.scroller.vadjustment();
        if y < adj.value() {
            adj.set_value(y);
        } else if y + 100.0 > adj.value() + adj.page_size() {
            adj.set_value(y + 150.0 - adj.page_size());
        }
    }

    async fn choose_open(self: Rc<Self>) {
        let filter = gabc_filter();
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Open GABC")
            .filters(&filters)
            .default_filter(&filter)
            .build();
        if let Some(parent) = self.path.borrow().as_ref().and_then(|p| p.parent()) {
            dialog.set_initial_folder(Some(&gio::File::for_path(parent)));
        }
        if let Ok(file) = dialog.open_future(Some(&self.win)).await
            && let Some(path) = file.path()
        {
            self.open(path).await;
        }
    }

    pub async fn open(self: Rc<Self>, path: PathBuf) {
        if self.loading.get() || !self.confirm_discard().await {
            return;
        }
        if !has_gabc_ext(&path) {
            self.toast("Open a .gabc chant file");
            return;
        }
        let path = match path.canonicalize() {
            Ok(path) => path,
            Err(e) => {
                self.toast(&format!("Could not open {}: {e}", path.display()));
                return;
            }
        };
        self.loading.set(true);
        self.title.set_subtitle("Rendering chant…");
        let input = path.clone();
        let result = gio::spawn_blocking(move || -> Result<_, String> {
            let source = std::fs::read_to_string(input).map_err(|e| e.to_string())?;
            let doc = Document::parse(source)?;
            let preview = chant::render(&doc, WIDTH)?;
            Ok((doc, preview))
        })
        .await;
        self.loading.set(false);
        self.title.set_subtitle("");
        let result = result.unwrap_or_else(|_| Err("Chant renderer crashed".into()));
        match result {
            Ok((doc, preview)) => {
                let handles: Result<Vec<_>, _> =
                    preview.pages.iter().map(|p| Svg::new(&p.svg)).collect();
                let handles = match handles {
                    Ok(handles) => handles,
                    Err(e) => {
                        self.toast(&e);
                        return;
                    }
                };
                self.svgs.replace(handles);
                self.path.replace(Some(path));
                self.editor.replace(Some(Editor::new(doc, preview)));
                self.build_pages();
                self.stack.set_visible_child_name("score");
                self.scroller.vadjustment().set_value(0.0);
                self.refresh();
                self.scroller.grab_focus();
                let weak = self.weak();
                self.scroller.add_tick_callback(move |scroller, _| {
                    if let Some(w) = weak.upgrade() {
                        if scroller.width() > 60 {
                            w.fit_width();
                            return glib::ControlFlow::Break;
                        }
                        glib::ControlFlow::Continue
                    } else {
                        glib::ControlFlow::Break
                    }
                });
            }
            Err(e) => {
                self.toast(&format!("Could not open {}: {e}", path.display()));
                self.refresh();
            }
        }
    }

    async fn save(self: &Rc<Self>, save_as: bool) -> bool {
        if self.loading.get() || self.editor.borrow().is_none() {
            return false;
        }
        let current = self.path.borrow().clone();
        let path = if let Some(path) = current.as_ref().filter(|_| !save_as) {
            path.clone()
        } else {
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&gabc_filter());
            let name = current
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or("chant.gabc".into());
            let dialog = gtk::FileDialog::builder()
                .title("Save GABC As")
                .filters(&filters)
                .initial_name(name)
                .build();
            if let Some(parent) = current.as_ref().and_then(|p| p.parent()) {
                dialog.set_initial_folder(Some(&gio::File::for_path(parent)));
            }
            let Ok(file) = dialog.save_future(Some(&self.win)).await else {
                return false;
            };
            let Some(path) = file.path() else {
                return false;
            };
            if has_gabc_ext(&path) {
                path
            } else {
                path.with_extension("gabc")
            }
        };
        let result = {
            let mut state = self.editor.borrow_mut();
            let Some(ed) = state.as_mut() else {
                return false;
            };
            ed.finish();
            ed.doc.save(&path)
        };
        match result {
            Ok(()) => {
                self.path.replace(Some(path));
                self.refresh();
                true
            }
            Err(e) => {
                self.toast(&format!("Could not save: {e}"));
                false
            }
        }
    }

    async fn confirm_discard(self: &Rc<Self>) -> bool {
        if self
            .editor
            .borrow()
            .as_ref()
            .is_none_or(|ed| !ed.doc.is_dirty())
        {
            return true;
        }
        let dialog = adw::AlertDialog::new(
            Some("Save Changes?"),
            Some("Save your chords to the GABC file before continuing?"),
        );
        dialog.add_responses(&[
            ("cancel", "Cancel"),
            ("discard", "Discard"),
            ("save", "Save"),
        ]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        match dialog.choose_future(Some(&self.win)).await.as_str() {
            "discard" => true,
            "save" => self.save(false).await,
            _ => false,
        }
    }

    fn sibling(&self, direction: i32) {
        let Some(current) = self.path.borrow().clone() else {
            return;
        };
        let Some(parent) = current.parent() else {
            return;
        };
        let mut files: Vec<PathBuf> = std::fs::read_dir(parent)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| has_gabc_ext(p))
            .collect();
        files.sort();
        let Some(index) = files.iter().position(|p| *p == current) else {
            return;
        };
        let next = index as i64 + i64::from(direction);
        if let Some(path) = usize::try_from(next)
            .ok()
            .and_then(|i| files.get(i))
            .cloned()
        {
            self.spawn(|w| {
                Box::pin(async move {
                    w.open(path).await;
                })
            });
        } else {
            self.toast("No more chants in this direction");
        }
    }

    fn help(&self) {
        adw::AlertDialog::new(Some("Editing Chords"), Some(
            "Click a neume and type a chord. Typing replaces the selected chord.\n\nTab / Shift+Tab: select next / previous neume\nLeft / Right: move chord one neume\nUp / Down: move to adjacent staff\nCtrl+arrows: fine-tune chord position\nCtrl+Shift+arrows: larger adjustments\nDrag: move chord to a neume\nCtrl+drag: fine-tune position\nF2 / double-click: edit existing text\nEnter: finish typing\nBackspace: erase a character, or delete selected chord\nDelete: delete chord\nCtrl+R: reset manual offsets\nCtrl+Z / Ctrl+Shift+Z: undo / redo\nCtrl+S: save GABC\n\nOverlapping chords are raised automatically. Chant spacing stays fixed."
        )).present(Some(&self.win));
    }
}

fn has_gabc_ext(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gabc"))
}

fn gabc_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Gregorio Chant (.gabc)"));
    filter.add_suffix("gabc");
    filter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a graphical GTK session"]
    fn keyboard_save_reopen_and_close() {
        gtk::init().unwrap();
        let app = adw::Application::builder()
            .application_id("dev.dominic.ChantEdit.Test")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let dir = std::env::temp_dir().join(format!("chantedit-gtk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.gabc");
        let original = std::env::var_os("CHANTEDIT_UI_SAMPLE")
            .map(|path| std::fs::read_to_string(path).unwrap())
            .unwrap_or_else(|| "name:GTK test;\n%%\n(c4) A(ghg/h.) B(hi) C(g.) (::)\n".into());
        let original = Document::parse(original).unwrap().music;
        std::fs::write(&path, &original).unwrap();
        let w = Window::new(&app);
        w.win.present();
        glib::MainContext::default().block_on(async {
            w.clone().open(path.clone()).await;
            glib::timeout_future(std::time::Duration::from_millis(150)).await;
            w.scroller.grab_focus();
            assert!(w.key(gdk::Key::C, gdk::ModifierType::empty()));
            assert!(w.key(gdk::Key::Tab, gdk::ModifierType::empty()));
            assert!(w.key(gdk::Key::D, gdk::ModifierType::empty()));
            assert!(w.key(gdk::Key::m, gdk::ModifierType::empty()));
            assert!(w.key(gdk::Key::Right, gdk::ModifierType::CONTROL_MASK));
            assert!(w.key(gdk::Key::Up, gdk::ModifierType::CONTROL_MASK));
            let edited = w.editor.borrow().as_ref().unwrap().doc.chords.clone();
            assert_eq!(edited[1].dx, 0.5);
            assert_eq!(edited[1].dy, 0.5);
            if let Some(path) = std::env::var_os("CHANTEDIT_UI_SCREENSHOT") {
                glib::timeout_future(std::time::Duration::from_millis(150)).await;
                let paintable = gtk::WidgetPaintable::new(Some(&w.win));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(
                    &snapshot,
                    f64::from(w.win.width()),
                    f64::from(w.win.height()),
                );
                let node = snapshot.to_node().expect("rendered window");
                let texture = w.win.renderer().unwrap().render_texture(&node, None);
                texture.save_to_png(path).unwrap();
            }
            assert!(w.save(false).await);
            assert!(!w.editor.borrow().as_ref().unwrap().doc.is_dirty());
            w.clone().open(path.clone()).await;
            assert_eq!(w.editor.borrow().as_ref().unwrap().doc.chords, edited);
            assert_eq!(w.editor.borrow().as_ref().unwrap().doc.music, original);
            w.win.close();
            assert!(!w.win.is_visible());
        });
        std::fs::remove_dir_all(dir).unwrap();
    }
}
