# ChantEdit

Add guitar chords above chant scores. Open a PDF, type your chords, and export
a copy to print or share. ChantEdit finds room above the music and helps align
chords with the notes.

## Install

ChantEdit is a Linux desktop app. On Arch Linux, install the requirements:

```sh
sudo pacman -S --needed base-devel rust gtk4 libadwaita poppler-glib qpdf
```

Download the source, open a terminal in the project folder, and run:

```sh
make install
```

Then open **ChantEdit** from your app
launcher. It installs into `~/.local`, without needing `sudo`.

## Use

1. **Open a score** with **Ctrl+O**. ChantEdit finds the chord lines automatically.
2. **Add chords** directly on the score. Click a note position and type a chord;
   the text updates as you type. **Tab** finishes it and selects the next note
   or chord; **Shift+Tab** goes backwards. Typing on a selected chord replaces it.
3. **Adjust placement** by clicking the score to choose a starting point, or
   dragging a chord. Arrow keys move the selected chord, **Shift+Left/Right**
   move in bigger steps, and **Ctrl+Left/Right** jump it to a note. **Backspace**
   removes the last typed character, or deletes a chord when you aren't typing.
4. **Save** with **Ctrl+S** to keep editing later, or **export a PDF** with
   **Ctrl+E** to print or share.

Saved work uses a `.ce` file containing both the score and your chords. Keep it
to continue editing, even if the original PDF moves. Opening a PDF with a saved
`.ce` beside it resumes that saved work. Export creates a separate
`Name (chords).pdf`; your original PDF stays untouched.

Use **Preferences** in the main menu (**Ctrl+,**) to change the chord font, size,
and placement options. Guide lines are optional and off by default. If a chord line is missing,
**Ctrl+click** the score to add one. Press **F1** for all keyboard shortcuts.

## Programmatic use (headless)

These commands do not open a window or require a display:

```sh
chantedit --json score.pdf > score.json
chantedit --apply chords.json score.pdf --output annotated.pdf --save annotated.ce
# Read the same request from stdin with --apply -.
```

`--json` returns a version-1 object with `pages` (size and staff bounds) and
`notes` in reading order. A note contains `index`, `page`, `staff`, and `x`.
All indices are zero-based; coordinates are PDF points from the top left.
These are detected **note/neume columns**, not individual pitches: a neume may
contain several sung notes, and detection can miss columns.

The import format is:

```json
{"version":1,"chords":[{"page":0,"staff":0,"x":120.5,"text":"C Dm"}]}
```

Imports add to existing chords when the input is a `.ce` document. They use
the editor's normal layout/ink avoidance and export without rasterizing the
original PDF. On pages with no detected staff, `staff: 0` creates a manual
line near the top. `--output` and `--save` can be used independently. Open the
saved `.ce` to adjust placements; the original score is never overwritten.
Add `--spread` to wrap overly long imports and stagger colliding chord strings
into rows above their anchors. Dense rows may encroach on titles/other score
content and still need manual adjustment; no labels are discarded.
JSON goes to stdout; errors go to stderr with a nonzero exit code.

## Develop locally

The app uses Rust, GTK 4, and libadwaita. With the requirements above installed:

```sh
cargo run -- "path/to/score.pdf"
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

To run the GTK keyboard and close-window regression test in a graphical session:

```sh
cargo test --bin chantedit keyboard_and_close_event_routing -- --ignored --test-threads=1
```

Sample-score tests use `~/Downloads/Chant` (or `CHANTEDIT_SAMPLES`) and skip
missing scores. Run `make` to build release binaries in `bin/`.

## License

ChantEdit is free software, licensed under the
[GNU General Public License, version 3 or later](LICENSE).
