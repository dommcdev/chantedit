package ui

import (
	"fmt"
	"html"

	"github.com/diamondburned/gotk4-adwaita/pkg/adw"
	"github.com/diamondburned/gotk4/pkg/gtk/v4"
	"github.com/diamondburned/gotk4/pkg/pango"

	"github.com/dominic/chantedit/internal/layout"
)

type sidebar struct {
	entry   *adw.EntryRow
	chordGp *adw.PreferencesGroup
	textGp  *adw.PreferencesGroup
	lineGp  *adw.PreferencesGroup
	font    *gtk.FontDialogButton
	size    *adw.SpinRow
	avoid   *adw.SwitchRow
	snap    *adw.SwitchRow
	offset  *adw.SpinRow
	guides  *adw.SwitchRow
	keys    *adw.ExpanderRow
	syncing bool
}

var shortcuts = [][2]string{
	{"Enter", "Add typed chord(s); several at once with spaces"},
	{"Click", "Put the cursor on the nearest chord line"},
	{"Drag", "Move a chord (also to another line)"},
	{"← →", "Nudge chord / cursor"},
	{"Shift ← →", "Nudge in bigger steps"},
	{"Ctrl ← →", "Jump to previous / next note"},
	{"↑ ↓", "Move to previous / next chord line"},
	{"Shift ↑ ↓", "Raise / lower only this chord"},
	{"Ctrl ↑ ↓", "Raise / lower the whole line"},
	{"Ctrl R", "Reset chord to automatic height"},
	{"Tab  Shift Tab", "Select next / previous chord"},
	{"Home  End", "Select first / last chord"},
	{"F2  Enter  Double-click", "Edit selected chord"},
	{"Backspace  Delete", "Delete selected chord"},
	{"Esc", "Cancel edit / deselect"},
	{"Ctrl Z  Ctrl Shift Z", "Undo / redo"},
	{"Ctrl Click", "Add a chord line where detection missed one"},
	{"Ctrl E", "Export PDF with chords"},
	{"Alt PgUp  Alt PgDn", "Previous / next PDF in folder"},
	{"Ctrl scroll  Ctrl + −  Ctrl 0", "Zoom / fit width"},
	{"Ctrl G", "Show / hide guide lines"},
}

