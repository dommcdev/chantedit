# ChantEdit

Add guitar chords to Gregorian chant **`.gabc` files**. Click a neume, type a
chord, and save the annotations directly into the original GABC. Your separate
Gregorio/LaTeX pipeline can then typeset the final score.

## Install

ChantEdit is a Linux desktop app using Rust, GTK 4 and libadwaita. On Arch Linux:

```sh
sudo pacman -S --needed base-devel rust gtk4 libadwaita librsvg
make install
```

It installs into `~/.local` without `sudo`. **Previewing requires no TeX,
Docker, Node, browser, network access, or separate font downloads.** The chant
renderer and its musical glyph outlines are embedded in the binary.

## Use

```sh
cargo run -- "path/to/score.gabc"
```

1. **Open** a GABC file with **Ctrl+O**.
2. **Click a neume and type a chord.** Text updates immediately. Typing on a
   selected chord replaces it; **F2** or double-click edits its existing text.
3. **Tab / Shift+Tab** finish typing and select the next / previous neume.
4. **Left / Right** move the selected chord one existing neume boundary.
   **Up / Down** move it to the nearest anchor on the adjacent staff.
5. **Ctrl+arrows** fine-tune the selected chord's horizontal/vertical placement
   in 0.5-unit steps. **Ctrl+Shift+arrows** use larger steps. **Ctrl+R** resets
   manual offsets. Dragging snaps to a neume; **Ctrl+drag** adjusts placement.
6. **Ctrl+S** saves directly to the GABC. **Save As** creates another GABC.

**Backspace** erases typed text, or deletes a selected chord when not typing.
**Delete** removes the selected chord. **Ctrl+Z / Ctrl+Shift+Z** undo / redo.
**Ctrl+0** fits the score width; **Ctrl++ / Ctrl+-** change zoom.
**Ctrl+G** toggles anchor guides; **F1** shows keyboard help.
**Alt+Page Up / Down** browse GABC files in the current folder.

Overlapping chords automatically stagger upwards. Fine positioning moves only
the chord text: **the app never inserts musical spacing or splits a neume to
make room for chords**. Anchors are existing groups, not individual pitches
inside a connected neume. A manual offset can place a chord above another part
of its group.

## File format and preview

The GABC is the only saved document. There are no `.ce` archives or JSON
sidecars. Original headers, lyrics, comments and music are retained verbatim;
an unchanged open/save is byte-for-byte identical. Saves use atomic replacement.

Plain chord annotations such as `[alt:C]` and `<alt>C</alt>` are imported.
Other above-line text is preserved. Edited/new chords use self-contained TeX
inside Gregorio's `[nv:…]` note-level tags to specify a 9bp bold chord font and
placement offsets. Musical
accidentals use TeX's sharp/flat symbols. Automatic staggering and manual
adjustments are encoded separately, so both survive reopening.

The prototypes found that `[alt]` can consume neumatic cuts, split neumes,
and influence bar/clef spacing. To preserve chant geometry strictly, ChantEdit
attaches a **zero-width note-level annotation to the first note of the group**,
then invokes Gregorio's own above-line text drawing facility at the glyph's
left edge. This preserves existing glyphs and all musical spacing; no custom
preamble macro is required. Existing unchanged `[alt]` annotations retain their
original spelling and position.

The **editing preview** uses Exsurge in an embedded QuickJS engine, measures
lyrics with Pango, and draws SVG directly through librsvg. Source anchors map
to renderer note objects, rather than guessed PDF pixels. Editing and zooming
do not rerender the chant layout.

Exsurge and Gregorio have different engraving/line-breaking algorithms. The
preview is for placing chord anchors, not an exact final-PDF proof. Chords
remain attached to the same musical groups in the final pipeline, but a
different font or page width can require reviewing dense chord collisions.
Common square-notation chant is supported, including the sample hymns in
`~/Downloads`. Extended notation unsupported by the preview may be rejected;
NABC and fused-neume notation are not currently supported.

## Headless preview checks

These work without a display and do not modify the input:

```sh
chantedit --check score.gabc
chantedit --preview score.gabc --output preview-directory
```

`--check` verifies parsing, anchor mapping and a lossless round trip.
`--preview` additionally writes one chant SVG per staff for inspecting the
renderer. Editable chords are drawn separately by the GUI.

## Development and verification

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Real-score tests use the two sample hymns in `~/Downloads` (or
`CHANTEDIT_SAMPLES`) and skip them if absent.

In a graphical session, test typing, offsets, save/reopen, and window closing:

```sh
cargo test --bin chantedit keyboard_save_reopen_and_close -- --ignored --test-threads=1
```

For the optional authoritative Gregorio check, use an already-installed TeX
Live Docker image (override with `CHANTEDIT_TEX_IMAGE`):

```sh
cargo test --test gregorio -- --ignored
```

This compiles plain and annotated scores and checks that their glyph sequences
and horizontal glyph coordinates are identical. Set
`CHANTEDIT_GREGORIO_SOURCE=/path/to/score.gabc` to check another score.
Docker is used only for this verification, never by the app.

## License

ChantEdit is [GPL-3.0-or-later](LICENSE).
The embedded Exsurge renderer is [MIT-licensed](data/vendor/exsurge.LICENSE).
