package ui

// Scripted driving of the editor, used to test the UI headlessly-ish:
//
//	CHANTEDIT_SCRIPT='type:Dm;enter;key:Right;key:Right+ctrl;snap:/tmp/a.png;quit'
//
// Commands (separated by ';'):
//
//	type:TEXT            set the chord entry text
//	enter                press Enter in the chord entry
//	key:NAME[+mod...]    send a key (GDK key name; mods: ctrl, shift, alt)
//	click:PAGE,X,Y       click at page coordinates (pt); add +ctrl for Ctrl+click
//	drag:PAGE,X,Y,DX,DY  drag from a point by an offset (pt)
//	size:PT              set chord text size
//	snap:PATH            save a PNG of the window
//	export:PATH          export the PDF
//	action:NAME          activate a window action (e.g. next-file)
//	quit                 close the window

import (
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/diamondburned/gotk4/pkg/core/glib"
	"github.com/diamondburned/gotk4/pkg/gdk/v4"
)

func (w *Window) runScript() {
	script := os.Getenv("CHANTEDIT_SCRIPT")
	if script == "" {
		return
	}
	cmds := strings.Split(script, ";")
	var step func() bool
	i, retries := 0, 0
	step = func() bool {
		if w.data == nil || !w.win.Mapped() || w.scroller.Width() == 0 {
			return true
		}
		for _, a := range w.analyses {
			if a == nil {
				return true
			}
		}
		if i >= len(cmds) {
			return false
		}
		cmd := strings.TrimSpace(cmds[i])
		err := w.runCommand(cmd)
		// The window may not have painted a frame yet.
		if err != nil && strings.HasPrefix(cmd, "snap:") && retries < 20 {
			retries++
			return true
		}
		i, retries = i+1, 0
		if err != nil {
			fmt.Fprintf(os.Stderr, "script: %s: %v\n", cmd, err)
		}
		return i < len(cmds)
	}
	glib.TimeoutAdd(250, step)
}

func (w *Window) runCommand(cmd string) error {
	name, arg, _ := strings.Cut(cmd, ":")
	nums := func() []float64 {
		var out []float64
		for _, f := range strings.Split(strings.Split(arg, "+")[0], ",") {
			v, _ := strconv.ParseFloat(strings.TrimSpace(f), 64)
			out = append(out, v)
		}
		return out
	}
	z := w.zoomOr1()
	switch name {
	case "type":
		w.sb.entry.SetText(arg)
	case "enter":
		w.commitEntry()
	case "key":
		parts := strings.Split(arg, "+")
		kv := gdk.KeyvalFromName(parts[0])
		var mods gdk.ModifierType
		for _, m := range parts[1:] {
			switch m {
			case "ctrl":
				mods |= gdk.ControlMask
			case "shift":
				mods |= gdk.ShiftMask
			case "alt":
				mods |= gdk.AltMask
			}
		}
		w.entryFocused = true
		if !w.onKey(kv, mods) {
			return fmt.Errorf("key not handled")
		}
	case "click":
		n := nums()
		var mods gdk.ModifierType
		if strings.HasSuffix(arg, "+ctrl") {
			mods = gdk.ControlMask
		}
		p := w.pages[int(n[0])]
		p.dragBegin(n[1]*z, n[2]*z, mods)
		p.dragEnd()
	case "drag":
		n := nums()
		p := w.pages[int(n[0])]
		p.dragBegin(n[1]*z, n[2]*z, 0)
		p.dragUpdate(n[3]*z*0.5, n[4]*z*0.5)
		p.dragUpdate(n[3]*z, n[4]*z)
		p.dragEnd()
	case "size":
		v, _ := strconv.ParseFloat(arg, 64)
		w.sb.size.SetValue(v)
	case "snap":
		return w.snapshotPNG(arg)
	case "export":
		w.exportTo(arg)
	case "action":
		act := w.win.LookupAction(arg)
		if act == nil {
			return fmt.Errorf("no such action")
		}
		if !act.Enabled() {
			return fmt.Errorf("action disabled")
		}
		act.Activate(nil)
	case "quit":
		w.quit()
	default:
		return fmt.Errorf("unknown command")
	}
	return nil
}

func (w *Window) snapshotPNG(path string) error {
	if d := w.win.VisibleDialog(); d != nil {
		return snapshotWidgetPNG(d, path)
	}
	return snapshotWidgetPNG(w.win.Content(), path)
}
