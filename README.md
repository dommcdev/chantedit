# ChantEdit

Add guitar chord names above chant (or other) scores, fast and mostly from the
keyboard. Chord lines are detected automatically for every staff, so you only
choose *which note* a chord goes over, never its height.

Rust + GTK 4 / libadwaita. Poppler renders the score; libqpdf stamps chords onto
the original pages so the music stays vector.

## Build & run

```sh
make            # first build compiles gtk-rs and takes a minute or two
./bin/chantedit "~/Downloads/Chant/Te Deum (Simple tone).pdf"
make install    # optional: ~/.local/bin/chantedit + desktop entry
```

On Arch: `pacman -S rust gtk4 libadwaita poppler-glib qpdf`.

The first compile is slow because of the gtk-rs crates. Later rebuilds take a
few seconds.

## Documents

Work is saved as a `.ce` file, not in `~/.local`. A `.ce` is a zip archive:

```
mimetype      application/x-chantedit
chords.json   settings, chords and line edits
score.pdf     the original PDF, byte for byte
```

The score travels inside the file, so a `.ce` still opens after the PDF is
moved, renamed, or gone — useful if you wipe machines and keep a folder of
pieces.

- **Open a PDF:** starts a new document. Ctrl+S writes `Name.ce` next to it.
- **Open a `.ce`:** keeps editing that document.
- **Open a PDF that already has a `.ce` next to it:** opens the saved document.

The original PDF is never modified. Export writes `Name (chords).pdf`.

App preferences (last folder, zoom, guide lines, default font/size) still live
in `~/.config/chantedit/prefs.json`.

## Workflow

1. Open a PDF (Ctrl+O). The cursor is already on the first line, first note.
2. Type a chord in the sidebar box and press Enter. The next chord you enter
   goes after the selected one, on the next note. Type several at once
   separated by spaces (`C F Dm`) to place them on consecutive notes.
3. With the box empty, the arrows adjust the selected chord:
   - ← → nudge (Shift for bigger steps), Ctrl+← → jump note to note
   - ↑ ↓ move to the previous/next chord line
   - Shift+↑ ↓ raise/lower just this chord, Ctrl+R to reset it
   - Ctrl+↑ ↓ raise/lower the whole line
4. Click anywhere to put the cursor on the nearest line. Drag chords with the
   mouse. Tab / Shift+Tab walk through chords, Backspace deletes.
5. Ctrl+S saves the `.ce`. Ctrl+E exports a PDF with the chords drawn on.
   Alt+Page Down opens the next piece in the folder.

F1 opens the full list of keyboard shortcuts.

## How line detection works

Analysis is on a 150 dpi render, so it does not matter whether the PDF is
vector, text, or a scan:

1. **Staff lines** are rows containing long horizontal dark runs; lines with
   regular spacing are grouped into staves (3–6 lines, so 4-line chant and
   5-line modern staves both work).
2. **Text above each staff** (the previous system's lyrics, translations,
   titles) is found by rows that are cut into many thin, closely spaced
   strokes, which letters produce and note heads/stems don't.
3. The **chord line** is centred between the bottom of that text and the top
   staff line, capped at a comfortable distance from the staff when the gap
   is large. Lines where the chosen size doesn't really fit are drawn orange:
   lower the text size for those scores.
4. **Note columns** are found after erasing staff lines, which gives the snap
   targets for clicks, new chords and Ctrl+← →.
5. Each chord is then checked against the actual ink under it; if it would
   touch a high note or a descender it is shifted up (or down) just enough,
   within the free band ("Avoid Notes Automatically").

If detection misses a line, Ctrl+click to add one; right-click a line to
remove it or reset its position.

To inspect detection on a new kind of score:

```sh
make detect PDF="some score.pdf"   # overlays in /tmp/chantdetect
```

## Layout

```
src/analysis.rs     staff / chord-line / note detection
src/layout.rs       lines + chord placement + text drawing
src/document.rs     the .ce format
src/pdf.rs          Poppler rendering
src/export.rs       cairo overlay + libqpdf stamp
src/ui/             libadwaita window, page view, keyboard handling
src/bin/chantdetect.rs   debug tool that draws detection results to PNGs
```

`CHANTEDIT_SCRIPT` drives the UI for testing (see `src/ui/window.rs`).
