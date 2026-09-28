//! The sidebar: chord entry, text settings and line options.

use adw::prelude::*;

pub struct Sidebar {
    pub root: adw::PreferencesPage,
    pub entry: adw::EntryRow,
    pub chord_group: adw::PreferencesGroup,
    pub text_group: adw::PreferencesGroup,
    pub line_group: adw::PreferencesGroup,
    pub font: gtk::FontDialogButton,
    pub size: adw::SpinRow,
    pub avoid: adw::SwitchRow,
    pub snap: adw::SwitchRow,
    pub guides: adw::SwitchRow,
    pub offset: adw::SpinRow,
    pub reset: adw::ButtonRow,
}

impl Sidebar {
    pub fn new() -> Sidebar {
        let root = adw::PreferencesPage::new();

        let chord_group = adw::PreferencesGroup::builder().title("Chord").build();
        let entry = adw::EntryRow::builder()
            .title("Type a chord, press Enter")
            .build();
        chord_group.add(&entry);
        root.add(&chord_group);

        let text_group = adw::PreferencesGroup::builder().title("Chord Text").build();
        let font = gtk::FontDialogButton::builder()
            .dialog(&gtk::FontDialog::new())
            .level(gtk::FontLevel::Face)
            .use_size(false)
            .valign(gtk::Align::Center)
            .build();
        let font_row = adw::ActionRow::builder().title("Font").build();
        font_row.add_suffix(&font);
        text_group.add(&font_row);
        let size = adw::SpinRow::builder()
            .title("Size")
            .subtitle("Points")
            .digits(1)
            .adjustment(&gtk::Adjustment::new(9.0, 3.0, 40.0, 0.5, 2.0, 0.0))
            .build();
        text_group.add(&size);
        let avoid = adw::SwitchRow::builder()
            .title("Avoid Notes Automatically")
            .subtitle("Shift a chord up or down if it would touch the music")
            .build();
        text_group.add(&avoid);
        let snap = adw::SwitchRow::builder()
            .title("Snap to Notes")
            .subtitle("Clicks and new chords land on the nearest note")
            .build();
        text_group.add(&snap);
        root.add(&text_group);

        let line_group = adw::PreferencesGroup::builder()
            .title("Chord Lines")
            .build();
        let guides = adw::SwitchRow::builder().title("Show Guide Lines").build();
        line_group.add(&guides);
        let offset = adw::SpinRow::builder()
            .title("Move All Lines")
            .subtitle("Points, negative is up")
            .digits(1)
            .adjustment(&gtk::Adjustment::new(0.0, -40.0, 40.0, 0.5, 2.0, 0.0))
            .build();
        line_group.add(&offset);
        let reset = adw::ButtonRow::builder()
            .title("Reset Line Adjustments")
            .build();
        line_group.add(&reset);
        root.add(&line_group);

        Sidebar {
            root,
            entry,
            chord_group,
            text_group,
            line_group,
            font,
            size,
            avoid,
            snap,
            guides,
            offset,
            reset,
        }
    }

    /// Whether keyboard focus is in the chord entry.
    pub fn entry_focused(&self, window: &impl IsA<gtk::Window>) -> bool {
        gtk::prelude::GtkWindowExt::focus(window.as_ref()).is_some_and(|f| {
            f == *self.entry.upcast_ref::<gtk::Widget>() || f.is_ancestor(&self.entry)
        })
    }

    pub fn set_document_open(&self, open: bool) {
        self.chord_group.set_sensitive(open);
        self.text_group.set_sensitive(open);
        self.line_group.set_sensitive(open);
    }
}
