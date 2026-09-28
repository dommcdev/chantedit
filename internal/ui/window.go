package ui

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
	"unicode"

	"github.com/diamondburned/gotk4-adwaita/pkg/adw"
	"github.com/diamondburned/gotk4/pkg/core/glib"
	"github.com/diamondburned/gotk4/pkg/gdk/v4"
	"github.com/diamondburned/gotk4/pkg/gio/v2"
	glibv2 "github.com/diamondburned/gotk4/pkg/glib/v2"
	"github.com/diamondburned/gotk4/pkg/gtk/v4"

	"github.com/dominic/chantedit/internal/analysis"
	"github.com/dominic/chantedit/internal/doc"
	"github.com/dominic/chantedit/internal/export"
	"github.com/dominic/chantedit/internal/layout"
	"github.com/dominic/chantedit/internal/poppler"
)

const (
	pageMargin  = 24
	pageSpacing = 24
	exportTag   = " (chords)"
)

type cursor struct {
	ok   bool
	page int
	line string
	x    float64
}

type snapshot struct {
	edits doc.Edits
	sel   int
}

type Window struct {
	app   *adw.Application
	win   *adw.ApplicationWindow
	prefs *doc.Prefs

	toasts   *adw.ToastOverlay
	split    *adw.OverlaySplitView
	title    *adw.WindowTitle
	stack    *gtk.Stack
	scroller *gtk.ScrolledWindow
	pagesBox *gtk.Box
	popover  *gtk.Popover
	pages    []*pageView

	sb sidebar

	// document
	path     string
	pdf      *poppler.Document
	data     *doc.Data
	analyses []*analysis.Page
	lines    [][]layout.Line
	font     *layout.Font
	gen      int
	zoom     float64

	// editing state
	sel          int // selected chord id, 0 = none
	editing      bool
	cur          cursor
	undo, redo   []snapshot
	lastOp       string
	lastOpTime   time.Time
	saveTimer    glib.SourceHandle
	entryFocused bool
}

