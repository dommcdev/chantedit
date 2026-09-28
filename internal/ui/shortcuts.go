package ui

import "github.com/diamondburned/gotk4-adwaita/pkg/adw"

type shortcutItem struct{ title, accel, subtitle string }

var shortcutSections = []struct {
	title string
	items []shortcutItem
}{
	{"Adding Chords", []shortcutItem{
		{"Add Typed Chord", "Return", "Type several separated by spaces to add them on consecutive notes"},
		{"Edit Selected Chord", "F2", "Or double-click the chord"},
		{"Delete Selected Chord", "BackSpace Delete", ""},
		{"Cancel Edit / Deselect", "Escape", ""},
		{"Undo", "<Control>z", ""},
		{"Redo", "<Control><Shift>z <Control>y", ""},
	}},
	{"Moving Chords", []shortcutItem{
		{"Nudge Left / Right", "Left Right", "Or drag the chord with the mouse"},
		{"Nudge in Bigger Steps", "<Shift>Left <Shift>Right", ""},
		{"Jump to Previous / Next Note", "<Control>Left <Control>Right", ""},
		{"Move to Previous / Next Line", "Up Down", ""},
		{"Raise / Lower Only This Chord", "<Shift>Up <Shift>Down", ""},
		{"Reset Chord Height", "<Control>r", ""},
	}},
	{"Selecting", []shortcutItem{
		{"Select Next / Previous Chord", "Tab <Shift>Tab", ""},
		{"Select First / Last Chord", "Home End", ""},
		{"Scroll Page", "Page_Up Page_Down", ""},
	}},
	{"Chord Lines", []shortcutItem{
		{"Raise / Lower Whole Line", "<Control>Up <Control>Down", ""},
		{"Remove Current Line", "<Control><Shift>Delete", "Ctrl+click on the score to add a line where one is missing"},
		{"Show / Hide Guide Lines", "<Control>g", ""},
	}},
	{"Files", []shortcutItem{
		{"Open", "<Control>o", ""},
		{"Export PDF With Chords", "<Control>e", ""},
		{"Export As", "<Control><Shift>e", ""},
		{"Previous / Next PDF in Folder", "<Alt>Page_Up <Alt>Page_Down", ""},
		{"Quit", "<Control>q", ""},
	}},
	{"View", []shortcutItem{
		{"Zoom In / Out", "<Control>plus <Control>minus", "Or Ctrl+scroll"},
		{"Fit Page Width", "<Control>0", ""},
		{"Toggle Sidebar", "F9", ""},
		{"Keyboard Shortcuts", "F1 <Control>question", ""},
	}},
}

func (w *Window) showShortcuts() {
	d := adw.NewShortcutsDialog()
	for _, s := range shortcutSections {
		sec := adw.NewShortcutsSection(s.title)
		for _, it := range s.items {
			item := adw.NewShortcutsItem(it.title, it.accel)
			if it.subtitle != "" {
				item.SetSubtitle(it.subtitle)
			}
			sec.Add(item)
		}
		d.Add(sec)
	}
	d.ConnectClosed(w.refocus)
	d.Present(w.win)
}
