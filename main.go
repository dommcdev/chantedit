// ChantEdit: add guitar chord symbols above chant scores in PDFs.
package main

import (
	"log/slog"
	"os"

	"github.com/dominic/chantedit/internal/ui"
)

func main() {
	// gotk4 forwards every GLib info/debug message (e.g. Vulkan loader chatter).
	slog.SetLogLoggerLevel(slog.LevelWarn)
	os.Exit(ui.Run(os.Args))
}
