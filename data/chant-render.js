// No DOM: supply the canvas text-measurement interface using native Pango.
function renderChant(source, targets, width) {
  const e = exsurge;
  const ctxt = new e.ChantContext();
  ctxt.textMeasuringStrategy = e.TextMeasuringStrategy.Canvas;
  ctxt.canvasCtxt = {
    font: '16px serif',
    measureText(text) { return JSON.parse(nativeMeasure(this.font, text)); }
  };
  ctxt.setFont('serif', 16);
  ctxt.activeClef = e.Clef.default();
  const mappings = [], translations = [];
  let offset = 0;
  // Exsurge's convenience parser trims words but does not account for the
  // trimmed whitespace in sourceIndex. Preserve that offset explicitly.
  for (const raw of e.Gabc.splitWords(source)) {
    const word = raw.trim();
    if (word) {
      const leading = raw.length - raw.trimStart().length;
      mappings.push(e.Gabc.createMappingFromWord(ctxt, word, offset + leading, translations));
    }
    offset += raw.length + 1;
  }
  const last = mappings[mappings.length - 1];
  if (last && last.notations.length) last.notations[last.notations.length - 1].trailingSpace = 0;
  const score = new e.ChantScore(ctxt, mappings, false);
  score.performLayout(ctxt);
  score.layoutChantLines(ctxt, width - 32);
  let defs = '';
  for (const key in ctxt.defs) defs += ctxt.defs[key];
  defs += ctxt.createStyle();
  const pages = [], positions = [];
  const mapped = new Set();
  const wanted = new Map(targets.map((offset, index) => [offset, index]));
  for (const line of score.lines) {
    const page = pages.length;
    const top = line.bounds.y + line.notationBounds.y - 64;
    const baseline = line.bounds.y - top;
    const height = line.bounds.height + 88;
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${width}" height="${height}"><defs>${defs}</defs><g transform="translate(16,0)">${line.createSvgFragment(ctxt, top)}</g></svg>`;
    pages.push({svg, width, height});
    for (let i = line.notationsStartIndex; i < line.notationsStartIndex + line.numNotationsOnLine; i++) {
      const notation = score.notations[i];
      for (const note of notation.notes || []) {
        const index = wanted.get(note.sourceIndex);
        if (index === undefined || mapped.has(index)) continue;
        mapped.add(index);
        positions.push({index, page,
          x: 16 + notation.bounds.x + note.bounds.x,
          y: baseline + Math.min(-ctxt.staffInterval * 3 - 16, line.notationBounds.y - 12),
          note_y: baseline + note.bounds.y});
      }
    }
  }
  const found = new Set(positions.map(p => p.index));
  if (found.size !== targets.length) {
    const missing = targets.map((_, i) => i).filter(i => !found.has(i));
    throw new Error(`Preview cannot map neume anchors ${missing.join(', ')}. This notation is not supported by the preview renderer.`);
  }
  positions.sort((a, b) => a.index - b.index);
  return JSON.stringify({pages, positions});
}
