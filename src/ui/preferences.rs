//! Chord text and placement settings in a libadwaita preferences dialog.

use adw::prelude::*;

pub struct Preferences {
    pub dialog: adw::PreferencesDialog,
    pub text_group: adw::PreferencesGroup,
    pub font: gtk::FontDialogButton,
    pub size: adw::SpinRow,
    pub avoid: adw::SwitchRow,
    pub snap: adw::SwitchRow,
    pub guides: adw::SwitchRow,
    pub offset: adw::SpinRow,
    pub reset: adw::ButtonRow,
}

impl Preferences {
    pub fn new() -> Preferences {
        let dialog = adw::PreferencesDialog::new();
        let root = adw::PreferencesPage::builder()
            .title("Score")
            .icon_name("preferences-other-symbolic")
            .build();
        dialog.add(&root);

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

        Preferences {
            dialog,
            text_group,
            font,
            size,
            avoid,
            snap,
            guides,
            offset,
            reset,
        }
    }

    pub fn set_document_open(&self, open: bool) {
        self.text_group.set_sensitive(open);
        self.offset.set_sensitive(open);
        self.reset.set_sensitive(open);
    }
}