func (w *Window) buildSidebar() gtk.Widgetter {
	sb := &w.sb
	page := adw.NewPreferencesPage()

	// Chord entry
	sb.chordGp = adw.NewPreferencesGroup()
	sb.chordGp.SetTitle("Chord")
	sb.entry = adw.NewEntryRow()
	sb.entry.SetTitle("Type a chord, press Enter")
	sb.entry.ConnectEntryActivated(w.commitEntry)
	sb.entry.ConnectChanged(func() { w.queueDrawAll() })
	focus := gtk.NewEventControllerFocus()
	focus.ConnectEnter(func() { w.entryFocused = true })
	focus.ConnectLeave(func() { w.entryFocused = false })
	sb.entry.AddController(focus)
	sb.chordGp.Add(sb.entry)
	page.Add(sb.chordGp)

	// Text appearance
	sb.textGp = adw.NewPreferencesGroup()
	sb.textGp.SetTitle("Chord Text")
	fontRow := adw.NewActionRow()
	fontRow.SetTitle("Font")
	sb.font = gtk.NewFontDialogButton(gtk.NewFontDialog())
	sb.font.SetLevel(gtk.FontLevelFace)
	sb.font.SetUseSize(false)
	sb.font.SetVAlign(gtk.AlignCenter)
	fontRow.AddSuffix(sb.font)
	sb.textGp.Add(fontRow)

	sb.size = adw.NewSpinRowWithRange(3, 40, 0.5)
	sb.size.SetTitle("Size")
	sb.size.SetSubtitle("Points")
	sb.size.SetDigits(1)
	sb.textGp.Add(sb.size)

	sb.avoid = adw.NewSwitchRow()
	sb.avoid.SetTitle("Avoid Notes Automatically")
	sb.avoid.SetSubtitle("Shift a chord up or down if it would touch the music")
	sb.textGp.Add(sb.avoid)

	sb.snap = adw.NewSwitchRow()
	sb.snap.SetTitle("Snap to Notes")
	sb.snap.SetSubtitle("Clicks and new chords land on the nearest note")
	sb.textGp.Add(sb.snap)
	page.Add(sb.textGp)

	// Lines
	sb.lineGp = adw.NewPreferencesGroup()
	sb.lineGp.SetTitle("Chord Lines")
	sb.guides = adw.NewSwitchRow()
	sb.guides.SetTitle("Show Guide Lines")
	sb.guides.SetActive(w.prefs.Guides)
	sb.lineGp.Add(sb.guides)
	sb.offset = adw.NewSpinRowWithRange(-40, 40, 0.5)
	sb.offset.SetTitle("Move All Lines")
	sb.offset.SetSubtitle("Points, negative is up")
	sb.offset.SetDigits(1)
	sb.lineGp.Add(sb.offset)
	reset := adw.NewButtonRow()
	reset.SetTitle("Reset Line Adjustments")
	reset.ConnectActivated(w.resetLines)
	sb.lineGp.Add(reset)
	page.Add(sb.lineGp)

	// Keyboard help
	kg := adw.NewPreferencesGroup()
	sb.keys = adw.NewExpanderRow()
	sb.keys.SetTitle("Keyboard Shortcuts")
	sb.keys.SetSubtitle("F1")
	for _, s := range shortcuts {
		r := adw.NewActionRow()
		r.SetTitle(html.EscapeString(s[1]))
		k := gtk.NewLabel(s[0])
		k.AddCSSClass("dim-label")
		k.AddCSSClass("shortcut-key")
		r.AddSuffix(k)
		sb.keys.AddRow(r)
	}
	kg.Add(sb.keys)
	page.Add(kg)

	// Handlers
	settingsChanged := func(fontChanged bool) {
		if sb.syncing || w.data == nil {
			return
		}
		s := &w.data.Settings
		if fontChanged {
			w.font = layout.NewFont(s.Font, s.Size)
		}
		w.prefs.Defaults = *s
		w.prefs.Save()
		w.changed()
	}
	sb.font.NotifyProperty("font-desc", func() {
		if sb.syncing || w.data == nil {
			return
		}
		if d := sb.font.FontDesc(); d != nil {
			d.UnsetFields(pango.FontMaskSize)
			w.data.Settings.Font = d.String()
		}
		settingsChanged(true)
		w.refocus()
	})
	sb.size.NotifyProperty("value", func() {
		if sb.syncing || w.data == nil {
			return
		}
		w.data.Settings.Size = sb.size.Value()
		settingsChanged(true)
	})
	sb.avoid.NotifyProperty("active", func() {
		if sb.syncing || w.data == nil {
			return
		}
		w.data.Settings.AutoAvoid = sb.avoid.Active()
		settingsChanged(false)
		w.refocus()
	})
	sb.snap.NotifyProperty("active", func() {
		if sb.syncing || w.data == nil {
			return
		}
		w.data.Settings.SnapNotes = sb.snap.Active()
		settingsChanged(false)
		w.refocus()
	})
	sb.offset.NotifyProperty("value", func() {
		if sb.syncing || w.data == nil {
			return
		}
		w.data.Settings.LineOffset = sb.offset.Value()
		settingsChanged(false)
	})
	sb.guides.NotifyProperty("active", func() {
		w.prefs.Guides = sb.guides.Active()
		w.queueDrawAll()
	})

	return page
}

func (sb *sidebar) docGroups(on bool) {
	sb.chordGp.SetSensitive(on)
	sb.textGp.SetSensitive(on)
	sb.lineGp.SetSensitive(on)
}

// syncSidebar loads the document settings into the widgets.
func (w *Window) syncSidebar() {
	sb := &w.sb
	sb.syncing = true
	defer func() { sb.syncing = false }()
	s := w.data.Settings
	sb.font.SetFontDesc(pango.FontDescriptionFromString(s.Font))
	sb.size.SetValue(s.Size)
	sb.avoid.SetActive(s.AutoAvoid)
	sb.snap.SetActive(s.SnapNotes)
	sb.offset.SetValue(s.LineOffset)
}

func (w *Window) updateStatus() {
	sb := &w.sb
	if w.data == nil {
		sb.chordGp.SetDescription("")
		return
	}
	lineNo := func(page int, id string) string {
		for i, ln := range w.lines[page] {
			if ln.ID == id {
				return fmt.Sprintf("page %d, line %d", page+1, i+1)
			}
		}
		return fmt.Sprintf("page %d", page+1)
	}
	var msg string
	c := w.data.Chord(w.sel)
	switch {
	case w.editing && c != nil:
		msg = fmt.Sprintf("Editing “%s”: Enter to apply, Esc to cancel", c.Text)
	case c != nil:
		extra := ""
		if c.DY != nil {
			extra = " · height tweaked"
		}
		msg = fmt.Sprintf("“%s” selected · %s%s\nNext chord goes after it. Arrows move it.", c.Text, lineNo(c.Page, c.Line), extra)
	case w.cur.ok:
		msg = fmt.Sprintf("Cursor on %s. Type a chord and press Enter.", lineNo(w.cur.page, w.cur.line))
	default:
		msg = "Click on the score where the first chord goes."
	}
	sb.chordGp.SetDescription(html.EscapeString(msg))
}