func newWindow(app *adw.Application) *Window {
	w := &Window{app: app, prefs: doc.LoadPrefs()}
	w.zoom = w.prefs.Zoom
	w.win = adw.NewApplicationWindow(&app.Application)
	w.win.SetDefaultSize(1300, 950)
	w.win.SetTitle("ChantEdit")

	// Header bar
	header := adw.NewHeaderBar()
	w.title = adw.NewWindowTitle("ChantEdit", "")
	header.SetTitleWidget(w.title)

	openBtn := gtk.NewButtonFromIconName("document-open-symbolic")
	openBtn.SetTooltipText("Open PDF (Ctrl+O)")
	openBtn.SetActionName("win.open")
	header.PackStart(openBtn)

	nav := gtk.NewBox(gtk.OrientationHorizontal, 0)
	nav.AddCSSClass("linked")
	prev := gtk.NewButtonFromIconName("go-previous-symbolic")
	prev.SetTooltipText("Previous PDF in folder (Alt+Page Up)")
	prev.SetActionName("win.prev-file")
	next := gtk.NewButtonFromIconName("go-next-symbolic")
	next.SetTooltipText("Next PDF in folder (Alt+Page Down)")
	next.SetActionName("win.next-file")
	nav.Append(prev)
	nav.Append(next)
	header.PackStart(nav)

	sideBtn := gtk.NewToggleButton()
	sideBtn.SetIconName("sidebar-show-symbolic")
	sideBtn.SetTooltipText("Toggle sidebar (F9)")
	sideBtn.SetActive(true)

	menu := gio.NewMenu()
	sec1 := gio.NewMenu()
	sec1.Append("Export As…", "win.export-as")
	menu.AppendSection("", sec1)
	sec2 := gio.NewMenu()
	sec2.Append("Zoom In", "win.zoom-in")
	sec2.Append("Zoom Out", "win.zoom-out")
	sec2.Append("Fit Width", "win.zoom-fit")
	sec2.Append("Show Guide Lines", "win.toggle-guides")
	menu.AppendSection("", sec2)
	sec3 := gio.NewMenu()
	sec3.Append("Remove Current Chord Line", "win.remove-line")
	sec3.Append("Reset All Line Adjustments", "win.reset-lines")
	menu.AppendSection("", sec3)
	sec4 := gio.NewMenu()
	sec4.Append("Keyboard Shortcuts", "win.shortcuts")
	menu.AppendSection("", sec4)
	menuBtn := gtk.NewMenuButton()
	menuBtn.SetIconName("open-menu-symbolic")
	menuBtn.SetMenuModel(menu)
	menuBtn.SetTooltipText("Menu")

	exportBtn := gtk.NewButtonWithLabel("Export")
	exportBtn.AddCSSClass("suggested-action")
	exportBtn.SetTooltipText("Export PDF with chords (Ctrl+E)")
	exportBtn.SetActionName("win.export")
	header.PackEnd(menuBtn)
	header.PackEnd(exportBtn)
	header.PackEnd(sideBtn)

	// Content: page view
	w.pagesBox = gtk.NewBox(gtk.OrientationVertical, pageSpacing)
	w.pagesBox.SetMarginTop(pageMargin)
	w.pagesBox.SetMarginBottom(pageMargin)
	w.pagesBox.SetMarginStart(pageMargin)
	w.pagesBox.SetMarginEnd(pageMargin)
	w.pagesBox.SetHAlign(gtk.AlignCenter)
	w.scroller = gtk.NewScrolledWindow()
	w.scroller.SetChild(w.pagesBox)
	w.scroller.SetVExpand(true)
	w.scroller.SetHExpand(true)
	w.scroller.AddCSSClass("chant-canvas")

	scroll := gtk.NewEventControllerScroll(gtk.EventControllerScrollVertical)
	scroll.SetPropagationPhase(gtk.PhaseCapture)
	scroll.ConnectScroll(func(dx, dy float64) bool {
		if scroll.CurrentEventState()&gdk.ControlMask == 0 || w.data == nil {
			return false
		}
		if dy < 0 {
			w.setZoom(w.zoom * 1.1)
		} else if dy > 0 {
			w.setZoom(w.zoom / 1.1)
		}
		return true
	})
	w.scroller.AddController(scroll)

	w.popover = gtk.NewPopover()
	w.popover.SetParent(w.scroller)
	w.popover.SetHasArrow(true)
	w.popover.ConnectClosed(func() { w.refocus() })

	empty := adw.NewStatusPage()
	empty.SetIconName("folder-music-symbolic")
	empty.SetTitle("Open a Chant PDF")
	empty.SetDescription("Chord lines are detected automatically. Your chords are saved as you go.")
	emptyBtn := gtk.NewButtonWithLabel("Open PDF…")
	emptyBtn.AddCSSClass("pill")
	emptyBtn.AddCSSClass("suggested-action")
	emptyBtn.SetHAlign(gtk.AlignCenter)
	emptyBtn.SetActionName("win.open")
	empty.SetChild(emptyBtn)

	w.stack = gtk.NewStack()
	w.stack.AddNamed(empty, "empty")
	w.stack.AddNamed(w.scroller, "doc")
	w.stack.SetVisibleChildName("empty")

	w.split = adw.NewOverlaySplitView()
	w.split.SetSidebar(w.buildSidebar())
	w.split.SetContent(w.stack)
	w.split.SetMinSidebarWidth(310)
	w.split.SetMaxSidebarWidth(360)
	sideBtn.ConnectToggled(func() { w.split.SetShowSidebar(sideBtn.Active()) })
	w.split.NotifyProperty("show-sidebar", func() { sideBtn.SetActive(w.split.ShowSidebar()) })

	w.toasts = adw.NewToastOverlay()
	w.toasts.SetChild(w.split)

	tv := adw.NewToolbarView()
	tv.AddTopBar(header)
	tv.SetContent(w.toasts)
	w.win.SetContent(tv)

	css := gtk.NewCSSProvider()
	css.LoadFromString(`.chant-canvas { background-color: alpha(@window_fg_color, 0.08); }
.chant-page { box-shadow: 0 1px 4px alpha(black, 0.35); }
.shortcut-key { font-family: monospace; font-size: 0.9em; }`)
	gtk.StyleContextAddProviderForDisplay(gdk.DisplayGetDefault(), css, gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

	key := gtk.NewEventControllerKey()
	key.SetPropagationPhase(gtk.PhaseCapture)
	key.ConnectKeyPressed(func(keyval, keycode uint, state gdk.ModifierType) bool {
		return w.onKey(keyval, state)
	})
	w.win.AddController(key)

	w.win.ConnectCloseRequest(func() bool {
		w.saveNow()
		w.prefs.Save()
		return false
	})

	w.addActions()
	w.setDocSensitive(false)
	w.runScript()
	return w
}

func (w *Window) addActions() {
	add := func(name string, f func()) {
		a := gio.NewSimpleAction(name, nil)
		a.ConnectActivate(func(*glibv2.Variant) { f() })
		w.win.AddAction(a)
	}
	add("open", w.showOpenDialog)
	add("export", func() { w.exportTo("") })
	add("export-as", w.showExportDialog)
	add("next-file", func() { w.siblingFile(1) })
	add("prev-file", func() { w.siblingFile(-1) })
	add("zoom-in", func() { w.setZoom(w.zoom * 1.15) })
	add("zoom-out", func() { w.setZoom(w.zoom / 1.15) })
	add("zoom-fit", func() { w.setZoom(w.fitZoom()) })
	add("toggle-guides", func() { w.sb.guides.SetActive(!w.sb.guides.Active()) })
	add("toggle-sidebar", func() { w.split.SetShowSidebar(!w.split.ShowSidebar()) })
	add("remove-line", w.removeCurrentLine)
	add("reset-lines", w.resetLines)
	add("shortcuts", func() {
		w.split.SetShowSidebar(true)
		w.sb.keys.SetExpanded(!w.sb.keys.Expanded())
	})
	add("quit", func() { w.win.Close() })
}

func (w *Window) setDocSensitive(on bool) {
	for _, a := range []string{"export", "export-as", "next-file", "prev-file", "zoom-in",
		"zoom-out", "zoom-fit", "remove-line", "reset-lines"} {
		if act := w.win.LookupAction(a); act != nil {
			act.Cast().(*gio.SimpleAction).SetEnabled(on)
		}
	}
	w.sb.docGroups(on)
}

func (w *Window) toast(msg string) {
	t := adw.NewToast(msg)
	t.SetTimeout(3)
	w.toasts.AddToast(t)
}

// ------------------------------------------------------------------ opening

func (w *Window) showOpenDialog() {
	d := gtk.NewFileDialog()
	d.SetTitle("Open Chant PDF")
	filters := gio.NewListStore(gtk.GTypeFileFilter)
	f := gtk.NewFileFilter()
	f.SetName("PDF documents")
	f.AddMIMEType("application/pdf")
	f.AddSuffix("pdf")
	filters.Append(f.Object)
	d.SetFilters(filters)
	if dir := w.currentDir(); dir != "" {
		d.SetInitialFolder(gio.NewFileForPath(dir))
	}
	d.Open(context.Background(), &w.win.Window, func(res gio.AsyncResulter) {
		file, err := d.OpenFinish(res)
		if err != nil || file == nil {
			return
		}
		w.openPath(file.Path())
	})
}

func (w *Window) currentDir() string {
	if w.path != "" {
		return filepath.Dir(w.path)
	}
	return w.prefs.LastDir
}

func (w *Window) openPath(path string) {
	if path == "" {
		return
	}
	pdf, err := poppler.Open(path)
	if err != nil {
		w.toast("Could not open PDF: " + err.Error())
		return
	}
	data, err := doc.Load(path, w.prefs.Defaults)
	if err != nil {
		w.toast("Could not load saved chords: " + err.Error())
		return
	}
	w.saveNow()
	if w.pdf != nil {
		w.pdf.Close()
	}
	w.path, w.pdf, w.data = path, pdf, data
	w.gen++
	n := pdf.NPages()
	w.analyses = make([]*analysis.Page, n)
	w.lines = make([][]layout.Line, n)
	w.sel, w.editing, w.cur = 0, false, cursor{}
	w.undo, w.redo = nil, nil
	w.font = layout.NewFont(data.Settings.Font, data.Settings.Size)
	w.prefs.LastDir = filepath.Dir(path)

	w.syncSidebar()
	w.buildPages()
	w.stack.SetVisibleChildName("doc")
	w.setDocSensitive(true)
	w.updateTitle()
	w.updateStatus()
	w.sb.entry.SetText("")
	w.scroller.VAdjustment().SetValue(0)
	glib.IdleAdd(func() {
		if w.zoom <= 0 {
			w.setZoom(w.fitZoom())
		}
		w.refocus()
	})

	gen := w.gen
	go func() {
		d, err := poppler.Open(path)
		if err != nil {
			return
		}
		defer d.Close()
		for i := 0; i < n; i++ {
			pw, ph := d.PageSize(i)
			gray, gw, gh, _ := d.RenderGray(i, analysis.DPI)
			a := analysis.Analyze(gray, gw, gh, pw, ph)
			i := i
			glib.IdleAdd(func() {
				if w.gen != gen {
					return
				}
				w.analyses[i] = a
				w.recomputeLines()
				if !w.cur.ok && w.sel == 0 {
					if len(w.data.Chords) == 0 {
						w.cursorToStart()
					} else if i == n-1 {
						// Continue where the last session left off.
						cs := w.sortedChords()
						w.selectChord(cs[len(cs)-1].ID, false)
					}
				}
				w.updateTitle()
				w.updateStatus()
				w.pages[i].area.QueueDraw()
			})
		}
	}()
}

func (w *Window) updateTitle() {
	if w.data == nil {
		w.title.SetTitle("ChantEdit")
		w.title.SetSubtitle("")
		return
	}
	name := strings.TrimSuffix(filepath.Base(w.path), filepath.Ext(w.path))
	w.win.SetTitle(name + " – ChantEdit")
	w.title.SetTitle(name)
	nl := 0
	pending := 0
	for i, ls := range w.lines {
		nl += len(ls)
		if w.analyses[i] == nil {
			pending++
		}
	}
	sub := fmt.Sprintf("%d pages · %d chord lines · %d chords", len(w.lines), nl, len(w.data.Chords))
	if pending > 0 {
		sub = fmt.Sprintf("%d pages · analysing…", len(w.lines))
	}
	w.title.SetSubtitle(sub)
}

func (w *Window) siblingFile(dir int) {
	if w.path == "" {
		return
	}
	files := pdfsIn(filepath.Dir(w.path))
	idx := -1
	for i, f := range files {
		if f == w.path {
			idx = i
		}
	}
	j := idx + dir
	if idx < 0 || j < 0 || j >= len(files) {
		if dir > 0 {
			w.toast("This is the last PDF in the folder")
		} else {
			w.toast("This is the first PDF in the folder")
		}
		return
	}
	w.openPath(files[j])
}

func pdfsIn(dir string) []string {
	ents, err := os.ReadDir(dir)
	if err != nil {
		return nil
	}
	var out []string
	for _, e := range ents {
		name := e.Name()
		if e.IsDir() || !strings.EqualFold(filepath.Ext(name), ".pdf") {
			continue
		}
		if strings.HasSuffix(strings.TrimSuffix(name, filepath.Ext(name)), exportTag) {
			continue
		}
		out = append(out, filepath.Join(dir, name))
	}
	sort.Slice(out, func(i, j int) bool { return naturalLess(out[i], out[j]) })
	return out
}

// naturalLess compares strings case-insensitively with numbers by value
// ("Hymn 2" < "Hymn 10").
func naturalLess(a, b string) bool {
	ra, rb := []rune(strings.ToLower(a)), []rune(strings.ToLower(b))
	i, j := 0, 0
	for i < len(ra) && j < len(rb) {
		if unicode.IsDigit(ra[i]) && unicode.IsDigit(rb[j]) {
			si := i
			for i < len(ra) && unicode.IsDigit(ra[i]) {
				i++
			}
			sj := j
			for j < len(rb) && unicode.IsDigit(rb[j]) {
				j++
			}
			na := strings.TrimLeft(string(ra[si:i]), "0")
			nb := strings.TrimLeft(string(rb[sj:j]), "0")
			if len(na) != len(nb) {
				return len(na) < len(nb)
			}
			if na != nb {
				return na < nb
			}
			continue
		}
		if ra[i] != rb[j] {
			return ra[i] < rb[j]
		}
		i++
		j++
	}
	return len(ra)-i < len(rb)-j
}

// ------------------------------------------------------------------- export

func (w *Window) defaultExportPath() (string, bool) {
	dir := filepath.Dir(w.path)
	stem := strings.TrimSuffix(filepath.Base(w.path), filepath.Ext(w.path))
	name := stem + exportTag + ".pdf"
	if f, err := os.CreateTemp(dir, ".chantedit-*"); err == nil {
		f.Close()
		os.Remove(f.Name())
		return filepath.Join(dir, name), false
	}
	out := w.prefs.ExportDir
	if out == "" {
		home, _ := os.UserHomeDir()
		out = filepath.Join(home, "Documents", "Chant with chords")
	}
	return filepath.Join(out, name), true
}

func (w *Window) showExportDialog() {
	if w.data == nil {
		return
	}
	def, _ := w.defaultExportPath()
	d := gtk.NewFileDialog()
	d.SetTitle("Export PDF with Chords")
	d.SetInitialName(filepath.Base(def))
	d.SetInitialFolder(gio.NewFileForPath(filepath.Dir(def)))
	d.Save(context.Background(), &w.win.Window, func(res gio.AsyncResulter) {
		file, err := d.SaveFinish(res)
		if err != nil || file == nil {
			return
		}
		w.exportTo(file.Path())
	})
}

func (w *Window) exportTo(out string) {
	if w.data == nil {
		return
	}
	for _, a := range w.analyses {
		if a == nil {
			w.toast("Still analysing the pages, try again in a moment")
			return
		}
	}
	fallback := false
	if out == "" {
		out, fallback = w.defaultExportPath()
	}
	if abs, _ := filepath.Abs(out); abs == w.path {
		w.toast("Refusing to overwrite the original PDF")
		return
	}
	pages := make([]export.Page, len(w.pages))
	for i, p := range w.pages {
		pages[i] = export.Page{W: p.pw, H: p.ph}
	}
	for i := range w.data.Chords {
		c := &w.data.Chords[i]
		pl, _, ok := w.place(c)
		if !ok || c.Page >= len(pages) {
			continue
		}
		pages[c.Page].Items = append(pages[c.Page].Items, export.Item{Text: c.Text, CX: c.X, Baseline: pl.Baseline})
	}
	if err := export.Export(w.path, out, pages, w.font); err != nil {
		w.toast("Export failed: " + err.Error())
		return
	}
	if fallback {
		w.prefs.ExportDir = filepath.Dir(out)
	}
	w.saveNow()
	t := adw.NewToast(fmt.Sprintf("Exported “%s”", filepath.Base(out)))
	if fallback {
		t.SetTitle(fmt.Sprintf("Folder is read-only, exported to %s", out))
	}
	t.SetTimeout(5)
	t.SetButtonLabel("Open")
	t.ConnectButtonClicked(func() {
		gtk.NewFileLauncher(gio.NewFileForPath(out)).Launch(context.Background(), &w.win.Window, nil)
	})
	w.toasts.AddToast(t)
}

// ------------------------------------------------------------------- saving

func (w *Window) scheduleSave() {
	if w.saveTimer != 0 {
		glib.SourceRemove(w.saveTimer)
	}
	w.saveTimer = glib.TimeoutAdd(400, func() bool {
		w.saveTimer = 0
		w.saveNow()
		return false
	})
}

func (w *Window) saveNow() {
	if w.saveTimer != 0 {
		glib.SourceRemove(w.saveTimer)
		w.saveTimer = 0
	}
	if w.data == nil {
		return
	}
	if err := w.data.Save(); err != nil {
		w.toast("Could not save chords: " + err.Error())
	}
}
