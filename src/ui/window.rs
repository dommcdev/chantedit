//! The main window: file handling, the page view and keyboard/mouse input.
//! Editing itself happens in [`Editor`]; this module applies its effects.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk, pango};

use chantedit::document::{self, Document, Settings};
use chantedit::layout::{self, ChordFont, LineRef};
use chantedit::prefs::Prefs;
use chantedit::{export, pdf};

use super::editor::{Editor, Effect, NUDGE, NUDGE_BIG, V_STEP};
use super::files;
use super::page::PageView;
use super::render::{Msg, Workers};
use super::shortcuts;
use super::sidebar::Sidebar;

const PAGE_MARGIN: f64 = 24.0;
const PAGE_SPACING: f64 = 24.0;
const DEFAULT_ZOOM: f64 = 1.4;
const ZOOM_RANGE: (f64, f64) = (0.3, 6.0);
/// Re-render page images this long after the zoom stops changing.
const RENDER_DELAY: Duration = Duration::from_millis(120);

const DOC_ACTIONS: &[&str] = &[
    "save",
    "save-as",
    "export",
    "export-as",
    "next-file",
    "prev-file",
    "zoom-in",
    "zoom-out",
    "zoom-fit",
    "remove-line",
    "reset-lines",
];

#[derive(Default)]
struct RenderState {
    /// Device pixels per point wanted for page images.
    scale: f64,
    epoch: u64,
    requested: HashSet<usize>,
}

/// What a right-click was on.
#[derive(Clone, Copy)]
struct ContextTarget {
    page: usize,
    y: f64,
    chord: Option<u32>,
    line: Option<LineRef>,
}

