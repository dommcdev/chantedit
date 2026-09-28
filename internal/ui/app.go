package ui

import (
	"github.com/diamondburned/gotk4-adwaita/pkg/adw"
	"github.com/diamondburned/gotk4/pkg/gio/v2"
)

const appID = "dev.dominic.ChantEdit"

func Run(args []string) int {
	app := adw.NewApplication(appID, gio.ApplicationHandlesOpen)
	var win *Window
	ensure := func() *Window {
		if win == nil {
			win = newWindow(app)
		}
		win.win.Present()
		return win
	}
	app.ConnectActivate(func() { ensure() })
	app.ConnectOpen(func(files []gio.Filer, hint string) {
		w := ensure()
		if len(files) > 0 {
			w.openPath(files[0].Path())
		}
	})

	accels := map[string][]string{
		"win.open":           {"<Control>o"},
		"win.export":         {"<Control>e"},
		"win.export-as":      {"<Control><Shift>e"},
		"win.next-file":      {"<Alt>Page_Down", "<Control>bracketright"},
		"win.prev-file":      {"<Alt>Page_Up", "<Control>bracketleft"},
		"win.zoom-in":        {"<Control>plus", "<Control>equal", "<Control>KP_Add"},
		"win.zoom-out":       {"<Control>minus", "<Control>KP_Subtract"},
		"win.zoom-fit":       {"<Control>0"},
		"win.toggle-guides":  {"<Control>g"},
		"win.toggle-sidebar": {"F9"},
		"win.remove-line":    {"<Control><Shift>Delete"},
		"win.shortcuts":      {"F1", "<Control>question"},
		"win.quit":           {"<Control>q"},
	}
	for action, keys := range accels {
		app.SetAccelsForAction(action, keys)
	}
	return app.Run(args)
}
