//! The keyboard shortcuts dialog (F1).

struct Item {
    title: &'static str,
    accel: &'static str,
    subtitle: &'static str,
}

const fn item(title: &'static str, accel: &'static str, subtitle: &'static str) -> Item {
    Item {
        title,
        accel,
        subtitle,
    }
}

const SECTIONS: &[(&str, &[Item])] = &[
    (
        "Adding Chords",
        &[
            item(
                "Add Typed Chord",
                "Return",
                "Type several separated by spaces to add them on consecutive notes",
            ),
            item("Edit Selected Chord", "F2", "Or double-click the chord"),
            item("Delete Selected Chord", "BackSpace Delete", ""),
            item("Cancel Edit / Deselect", "Escape", ""),
            item("Undo", "<Control>z", ""),
            item("Redo", "<Control><Shift>z <Control>y", ""),
        ],
    ),
    (
        "Moving Chords",
        &[
            item(
                "Nudge Left / Right",
                "Left Right",
                "Or drag the chord with the mouse",
            ),
            item("Nudge in Bigger Steps", "<Shift>Left <Shift>Right", ""),
            item(
                "Jump to Previous / Next Note",
                "<Control>Left <Control>Right",
                "",
            ),
            item("Move to Previous / Next Line", "Up Down", ""),
            item("Raise / Lower Only This Chord", "<Shift>Up <Shift>Down", ""),
            item("Reset Chord Height", "<Control>r", ""),
        ],
    ),
    (
        "Selecting",
        &[
            item("Select Next / Previous Chord", "Tab <Shift>Tab", ""),
            item("Select First / Last Chord", "Home End", ""),
            item("Scroll Page", "Page_Up Page_Down", ""),
        ],
    ),
    (
        "Chord Lines",
        &[
            item("Raise / Lower Whole Line", "<Control>Up <Control>Down", ""),
            item(
                "Remove Current Line",
                "<Control><Shift>Delete",
                "Ctrl+click on the score to add a line where one is missing",
            ),
            item("Show / Hide Guide Lines", "<Control>g", ""),
        ],
    ),
    (
        "Files",
        &[
            item("Open", "<Control>o", "PDF scores or saved .ce documents"),
            item("Save", "<Control>s", ""),
            item("Save As", "<Control><Shift>s", ""),
            item("Export PDF With Chords", "<Control>e", ""),
            item("Export As", "<Control><Shift>e", ""),
            item(
                "Previous / Next Piece in Folder",
                "<Alt>Page_Up <Alt>Page_Down",
                "",
            ),
            item("Quit", "<Control>q", ""),
        ],
    ),
    (
        "View",
        &[
            item(
                "Zoom In / Out",
                "<Control>plus <Control>minus",
                "Or Ctrl+scroll",
            ),
            item("Fit Page Width", "<Control>0", ""),
            item("Toggle Sidebar", "F9", ""),
            item("Keyboard Shortcuts", "F1 <Control>question", ""),
        ],
    ),
];

pub fn dialog() -> adw::ShortcutsDialog {
    let dialog = adw::ShortcutsDialog::new();
    for (title, items) in SECTIONS {
        let section = adw::ShortcutsSection::new(Some(title));
        for it in *items {
            let item = adw::ShortcutsItem::new(it.title, it.accel);
            if !it.subtitle.is_empty() {
                item.set_subtitle(it.subtitle);
            }
            section.add(item);
        }
        dialog.add(section);
    }
    dialog
}