pub struct Window {
    app: adw::Application,
    pub win: adw::ApplicationWindow,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    split: adw::OverlaySplitView,
    stack: gtk::Stack,
    scroller: gtk::ScrolledWindow,
    pages_box: gtk::Box,
    context_menu: gtk::PopoverMenu,
    sb: Sidebar,
    pages: RefCell<Vec<PageView>>,
    editor: RefCell<Option<Editor>>,
    prefs: RefCell<Prefs>,
    zoom: Cell<f64>,
    fit_pending: Cell<bool>,
    doc_id: Cell<u64>,
    workers: RefCell<Option<Workers>>,
    render: RefCell<RenderState>,
    render_timer: RefCell<Option<glib::SourceId>>,
    msgs: async_channel::Sender<Msg>,
    pointer: Cell<Option<(f64, f64)>>,
    pinch_start: Cell<f64>,
    context: Cell<Option<ContextTarget>>,
    /// Suppresses widget signal handlers while widgets are updated from code.
    syncing: Cell<bool>,
    may_close: Cell<bool>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Window> {
        let prefs = Prefs::load();
        let zoom = prefs.zoom.unwrap_or(DEFAULT_ZOOM);
        let win = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(1300)
            .default_height(950)
            .title("ChantEdit")
            .build();

        // Header bar
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new("ChantEdit", "");
        header.set_title_widget(Some(&title));
        let open = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Open (Ctrl+O)")
            .action_name("win.open")
            .build();
        header.pack_start(&open);
        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        nav.add_css_class("linked");
        for (icon, tip, action) in [
            (
                "go-previous-symbolic",
                "Previous Piece in Folder (Alt+Page Up)",
                "win.prev-file",
            ),
            (
                "go-next-symbolic",
                "Next Piece in Folder (Alt+Page Down)",
                "win.next-file",
            ),
        ] {
            nav.append(
                &gtk::Button::builder()
                    .icon_name(icon)
                    .tooltip_text(tip)
                    .action_name(action)
                    .build(),
            );
        }
        header.pack_start(&nav);

        let menu = gio::Menu::new();
        let section = |items: &[(&str, &str)]| {
            let s = gio::Menu::new();
            for (label, action) in items {
                s.append(Some(label), Some(action));
            }
            menu.append_section(None, &s);
        };
        section(&[
            ("_Save", "win.save"),
            ("Save _As…", "win.save-as"),
            ("_Export As…", "win.export-as"),
        ]);
        section(&[
            ("Zoom _In", "win.zoom-in"),
            ("Zoom _Out", "win.zoom-out"),
            ("_Fit Width", "win.zoom-fit"),
            ("Show _Guide Lines", "win.toggle-guides"),
        ]);
        section(&[
            ("_Remove Current Chord Line", "win.remove-line"),
            ("Reset All _Line Adjustments", "win.reset-lines"),
        ]);
        section(&[
            ("_Keyboard Shortcuts", "win.shortcuts"),
            ("_About ChantEdit", "win.about"),
        ]);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text("Main Menu")
            .build();
        let export = gtk::Button::builder()
            .label("Export")
            .tooltip_text("Export PDF With Chords (Ctrl+E)")
            .action_name("win.export")
            .css_classes(["suggested-action"])
            .build();
        let sidebar_button = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle Sidebar (F9)")
            .active(true)
            .build();
        header.pack_end(&menu_button);
        header.pack_end(&export);
        header.pack_end(&sidebar_button);

        // Pages
        let pages_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(PAGE_SPACING as i32)
            .margin_top(PAGE_MARGIN as i32)
            .margin_bottom(PAGE_MARGIN as i32)
            .margin_start(PAGE_MARGIN as i32)
            .margin_end(PAGE_MARGIN as i32)
            .halign(gtk::Align::Center)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&pages_box)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["chant-canvas"])
            .build();
        let context_menu = gtk::PopoverMenu::builder().has_arrow(true).build();
        context_menu.set_parent(&scroller);

        let open_button = gtk::Button::builder()
            .label("_Open…")
            .use_underline(true)
            .halign(gtk::Align::Center)
            .action_name("win.open")
            .css_classes(["pill", "suggested-action"])
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("folder-music-symbolic")
            .title("Open a Chant Score")
            .description("Open a PDF to add chords, or a saved .ce document to keep editing")
            .child(&open_button)
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scroller, Some("doc"));

        let sb = Sidebar::new();
        sb.guides.set_active(prefs.guides);
        let split = adw::OverlaySplitView::builder()
            .sidebar(&sb.root)
            .content(&stack)
            .min_sidebar_width(310.0)
            .max_sidebar_width(360.0)
            .build();
        split
            .bind_property("show-sidebar", &sidebar_button, "active")
            .bidirectional()
            .sync_create()
            .build();

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&toasts));
        win.set_content(Some(&view));

        let css = gtk::CssProvider::new();
        css.load_from_string(
            ".chant-canvas { background-color: alpha(@window_fg_color, 0.08); }
             chantpage { box-shadow: 0 1px 4px alpha(black, 0.35); }",
        );
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&win),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let (msgs, inbox) = async_channel::unbounded();
        let w = Rc::new(Window {
            app: app.clone(),
            win,
            title,
            toasts,
            split,
            stack,
            scroller,
            pages_box,
            context_menu,
            sb,
            pages: RefCell::default(),
            editor: RefCell::default(),
            prefs: RefCell::new(prefs),
            zoom: Cell::new(zoom),
            fit_pending: Cell::new(false),
            doc_id: Cell::new(0),
            workers: RefCell::default(),
            render: RefCell::default(),
            render_timer: RefCell::default(),
            msgs,
            pointer: Cell::new(None),
            pinch_start: Cell::new(zoom),
            context: Cell::new(None),
            syncing: Cell::new(false),
            may_close: Cell::new(false),
        });

        let weak = Rc::downgrade(&w);
        glib::spawn_future_local(async move {
            while let Ok(msg) = inbox.recv().await {
                let Some(w) = weak.upgrade() else { break };
                w.on_message(msg);
            }
        });

        w.add_actions();
        w.connect_signals();
        w.set_document_open(false);
        w.start_script();
        w
    }

    fn weak(self: &Rc<Self>) -> Weak<Window> {
        Rc::downgrade(self)
    }

    fn set_document_open(&self, open: bool) {
        for name in DOC_ACTIONS {
            if let Some(a) = self
                .win
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                a.set_enabled(open);
            }
        }
        self.sb.set_document_open(open);
        self.stack
            .set_visible_child_name(if open { "doc" } else { "empty" });
    }

    fn toast(&self, msg: &str) {
        self.toasts.add_toast(
            adw::Toast::builder()
                .title(glib::markup_escape_text(msg))
                .timeout(3)
                .build(),
        );
    }

    fn refocus(&self) {
        if self.editor.borrow().is_some() {
            self.sb.entry.grab_focus();
        }
    }

    // ---------------------------------------------------------------- setup

    fn add_actions(self: &Rc<Self>) {
        let add = |name: &str, f: fn(&Rc<Window>)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.weak();
            action.connect_activate(move |_, _| {
                if let Some(w) = weak.upgrade() {
                    f(&w);
                }
            });
            self.win.add_action(&action);
        };
        add("open", |w| {
            w.spawn(|w| async move { w.choose_and_open().await })
        });
        add("save", |w| {
            w.spawn(|w| async move {
                w.save(false).await;
            })
        });
        add("save-as", |w| {
            w.spawn(|w| async move {
                w.save(true).await;
            })
        });
        add("export", |w| {
            w.spawn(|w| async move { w.export(None).await })
        });
        add("export-as", |w| {
            w.spawn(|w| async move { w.export_as().await })
        });
        add("next-file", |w| w.sibling(1));
        add("prev-file", |w| w.sibling(-1));
        add("zoom-in", |w| w.set_zoom(w.zoom.get() * 1.15, None));
        add("zoom-out", |w| w.set_zoom(w.zoom.get() / 1.15, None));
        add("zoom-fit", |w| w.set_zoom(w.fit_zoom(), None));
        add("toggle-guides", |w| {
            w.sb.guides.set_active(!w.sb.guides.is_active())
        });
        add("toggle-sidebar", |w| {
            w.split.set_show_sidebar(!w.split.shows_sidebar())
        });
        add("remove-line", |w| w.edit(Editor::remove_active_line));
        add("reset-lines", |w| w.edit(Editor::reset_lines));
        add("shortcuts", |w| {
            let d = shortcuts::dialog();
            let weak = w.weak();
            d.connect_closed(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.refocus();
                }
            });
            d.present(Some(&w.win));
        });
        add("about", |w| {
            adw::AboutDialog::builder()
                .application_name("ChantEdit")
                .application_icon("folder-music-symbolic")
                .version(env!("CARGO_PKG_VERSION"))
                .comments("Add guitar chords above chant scores")
                .build()
                .present(Some(&w.win));
        });

        let ctx = gio::SimpleActionGroup::new();
        let add_ctx = |name: &str, f: fn(&mut Editor, ContextTarget)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.weak();
            action.connect_activate(move |_, _| {
                let Some(w) = weak.upgrade() else { return };
                if let Some(t) = w.context.get() {
                    w.edit(|ed| f(ed, t));
                }
            });
            ctx.add_action(&action);
        };
        add_ctx("edit-chord", |ed, t| {
            t.chord.into_iter().for_each(|id| ed.start_edit(id))
        });
        add_ctx("reset-height", |ed, t| {
            t.chord.into_iter().for_each(|id| ed.reset_chord_y(id))
        });
        add_ctx("delete-chord", |ed, t| {
            if let Some(id) = t.chord {
                ed.select_chord(id, false);
                ed.delete_selected(true);
            }
        });
        add_ctx("add-line", |ed, t| ed.add_line_at(t.page, t.y));
        add_ctx("remove-line", |ed, t| {
            t.line.into_iter().for_each(|r| ed.remove_line(r))
        });
        add_ctx("reset-line", |ed, t| {
            t.line.into_iter().for_each(|r| ed.reset_line(r))
        });
        self.win.insert_action_group("ctx", Some(&ctx));
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = self.weak();

        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let wk = weak.clone();
        key.connect_key_pressed(move |_, key, _, state| {
            let handled = wk.upgrade().is_some_and(|w| w.handle_key(key, state));
            glib::Propagation::from(!handled)
        });
        self.win.add_controller(key);

        let wk = weak.clone();
        self.win.connect_close_request(move |_| {
            let stop = wk.upgrade().is_some_and(|w| w.on_close_request());
            glib::Propagation::from(!stop)
        });

        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let wk = weak.clone();
        drop.connect_drop(move |_, value, _, _| {
            let path = value
                .get::<gdk::FileList>()
                .ok()
                .and_then(|l| l.files().first().and_then(|f| f.path()));
            let (Some(path), Some(w)) = (path, wk.upgrade()) else {
                return false;
            };
            w.spawn(|w| async move { w.open(path).await });
            true
        });
        self.win.add_controller(drop);

        // Zooming: Ctrl+scroll and touchpad pinch, anchored at the pointer.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let wk = weak.clone();
        scroll.connect_scroll(move |c, _, dy| {
            if !c
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            if let Some(w) = wk.upgrade().filter(|w| w.editor.borrow().is_some()) {
                w.set_zoom(w.zoom.get() * 1.1f64.powf(-dy), w.pointer.get());
            }
            glib::Propagation::Stop
        });
        self.scroller.add_controller(scroll);
        let motion = gtk::EventControllerMotion::new();
        let wk = weak.clone();
        motion.connect_motion(move |_, x, y| {
            if let Some(w) = wk.upgrade() {
                w.pointer.set(Some((x, y)));
            }
        });
        let wk = weak.clone();
        motion.connect_leave(move |_| {
            if let Some(w) = wk.upgrade() {
                w.pointer.set(None);
            }
        });
        self.scroller.add_controller(motion);
        let pinch = gtk::GestureZoom::new();
        let wk = weak.clone();
        pinch.connect_begin(move |_, _| {
            if let Some(w) = wk.upgrade() {
                w.pinch_start.set(w.zoom.get());
            }
        });
        let wk = weak.clone();
        pinch.connect_scale_changed(move |g, scale| {
            if let Some(w) = wk.upgrade() {
                w.set_zoom(w.pinch_start.get() * scale, g.bounding_box_center());
            }
        });
        self.scroller.add_controller(pinch);

        let vadj = self.scroller.vadjustment();
        let wk = weak.clone();
        vadj.connect_value_changed(move |_| {
            if let Some(w) = wk.upgrade() {
                w.update_textures();
            }
        });
        let wk = weak.clone();
        vadj.connect_changed(move |_| {
            let Some(w) = wk.upgrade() else { return };
            if w.fit_pending.get() && w.scroller.width() > 0 {
                w.fit_pending.set(false);
                w.set_zoom(w.fit_zoom(), None);
            }
            w.update_textures();
        });
        let wk = weak.clone();
        self.win.connect_realize(move |win| {
            let Some(surface) = win.surface() else { return };
            let wk = wk.clone();
            surface.connect_scale_notify(move |_| {
                if let Some(w) = wk.upgrade() {
                    w.update_textures();
                }
            });
        });

        let wk = weak.clone();
        self.context_menu.connect_closed(move |_| {
            if let Some(w) = wk.upgrade() {
                w.refocus();
            }
        });

        // Sidebar
        let sb = &self.sb;
        let wk = weak.clone();
        sb.entry.connect_entry_activated(move |_| {
            if let Some(w) = wk.upgrade() {
                w.edit(Editor::commit_entry);
            }
        });
        let wk = weak.clone();
        sb.entry.connect_changed(move |e| {
            let Some(w) = wk.upgrade().filter(|w| !w.syncing.get()) else {
                return;
            };
            let text = e.text().to_string();
            w.edit(|ed| ed.pending = text);
        });
        let wk = weak.clone();
        sb.font.connect_font_desc_notify(move |b| {
            let (Some(w), Some(mut desc)) = (wk.upgrade(), b.font_desc()) else {
                return;
            };
            desc.unset_fields(pango::FontMask::SIZE);
            w.update_settings(|s| s.font = desc.to_string());
            w.refocus();
        });
        let wk = weak.clone();
        sb.size.connect_value_notify(move |r| {
            if let Some(w) = wk.upgrade() {
                w.update_settings(|s| s.size = r.value());
            }
        });
        let wk = weak.clone();
        sb.avoid.connect_active_notify(move |r| {
            if let Some(w) = wk.upgrade() {
                w.update_settings(|s| s.auto_avoid = r.is_active());
            }
        });
        let wk = weak.clone();
        sb.snap.connect_active_notify(move |r| {
            if let Some(w) = wk.upgrade() {
                w.update_settings(|s| s.snap_notes = r.is_active());
            }
        });
        let wk = weak.clone();
        sb.offset.connect_value_notify(move |r| {
            if let Some(w) = wk.upgrade() {
                w.update_settings(|s| s.line_offset = r.value());
            }
        });
        let wk = weak.clone();
        sb.guides.connect_active_notify(move |r| {
            if let Some(w) = wk.upgrade() {
                w.prefs.borrow_mut().guides = r.is_active();
                w.queue_draw_pages();
            }
        });
        sb.reset.connect_activated(move |_| {
            if let Some(w) = weak.upgrade() {
                w.edit(Editor::reset_lines);
            }
        });
    }

    fn spawn<F: std::future::Future<Output = ()> + 'static>(
        self: &Rc<Self>,
        f: impl FnOnce(Rc<Window>) -> F,
    ) {
        glib::spawn_future_local(f(self.clone()));
    }

    // -------------------------------------------------------------- editing

    /// Runs an editor operation and applies its effects.
    fn edit(self: &Rc<Self>, f: impl FnOnce(&mut Editor)) {
        let effects = {
            let mut st = self.editor.borrow_mut();
            let Some(ed) = st.as_mut() else { return };
            f(ed);
            ed.take_effects()
        };
        for e in effects {
            match e {
                Effect::Toast(m) => self.toast(&m),
                Effect::Reveal { page, x, y } => self.reveal(page, x, y),
                Effect::SetEntry(text) => {
                    self.syncing.set(true);
                    self.sb.entry.set_text(&text);
                    self.sb.entry.set_position(-1);
                    self.syncing.set(false);
                }
            }
        }
        self.refresh();
    }

    fn update_settings(self: &Rc<Self>, f: impl FnOnce(&mut Settings)) {
        if self.syncing.get() {
            return;
        }
        let mut changed = None;
        self.edit(|ed| {
            let mut s = ed.doc.settings.clone();
            f(&mut s);
            if s != ed.doc.settings {
                ed.set_settings(s.clone());
                changed = Some(s);
            }
        });
        if let Some(s) = changed {
            let mut prefs = self.prefs.borrow_mut();
            // The line offset belongs to one score; the rest carries over.
            prefs.defaults = Settings {
                line_offset: 0.0,
                ..s
            };
            let _ = prefs.save();
        }
    }

    fn sync_sidebar(&self) {
        let st = self.editor.borrow();
        let Some(ed) = st.as_ref() else { return };
        let s = &ed.doc.settings;
        self.syncing.set(true);
        self.sb
            .font
            .set_font_desc(&pango::FontDescription::from_string(&s.font));
        self.sb.size.set_value(s.size);
        self.sb.avoid.set_active(s.auto_avoid);
        self.sb.snap.set_active(s.snap_notes);
        self.sb.offset.set_value(s.line_offset);
        self.sb.entry.set_text("");
        self.syncing.set(false);
    }

    fn refresh(&self) {
        let st = self.editor.borrow();
        match st.as_ref() {
            None => {
                self.title.set_title("ChantEdit");
                self.title.set_subtitle("");
                self.win.set_title(Some("ChantEdit"));
                self.sb.chord_group.set_description(None);
            }
            Some(ed) => {
                let dot = if ed.is_dirty() { "• " } else { "" };
                let name = ed.name();
                self.title.set_title(&format!("{dot}{name}"));
                self.win
                    .set_title(Some(&format!("{dot}{name} – ChantEdit")));
                let mut sub = match &ed.path {
                    Some(p) => tilde(p),
                    None => "Not saved yet".to_owned(),
                };
                if !ed.analysis_done() {
                    sub.push_str(" · finding chord lines…");
                }
                self.title.set_subtitle(&sub);
                self.sb
                    .chord_group
                    .set_description(Some(&glib::markup_escape_text(&ed.status())));
            }
        }
        drop(st);
        self.queue_draw_pages();
    }

    fn queue_draw_pages(&self) {
        for p in self.pages.borrow().iter() {
            p.queue_draw();
        }
    }

    fn handle_key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> bool {
        use gdk::Key as K;
        if self.editor.borrow().is_none() || self.win.visible_dialog().is_some() {
            return false;
        }
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        if !self.sb.entry_focused(&self.win) {
            if key == K::Escape && !self.context_menu.is_visible() {
                self.refocus();
                return true;
            }
            return false;
        }
        let empty = self.sb.entry.text().is_empty();
        let (editing, sel) = {
            let st = self.editor.borrow();
            let ed = st.as_ref().expect("document open");
            (ed.editing, ed.sel)
        };
        let dir = match key {
            K::Left | K::KP_Left | K::Up | K::KP_Up => -1,
            _ => 1,
        };
        let step = f64::from(dir);
        match key {
            K::Left | K::Right | K::KP_Left | K::KP_Right => {
                if !(empty || alt) {
                    return false;
                }
                self.edit(|ed| match (ctrl, shift) {
                    (true, _) => ed.jump_note(dir),
                    (_, true) => ed.nudge(step * NUDGE_BIG),
                    _ => ed.nudge(step * NUDGE),
                });
            }
            K::Up | K::Down | K::KP_Up | K::KP_Down => self.edit(|ed| match (ctrl, shift) {
                (true, _) => ed.adjust_line(step * V_STEP),
                (_, true) => ed.nudge_y(step * V_STEP),
                _ => ed.change_line(dir),
            }),
            K::Tab | K::KP_Tab if !ctrl => {
                self.edit(|ed| ed.select_rel(if shift { -1 } else { 1 }))
            }
            K::ISO_Left_Tab if !ctrl => self.edit(|ed| ed.select_rel(-1)),
            K::Delete | K::KP_Delete if empty && !ctrl => self.edit(|ed| ed.delete_selected(false)),
            K::BackSpace if empty => self.edit(|ed| ed.delete_selected(true)),
            K::Escape => {
                if editing {
                    self.edit(Editor::cancel_edit);
                } else if !empty {
                    self.sb.entry.set_text("");
                } else if sel.is_some() {
                    self.edit(Editor::deselect);
                }
            }
            K::F2 => {
                if let Some(id) = sel {
                    self.edit(|ed| ed.start_edit(id));
                }
            }
            K::Home if empty => self.edit(Editor::select_first),
            K::End if empty => self.edit(Editor::select_last),
            K::Page_Up | K::Page_Down if !alt => {
                let vadj = self.scroller.vadjustment();
                let step = vadj.page_size() * 0.85 * if key == K::Page_Up { -1.0 } else { 1.0 };
                vadj.set_value(vadj.value() + step);
            }
            K::z | K::Z if ctrl && empty => {
                self.edit(if shift { Editor::redo } else { Editor::undo })
            }
            K::y | K::Y if ctrl && empty => self.edit(Editor::redo),
            K::r | K::R if ctrl && empty => {
                if let Some(id) = sel {
                    self.edit(|ed| ed.reset_chord_y(id));
                }
            }
            _ => return false,
        }
        true
    }

    // --------------------------------------------------------------- pages

    fn build_pages(self: &Rc<Self>, sizes: &[(f64, f64)]) {
        while let Some(child) = self.pages_box.first_child() {
            self.pages_box.remove(&child);
        }
        let z = self.zoom.get();
        let pages: Vec<PageView> = sizes
            .iter()
            .enumerate()
            .map(|(i, &(w, h))| {
                let view = PageView::new(w, h, z);
                let weak = self.weak();
                view.set_painter(move |snap| {
                    if let Some(w) = weak.upgrade() {
                        w.draw_page(i, snap);
                    }
                });
                self.connect_page_input(&view, i);
                self.pages_box.append(&view);
                view
            })
            .collect();
        *self.pages.borrow_mut() = pages;
    }

    fn connect_page_input(self: &Rc<Self>, view: &PageView, page: usize) {
        let weak = self.weak();

        let drag = gtk::GestureDrag::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        let wk = weak.clone();
        drag.connect_drag_begin(move |g, x, y| {
            let Some(w) = wk.upgrade() else { return };
            let ctrl = g
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK);
            let z = w.zoom.get();
            w.edit(|ed| ed.press(page, x / z, y / z, ctrl));
            w.refocus();
        });
        let wk = weak.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(w) = wk.upgrade() else { return };
            let z = w.zoom.get();
            w.edit(|ed| ed.drag_to(dx / z, dy / z, 4.0 / z));
        });
        let wk = weak.clone();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(w) = wk.upgrade() {
                w.edit(Editor::release);
            }
        });
        view.add_controller(drag);

        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        let wk = weak.clone();
        click.connect_pressed(move |_, n, x, y| {
            let Some(w) = wk.upgrade().filter(|_| n == 2) else {
                return;
            };
            let z = w.zoom.get();
            w.edit(|ed| ed.double_click(page, x / z, y / z));
        });
        view.add_controller(click);

        let menu = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        let target = view.downgrade();
        menu.connect_pressed(move |_, _, x, y| {
            let (Some(w), Some(view)) = (weak.upgrade(), target.upgrade()) else {
                return;
            };
            let z = w.zoom.get();
            w.show_context_menu(&view, page, x, y, z);
        });
        view.add_controller(menu);
    }

    fn show_context_menu(self: &Rc<Self>, view: &PageView, page: usize, x: f64, y: f64, z: f64) {
        let (px, py) = (x / z, y / z);
        let target = {
            let st = self.editor.borrow();
            let Some(ed) = st.as_ref() else { return };
            let chord = ed.hit(page, px, py);
            let line = ed.line_near(page, py, 12.0);
            let tweaked = chord
                .and_then(|id| ed.doc.edits.chord(id))
                .is_some_and(|c| c.dy.is_some());
            let moved = line.is_some_and(|r| ed.layout.line(r).offset != 0.0);
            (
                ContextTarget {
                    page,
                    y: py,
                    chord,
                    line,
                },
                tweaked,
                moved,
            )
        };
        let (target, tweaked, moved) = target;
        if let Some(id) = target.chord {
            self.edit(|ed| ed.select_chord(id, false));
        }
        self.context.set(Some(target));

        let menu = gio::Menu::new();
        if target.chord.is_some() {
            let s = gio::Menu::new();
            s.append(Some("_Edit Chord"), Some("ctx.edit-chord"));
            if tweaked {
                s.append(Some("Reset Chord _Height"), Some("ctx.reset-height"));
            }
            s.append(Some("_Delete Chord"), Some("ctx.delete-chord"));
            menu.append_section(None, &s);
        }
        let s = gio::Menu::new();
        s.append(Some("_Add Chord Line Here"), Some("ctx.add-line"));
        if target.line.is_some() {
            s.append(Some("_Remove This Chord Line"), Some("ctx.remove-line"));
            if moved {
                s.append(Some("Reset This Line’s _Position"), Some("ctx.reset-line"));
            }
        }
        menu.append_section(None, &s);
        self.context_menu.set_menu_model(Some(&menu));
        let Some(p) = view.compute_point(&self.scroller, &graphene::Point::new(x as f32, y as f32))
        else {
            return;
        };
        self.context_menu.set_pointing_to(Some(&gdk::Rectangle::new(
            p.x() as i32,
            p.y() as i32,
            1,
            1,
        )));
        self.context_menu.popup();
    }

    fn draw_page(&self, page: usize, snap: &gtk::Snapshot) {
        let st = self.editor.borrow();
        let Some(ed) = st.as_ref() else { return };
        let Some(lines) = ed.layout.lines.get(page) else {
            return;
        };
        let z = self.zoom.get() as f32;
        let accent = adw::StyleManager::default().accent_color_rgba();
        let alpha = |c: &gdk::RGBA, a: f32| gdk::RGBA::new(c.red(), c.green(), c.blue(), a);
        let tight = gdk::RGBA::new(0.9, 0.45, 0.0, 1.0);
        snap.save();
        snap.scale(z, z);

        if self.prefs.borrow().guides {
            let active = ed.active_line();
            for (index, line) in lines.iter().enumerate() {
                let on = active == Some(LineRef { page, index });
                let stroke = gsk::Stroke::new(1.0 / z);
                if !on {
                    stroke.set_dash(&[3.0 / z, 3.0 / z]);
                }
                let color = alpha(
                    if line.tight { &tight } else { &accent },
                    if on { 0.95 } else { 0.45 },
                );
                snap.append_stroke(&segment(line.x0, line.y, line.x1, line.y), &stroke, &color);
            }
        }

        for c in &ed.doc.edits.chords {
            let Some(p) = ed.placed(c.id).filter(|p| p.line.page == page) else {
                continue;
            };
            if ed.sel == Some(c.id) {
                let (b, pad) = (p.bounds, 1.5);
                let rect = graphene::Rect::new(
                    (b.x0 - pad) as f32,
                    (b.y0 - pad) as f32,
                    (b.x1 - b.x0 + 2.0 * pad) as f32,
                    (b.y1 - b.y0 + 2.0 * pad) as f32,
                );
                let path = gsk::PathBuilder::new();
                path.add_rounded_rect(&gsk::RoundedRect::from_rect(rect, 2.0));
                let path = path.to_path();
                snap.append_fill(&path, gsk::FillRule::Winding, &alpha(&accent, 0.25));
                let width = if ed.editing { 2.0 } else { 1.0 };
                snap.append_stroke(&path, &gsk::Stroke::new(width / z), &alpha(&accent, 0.9));
            }
            draw_text(snap, &ed.font, &c.text, c.x, p.baseline, &gdk::RGBA::BLACK);
        }

        let previews = ed.previews();
        if !previews.is_empty() {
            let a = ed.analyses.get(page).and_then(|a| a.as_deref());
            for pv in previews.iter().filter(|p| p.line.page == page) {
                let line = ed.layout.line(pv.line);
                let text = ed.font.text(&pv.text);
                let (baseline, _, _) =
                    layout::place(&text, pv.x, None, line, &ed.font, a, &ed.doc.settings);
                draw_text(
                    snap,
                    &ed.font,
                    &pv.text,
                    pv.x,
                    baseline,
                    &alpha(&accent, 0.7),
                );
            }
        } else if let (None, Some(cur), Some(r)) = (ed.sel, ed.cursor, ed.cursor_line()) {
            if r.page == page {
                let y = ed.layout.line(r).y;
                let (x, half) = (cur.x, ed.font.cap_height() * 0.9);
                snap.append_stroke(
                    &segment(x, y - half, x, y + half),
                    &gsk::Stroke::new(1.5 / z),
                    &accent,
                );
                let (x, top, tri) = (x as f32, (y - half) as f32, 2.5);
                let path = gsk::PathBuilder::new();
                path.move_to(x - tri, top - tri);
                path.line_to(x + tri, top - tri);
                path.line_to(x, top);
                path.close();
                snap.append_fill(&path.to_path(), gsk::FillRule::Winding, &accent);
            }
        }
        snap.restore();
    }

    // ---------------------------------------------------------------- zoom

    fn device_scale(&self) -> f64 {
        self.win
            .surface()
            .map_or(f64::from(self.win.scale_factor()), |s| s.scale())
    }

    /// Top-left of a page in scrollable content coordinates at zoom `z`.
    fn page_origin(&self, page: usize, z: f64) -> (f64, f64) {
        let pages = self.pages.borrow();
        let max_w = pages
            .iter()
            .map(|p| (p.page_size().0 * z).round())
            .fold(0.0, f64::max);
        let content_w = f64::from(self.scroller.width()).max(max_w + 2.0 * PAGE_MARGIN);
        let w = pages
            .get(page)
            .map_or(0.0, |p| (p.page_size().0 * z).round());
        let y = PAGE_MARGIN
            + pages
                .iter()
                .take(page)
                .map(|p| (p.page_size().1 * z).round() + PAGE_SPACING)
                .sum::<f64>();
        ((content_w - w) / 2.0, y)
    }

    fn content_size(&self, z: f64) -> (f64, f64) {
        let pages = self.pages.borrow();
        let max_w = pages
            .iter()
            .map(|p| (p.page_size().0 * z).round())
            .fold(0.0, f64::max);
        let h: f64 = pages.iter().map(|p| (p.page_size().1 * z).round()).sum();
        let gaps = PAGE_SPACING * pages.len().saturating_sub(1) as f64;
        (max_w + 2.0 * PAGE_MARGIN, h + gaps + 2.0 * PAGE_MARGIN)
    }

    fn fit_zoom(&self) -> f64 {
        let max_w = self
            .pages
            .borrow()
            .iter()
            .map(|p| p.page_size().0)
            .fold(0.0, f64::max);
        let avail = f64::from(self.scroller.width()) - 2.0 * PAGE_MARGIN - 20.0;
        if max_w <= 0.0 || avail <= 100.0 {
            DEFAULT_ZOOM
        } else {
            avail / max_w
        }
    }

    /// Changes the zoom, keeping the page point under `anchor` (scroller
    /// coordinates; default: the centre of the view) in place.
    fn set_zoom(self: &Rc<Self>, z: f64, anchor: Option<(f64, f64)>) {
        let z = z.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        let old = self.zoom.get();
        if (z - old).abs() < 1e-4 || self.pages.borrow().is_empty() {
            return;
        }
        let (vadj, hadj) = (self.scroller.vadjustment(), self.scroller.hadjustment());
        let (vx, vy) = anchor.unwrap_or((hadj.page_size() / 2.0, vadj.page_size() / 2.0));
        let (cx, cy) = (hadj.value() + vx, vadj.value() + vy);
        let n = self.pages.borrow().len();
        let page = (0..n)
            .rev()
            .find(|&i| self.page_origin(i, old).1 <= cy)
            .unwrap_or(0);
        let (ox, oy) = self.page_origin(page, old);
        let (px, py) = ((cx - ox) / old, (cy - oy) / old);

        self.zoom.set(z);
        self.prefs.borrow_mut().zoom = Some(z);
        for p in self.pages.borrow().iter() {
            p.set_zoom(z);
        }
        // Configure the adjustments for the new size right away so the
        // anchor stays put; GTK keeps these values when it relayouts.
        let (cw, ch) = self.content_size(z);
        let (nx, ny) = self.page_origin(page, z);
        vadj.set_upper(ch.max(vadj.page_size()));
        vadj.set_value(ny + py * z - vy);
        hadj.set_upper(cw.max(hadj.page_size()));
        hadj.set_value(nx + px * z - vx);

        if let Some(id) = self.render_timer.take() {
            id.remove();
        }
        let weak = self.weak();
        let id = glib::timeout_add_local_once(RENDER_DELAY, move || {
            if let Some(w) = weak.upgrade() {
                w.render_timer.take();
                w.update_textures();
            }
        });
        self.render_timer.replace(Some(id));
    }

    /// Scrolls so that a page point is comfortably in view.
    fn reveal(&self, page: usize, x: f64, y: f64) {
        let z = self.zoom.get();
        let (ox, oy) = self.page_origin(page, z);
        let scroll = |adj: gtk::Adjustment, target: f64, max_margin: f64| {
            let margin = (adj.page_size() / 4.0).min(max_margin);
            if target < adj.value() + margin {
                adj.set_value(target - margin);
            } else if target > adj.value() + adj.page_size() - margin {
                adj.set_value(target - adj.page_size() + margin);
            }
        };
        scroll(self.scroller.vadjustment(), oy + y * z, 140.0);
        let hadj = self.scroller.hadjustment();
        if hadj.upper() > hadj.page_size() + 1.0 {
            scroll(hadj, ox + x * z, 100.0);
        }
    }

    /// Requests page images for pages near the view at the current scale
    /// and frees those far away.
    fn update_textures(&self) {
        if self.render_timer.borrow().is_some() {
            return; // still zooming
        }
        let workers = self.workers.borrow();
        let Some(workers) = workers.as_ref() else {
            return;
        };
        let z = self.zoom.get();
        let scale = z * self.device_scale();
        let mut rs = self.render.borrow_mut();
        if (rs.scale - scale).abs() > 1e-6 {
            rs.scale = scale;
            rs.epoch += 1;
            workers.set_epoch(rs.epoch);
            rs.requested.clear();
        }
        let vadj = self.scroller.vadjustment();
        let (top, height) = (vadj.value(), vadj.page_size().max(1.0));
        let centre = top + height / 2.0;
        let pages = self.pages.borrow();
        let mut wanted: Vec<(f64, usize)> = Vec::new();
        for (i, p) in pages.iter().enumerate() {
            let y0 = self.page_origin(i, z).1;
            let y1 = y0 + p.page_size().1 * z;
            if y1 < top - 3.0 * height || y0 > top + 4.0 * height {
                p.set_texture(None);
                rs.requested.remove(&i);
            } else if y1 >= top - height
                && y0 <= top + 2.0 * height
                && p.texture_scale() != scale
                && !rs.requested.contains(&i)
            {
                wanted.push((((y0 + y1) / 2.0 - centre).abs(), i));
            }
        }
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, i) in wanted {
            rs.requested.insert(i);
            workers.render(i, scale, rs.epoch);
        }
    }

    fn on_message(self: &Rc<Self>, msg: Msg) {
        match msg {
            Msg::Analysed { doc, page, result } if doc == self.doc_id.get() => {
                self.edit(|ed| ed.set_analysis(page, result));
            }
            Msg::Rendered {
                doc,
                page,
                epoch,
                image,
            } if doc == self.doc_id.get() => {
                let rs = self.render.borrow();
                if epoch != rs.epoch {
                    return;
                }
                let texture = gdk::MemoryTexture::new(
                    image.width,
                    image.height,
                    gdk::MemoryFormat::B8g8r8x8,
                    &image.data,
                    image.stride,
                );
                if let Some(p) = self.pages.borrow().get(page) {
                    p.set_texture(Some((texture.upcast(), rs.scale)));
                }
            }
            Msg::Failed { doc, message } if doc == self.doc_id.get() => {
                self.toast(&format!("Could not analyse the score: {message}"));
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- files

    async fn choose_and_open(self: Rc<Self>) {
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        let all = gtk::FileFilter::new();
        all.set_name(Some("Chant Scores"));
        all.add_mime_type("application/pdf");
        all.add_suffix("pdf");
        all.add_suffix(document::EXTENSION);
        filters.append(&all);
        filters.append(&ce_filter());
        let pdfs = gtk::FileFilter::new();
        pdfs.set_name(Some("PDF Documents"));
        pdfs.add_mime_type("application/pdf");
        pdfs.add_suffix("pdf");
        filters.append(&pdfs);
        let dialog = gtk::FileDialog::builder()
            .title("Open")
            .filters(&filters)
            .default_filter(&all)
            .build();
        if let Some(dir) = self.current_dir() {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        if let Ok(path) = dialog.open_future(Some(&self.win)).await.map(|f| f.path()) {
            if let Some(path) = path {
                self.open(path).await;
            }
        }
    }

    fn current_dir(&self) -> Option<PathBuf> {
        self.editor
            .borrow()
            .as_ref()
            .and_then(Editor::dir)
            .or_else(|| self.prefs.borrow().last_dir.clone())
    }

    pub async fn open(self: Rc<Self>, path: PathBuf) {
        if self.confirm_discard().await {
            self.load(&path);
        }
    }

    fn load(self: &Rc<Self>, path: &Path) {
        let display = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let fail = |what: &str, e: &dyn std::fmt::Display| {
            self.toast(&format!("Could not open “{display}”: {what}{e}"))
        };
        // A PDF that already has a document next to it opens the document.
        let twin = files::document_for(path);
        let (doc, doc_path, source) = if files::is_document(path) || twin.exists() {
            let file = if files::is_document(path) {
                path.to_owned()
            } else {
                twin
            };
            match Document::load(&file) {
                Ok(doc) => {
                    if file != path {
                        let name = file
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        self.toast(&format!("Opened your saved chords from “{name}”"));
                    }
                    (doc, Some(file), None)
                }
                Err(e) => return fail("", &e),
            }
        } else {
            match std::fs::read(path) {
                Ok(bytes) => {
                    let defaults = self.prefs.borrow().defaults.clone();
                    (
                        Document::new(bytes, display.clone(), defaults),
                        None,
                        Some(path.to_owned()),
                    )
                }
                Err(e) => return fail("", &e),
            }
        };
        let sizes = match pdf::open(&doc.pdf) {
            Ok(p) => pdf::page_sizes(&p),
            Err(e) => return fail("not a readable PDF: ", &e),
        };
        if sizes.is_empty() {
            return fail("", &"the PDF has no pages");
        }

        let id = self.doc_id.get() + 1;
        self.doc_id.set(id);
        {
            let mut rs = self.render.borrow_mut();
            rs.epoch += 1;
            rs.scale = 0.0;
            rs.requested.clear();
            let workers = Workers::start(id, doc.pdf.clone(), self.msgs.clone());
            workers.set_epoch(rs.epoch);
            self.workers.replace(Some(workers));
        }
        let file = doc_path.clone().unwrap_or_else(|| path.to_owned());
        self.editor
            .replace(Some(Editor::new(doc, doc_path, source, sizes.clone())));

        self.build_pages(&sizes);
        self.sync_sidebar();
        self.set_document_open(true);
        self.fit_pending.set(self.prefs.borrow().zoom.is_none());
        self.scroller.vadjustment().set_value(0.0);
        {
            let mut prefs = self.prefs.borrow_mut();
            prefs.last_dir = file.parent().map(Into::into);
        }
        add_recent(&file);
        self.refresh();
        self.refocus();
        let weak = self.weak();
        glib::idle_add_local_once(move || {
            if let Some(w) = weak.upgrade() {
                w.update_textures();
            }
        });
    }

    /// Asks what to do with unsaved changes. `true` means go ahead.
    async fn confirm_discard(self: &Rc<Self>) -> bool {
        let dirty = self
            .editor
            .borrow()
            .as_ref()
            .filter(|e| e.is_dirty())
            .map(Editor::name);
        let Some(name) = dirty else { return true };
        let dialog = adw::AlertDialog::new(
            Some("Save Changes?"),
            Some(&format!(
                "“{name}” has unsaved changes. Changes which are not saved will be permanently lost."
            )),
        );
        dialog.add_responses(&[
            ("cancel", "_Cancel"),
            ("discard", "_Discard"),
            ("save", "_Save"),
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

    /// Saves the document, asking for a file name if needed. Returns whether
    /// it was saved.
    async fn save(self: &Rc<Self>, save_as: bool) -> bool {
        let (current, name, dir, source) = {
            let st = self.editor.borrow();
            let Some(ed) = st.as_ref() else { return false };
            (ed.path.clone(), ed.name(), ed.dir(), ed.source.clone())
        };
        // A document created from a PDF saves next to it as Name.ce, with no
        // dialog, so a day's work on a folder of scores is just Ctrl+S.
        let sibling = source
            .as_ref()
            .map(|p| files::document_for(p))
            .filter(|p| p.parent().is_some_and(files::is_writable));
        let path = match current
            .filter(|_| !save_as)
            .or_else(|| sibling.filter(|_| !save_as))
        {
            Some(p) => p,
            None => {
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&ce_filter());
                let dialog = gtk::FileDialog::builder()
                    .title(if save_as { "Save As" } else { "Save Chords" })
                    .initial_name(format!("{name}.{}", document::EXTENSION))
                    .filters(&filters)
                    .build();
                if let Some(dir) = dir.or_else(|| self.prefs.borrow().last_dir.clone()) {
                    dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
                }
                let Some(path) = dialog
                    .save_future(Some(&self.win))
                    .await
                    .ok()
                    .and_then(|f| f.path())
                else {
                    return false;
                };
                if files::is_document(&path) {
                    path
                } else {
                    path.with_extension(document::EXTENSION)
                }
            }
        };
        let result = self.editor.borrow().as_ref().map(|ed| ed.doc.save(&path));
        match result {
            Some(Ok(())) => {
                if let Some(ed) = self.editor.borrow_mut().as_mut() {
                    ed.mark_saved(path.clone());
                }
                add_recent(&path);
                self.refresh();
                true
            }
            Some(Err(e)) => {
                self.toast(&format!("Could not save: {e}"));
                false
            }
            None => false,
        }
    }

    fn default_export_path(&self) -> Option<(PathBuf, bool)> {
        let st = self.editor.borrow();
        let ed = st.as_ref()?;
        let name = files::export_name(&ed.name());
        let dir = ed.dir().or_else(|| self.prefs.borrow().last_dir.clone())?;
        if files::is_writable(&dir) {
            return Some((dir.join(name), false));
        }
        let fallback = self
            .prefs
            .borrow()
            .export_dir
            .clone()
            .unwrap_or_else(files::default_export_dir);
        Some((fallback.join(name), true))
    }

    async fn export_as(self: Rc<Self>) {
        let Some((default, _)) = self.default_export_path() else {
            return;
        };
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        let pdfs = gtk::FileFilter::new();
        pdfs.set_name(Some("PDF Documents"));
        pdfs.add_suffix("pdf");
        filters.append(&pdfs);
        let dialog = gtk::FileDialog::builder()
            .title("Export PDF With Chords")
            .initial_name(default.file_name().unwrap_or_default().to_string_lossy())
            .filters(&filters)
            .build();
        if let Some(dir) = default.parent() {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        if let Some(path) = dialog
            .save_future(Some(&self.win))
            .await
            .ok()
            .and_then(|f| f.path())
        {
            self.export(Some(path)).await;
        }
    }

    async fn export(self: Rc<Self>, out: Option<PathBuf>) {
        let (out, fallback) = match out {
            Some(p) => (p, false),
            None => match self.default_export_path() {
                Some(d) => d,
                None => return,
            },
        };
        let job = {
            let st = self.editor.borrow();
            let Some(ed) = st.as_ref() else { return };
            if !ed.analysis_done() {
                drop(st);
                self.toast("Still finding chord lines, try again in a moment");
                return;
            }
            let protected = [ed.source.as_deref(), ed.path.as_deref()];
            if protected.iter().flatten().any(|p| same_file(p, &out)) {
                drop(st);
                self.toast("Choose a different name: that would overwrite the score");
                return;
            }
            export::prepare(&ed.doc.pdf, &ed.export_pages(), &ed.font, &out)
        };
        let result = match job {
            Ok(job) => gio::spawn_blocking(move || job.run())
                .await
                .unwrap_or_else(|_| Err("export crashed".into())),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            self.toast(&format!("Export failed: {e}"));
            return;
        }
        if fallback {
            self.prefs.borrow_mut().export_dir = out.parent().map(Into::into);
        }
        let name = out.file_name().unwrap_or_default().to_string_lossy();
        let title = if fallback {
            format!("Folder is read-only, exported to {}", tilde(&out))
        } else {
            format!("Exported “{name}”")
        };
        let toast = adw::Toast::builder()
            .title(glib::markup_escape_text(&title))
            .button_label("Open")
            .timeout(5)
            .build();
        let weak = self.weak();
        toast.connect_button_clicked(move |_| {
            let Some(w) = weak.upgrade() else { return };
            gtk::FileLauncher::new(Some(&gio::File::for_path(&out))).launch(
                Some(&w.win),
                gio::Cancellable::NONE,
                |_| {},
            );
        });
        self.toasts.add_toast(toast);
    }

    /// Opens the previous/next piece in the current folder.
    fn sibling(self: &Rc<Self>, dir: i32) {
        let current = self
            .editor
            .borrow()
            .as_ref()
            .and_then(|ed| ed.path.clone().or(ed.source.clone()));
        let Some(current) = current else { return };
        let Some(folder) = current.parent() else {
            return;
        };
        let cur = current.to_string_lossy();
        let pieces = files::pieces_in(folder);
        let order = |p: &PathBuf| files::natural_cmp(&p.to_string_lossy(), &cur);
        let next = if dir > 0 {
            pieces.iter().find(|p| order(p).is_gt())
        } else {
            pieces.iter().rev().find(|p| order(p).is_lt())
        };
        match next {
            Some(p) => {
                let p = p.clone();
                self.spawn(|w| async move { w.open(p).await });
            }
            None if dir > 0 => self.toast("This is the last piece in the folder"),
            None => self.toast("This is the first piece in the folder"),
        }
    }

    fn on_close_request(self: &Rc<Self>) -> bool {
        let dirty = self.editor.borrow().as_ref().is_some_and(Editor::is_dirty);
        if self.may_close.get() || !dirty {
            let _ = self.prefs.borrow().save();
            glib::idle_add_local_once(|| std::process::exit(0));
            return false;
        }
        self.spawn(|w| async move {
            if w.confirm_discard().await {
                w.may_close.set(true);
                w.win.close();
            }
        });
        true
    }

    /// Closes the window (asking about unsaved changes first).
    pub fn quit(&self) {
        if let Some(d) = self.win.visible_dialog() {
            d.force_close();
        }
        self.may_close.set(true);
        self.win.close();
        self.app.quit();
        // GtkApplication keeps running while we hold a strong window ref.
        std::process::exit(0);
    }

    // ------------------------------------------------------- scripted tests

    /// `CHANTEDIT_SCRIPT` drives the window for automated testing, e.g.
    /// `type:Dm Em;enter;key:Right+ctrl;snap:/tmp/a.png;save:/tmp/a.ce;quit`.
    ///
    /// Commands: `type:TEXT`, `enter`, `key:NAME[+ctrl][+shift][+alt]`,
    /// `click:PAGE,X,Y[+ctrl]`, `drag:PAGE,X,Y,DX,DY` (points), `size:PT`,
    /// `snap:PNG`, `save:PATH`, `export:PATH`, `action:NAME`, `quit`.
    fn start_script(self: &Rc<Self>) {
        let Ok(script) = std::env::var("CHANTEDIT_SCRIPT") else {
            return;
        };
        let commands: Vec<String> = script
            .split(';')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        let weak = self.weak();
        let (mut i, mut retries) = (0, 0);
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(w) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(cmd) = commands.get(i) else {
                return glib::ControlFlow::Break;
            };
            let ready = match w.editor.borrow().as_ref() {
                None if retries < 12 => {
                    retries += 1;
                    return glib::ControlFlow::Continue;
                }
                None => true,
                Some(ed) => ed.analysis_done(),
            };
            if !ready {
                return glib::ControlFlow::Continue;
            }
            let result = w.run_command(cmd);
            if result.is_err() && cmd.starts_with("snap:") && retries < 20 {
                retries += 1; // the window may not have painted yet
                return glib::ControlFlow::Continue;
            }
            if let Err(e) = result {
                eprintln!("script: {cmd}: {e}");
            }
            let done = cmd == "quit";
            (i, retries) = (i + 1, 0);
            if done {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn run_command(self: &Rc<Self>, cmd: &str) -> Result<(), String> {
        let (name, arg) = cmd.split_once(':').unwrap_or((cmd, ""));
        let (arg, ctrl) = match arg.strip_suffix("+ctrl") {
            Some(a) => (a, true),
            None => (arg, false),
        };
        let nums = || -> Vec<f64> {
            arg.split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect()
        };
        match name {
            "type" => self.sb.entry.set_text(arg),
            "enter" => self.edit(Editor::commit_entry),
            "key" => {
                let mut parts = arg.split('+');
                let key =
                    gdk::Key::from_name(parts.next().unwrap_or_default()).ok_or("unknown key")?;
                let mut state = gdk::ModifierType::empty();
                for m in parts {
                    state |= match m {
                        "ctrl" => gdk::ModifierType::CONTROL_MASK,
                        "shift" => gdk::ModifierType::SHIFT_MASK,
                        "alt" => gdk::ModifierType::ALT_MASK,
                        _ => return Err(format!("unknown modifier {m}")),
                    };
                }
                if ctrl {
                    state |= gdk::ModifierType::CONTROL_MASK;
                }
                self.refocus();
                if !self.handle_key(key, state) {
                    return Err("key not handled".into());
                }
            }
            "click" => match nums()[..] {
                [p, x, y] => {
                    self.edit(|ed| ed.press(p as usize, x, y, ctrl));
                    self.edit(Editor::release);
                }
                _ => return Err("expected PAGE,X,Y".into()),
            },
            "drag" => match nums()[..] {
                [p, x, y, dx, dy] => {
                    let z = self.zoom.get();
                    self.edit(|ed| {
                        ed.press(p as usize, x, y, false);
                        ed.drag_to(dx / 2.0, dy / 2.0, 4.0 / z);
                        ed.drag_to(dx, dy, 4.0 / z);
                        ed.release();
                    });
                }
                _ => return Err("expected PAGE,X,Y,DX,DY".into()),
            },
            "size" => self.sb.size.set_value(arg.parse().map_err(|_| "bad size")?),
            "snap" => {
                if !self.textures_ready() {
                    return Err("pages not rendered yet".into());
                }
                self.snapshot_png(Path::new(arg))?;
            }
            "save" => {
                let path = PathBuf::from(arg);
                let result = self.editor.borrow().as_ref().map(|ed| ed.doc.save(&path));
                result.ok_or("no document")?.map_err(|e| e.to_string())?;
                if let Some(ed) = self.editor.borrow_mut().as_mut() {
                    ed.mark_saved(path);
                }
                self.refresh();
            }
            "export" => self.spawn(|w| {
                let out = PathBuf::from(arg);
                async move { w.export(Some(out)).await }
            }),
            "action" => {
                let action = self.win.lookup_action(arg).ok_or("no such action")?;
                if !action.is_enabled() {
                    return Err("action disabled".into());
                }
                action.activate(None);
            }
            "quit" => {
                self.may_close.set(true);
                self.quit();
            }
            _ => return Err("unknown command".into()),
        }
        Ok(())
    }

    fn textures_ready(&self) -> bool {
        let rs = self.render.borrow();
        let pages = self.pages.borrow();
        !rs.requested.is_empty()
            && rs
                .requested
                .iter()
                .all(|&i| pages.get(i).is_some_and(|p| p.texture_scale() == rs.scale))
            || pages.is_empty()
    }

    fn snapshot_png(&self, path: &Path) -> Result<(), String> {
        let widget: gtk::Widget = match self.win.visible_dialog() {
            Some(d) => d.upcast(),
            None => self.win.content().ok_or("no content")?,
        };
        let (w, h) = (widget.width(), widget.height());
        if w == 0 || h == 0 {
            return Err("not allocated".into());
        }
        let paintable = gtk::WidgetPaintable::new(Some(&widget));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
        let node = snapshot.to_node().ok_or("nothing drawn")?;
        let renderer = widget
            .native()
            .and_then(|n| n.renderer())
            .ok_or("no renderer")?;
        let texture = renderer.render_texture(
            &node,
            Some(&graphene::Rect::new(0.0, 0.0, w as f32, h as f32)),
        );
        texture.save_to_png(path).map_err(|e| e.to_string())
    }
}

fn segment(x0: f64, y0: f64, x1: f64, y1: f64) -> gsk::Path {
    let b = gsk::PathBuilder::new();
    b.move_to(x0 as f32, y0 as f32);
    b.line_to(x1 as f32, y1 as f32);
    b.to_path()
}

fn draw_text(
    snap: &gtk::Snapshot,
    font: &ChordFont,
    text: &str,
    cx: f64,
    baseline: f64,
    color: &gdk::RGBA,
) {
    let t = font.text(text);
    let (x, y) = font.origin(&t, cx, baseline);
    snap.save();
    snap.translate(&graphene::Point::new(x as f32, y as f32));
    snap.append_layout(&t.layout, color);
    snap.restore();
}

fn ce_filter() -> gtk::FileFilter {
    let f = gtk::FileFilter::new();
    f.set_name(Some("ChantEdit Documents"));
    f.add_mime_type(document::MIME_TYPE);
    f.add_suffix(document::EXTENSION);
    f
}

fn add_recent(path: &Path) {
    gtk::RecentManager::default().add_item(&gio::File::for_path(path).uri());
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// A path with the home folder shown as `~`.
fn tilde(path: &Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
