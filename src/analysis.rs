//! Finds staves, chord-line positions and note columns on a rendered page.
//!
//! Only pixels are inspected, so vector music, real text and scans all work
//! the same way. Per page:
//!
//! 1. Threshold a grayscale render into a dark-pixel mask.
//! 2. Staff lines are rows containing long horizontal dark runs.
//! 3. Lines with regular spacing are grouped into staves (3–6 lines).
//! 4. For every staff, the band up to the text above it (the previous
//!    system's lyrics, a title, …) is profiled so a chord line can be fitted
//!    for any chord height.
//! 5. Note/neume columns are found so chords can snap to notes.
//!
//! Public coordinates are PDF points with the origin at the top left.

use std::sync::Arc;

/// Resolution pages are analysed at.
pub const DPI: f64 = 150.0;

const DARK_THRESHOLD: u8 = 150;

#[derive(Debug, Clone)]
pub struct Staff {
    /// y of the top and bottom staff lines.
    pub top: f64,
    pub bottom: f64,
    /// Distance between staff lines.
    pub space: f64,
    pub x0: f64,
    pub x1: f64,
    pub n_lines: usize,
    /// x centres of note columns, ascending.
    pub notes: Arc<[f64]>,
}

/// A chord line fitted above a staff for a given chord height.
#[derive(Debug, Clone, Copy)]
pub struct Fit {
    /// Vertical centre of the chord text.
    pub y: f64,
    /// The free band the line was centred in.
    pub gap_top: f64,
    pub gap_bottom: f64,
    /// Chords may be moved between `ceil` and `floor` to avoid ink.
    pub ceil: f64,
    pub floor: f64,
    /// The chords do not really fit.
    pub tight: bool,
}

/// Row profile of the band between a staff and whatever is above it.
#[derive(Debug)]
struct Region {
    /// First row (px) of the band.
    r0: usize,
    /// Dark pixels per row within the staff's x range.
    prof: Vec<u32>,
    /// Row looks like it cuts through a line of text.
    text: Vec<bool>,
}

#[derive(Debug)]
pub struct Page {
    /// Page size in points.
    pub width: f64,
    pub height: f64,
    pub staves: Vec<Staff>,
    /// px per pt.
    scale: f64,
    ink: Bitmap,
    regions: Vec<Region>,
}

impl Page {
    /// Inspects a grayscale page render (0 = black) of `w`×`h` pixels.
    pub fn analyze(gray: &[u8], w: usize, h: usize, width: f64, height: f64) -> Page {
        assert_eq!(gray.len(), w * h);
        let dark: Vec<u8> = gray.iter().map(|&g| u8::from(g < DARK_THRESHOLD)).collect();
        let s = w as f64 / width;

        let lines = find_lines(&dark, w, h);
        let staves: Vec<Staff> = group_staves(&lines, h)
            .into_iter()
            .map(|g| {
                let top = g[0].y;
                let bottom = g[g.len() - 1].y;
                let sp = (bottom - top) / (g.len() - 1) as f64;
                let x0 = median(g.iter().map(|l| l.x0));
                let x1 = median(g.iter().map(|l| l.x1));
                let notes = note_columns(&dark, w, h, top, bottom, sp, x0, x1, &g);
                Staff {
                    top: top / s,
                    bottom: bottom / s,
                    space: sp / s,
                    x0: x0 / s,
                    x1: x1 / s,
                    n_lines: g.len(),
                    notes: notes.into_iter().map(|x| x / s).collect(),
                }
            })
            .collect();
        let regions = (0..staves.len())
            .map(|i| text_region(&dark, w, h, s, &staves, i))
            .collect();

        Page {
            width,
            height,
            staves,
            scale: s,
            ink: Bitmap::from_mask(&dark, w, h),
            regions,
        }
    }

    /// Counts dark pixels inside a rectangle given in points.
    pub fn ink_in_rect(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> u32 {
        let s = self.scale;
        let (w, h) = (self.ink.w as isize, self.ink.h as isize);
        let c0 = ((x0 * s) as isize).clamp(0, w) as usize;
        let c1 = ((x1 * s + 0.999) as isize).clamp(0, w) as usize;
        let r0 = ((y0 * s) as isize).clamp(0, h) as usize;
        let r1 = ((y1 * s + 0.999) as isize).clamp(0, h) as usize;
        self.ink.count(c0, c1, r0, r1)
    }

    /// Finds the chord line above staff `i` for chords `cap_height` points tall.
    pub fn fit_line(&self, i: usize, cap_height: f64) -> Fit {
        let st = &self.staves[i];
        let rg = &self.regions[i];
        let s = self.scale;
        let sp = st.space * s;
        let n = rg.prof.len() as isize;
        let width_px = ((st.x1 - st.x0) * s).max(1.0);
        let need = (cap_height * s * 1.2) as isize + 2;
        let prof = |row: isize| f64::from(rg.prof[row as usize]);

        // Lowest text line above the staff (the previous system's lyrics, or
        // a title). Rows right on top of the staff are notes, not text.
        let min_rows = 3.max((0.3 * sp) as isize);
        let skip = (0.4 * sp) as isize;
        let mut barrier = None;
        let mut run = 0;
        let mut y = n - 1 - skip;
        while y >= 0 {
            if rg.text[y as usize] {
                run += 1;
                if run >= min_rows {
                    barrier = Some(y + run - 1);
                    break;
                }
            } else {
                run = 0;
            }
            y -= 1;
        }

        let hi = n;
        let dense = (0.05 * width_px).max(2.0);
        let mut top = match barrier {
            // Include descender-heavy rows directly under the text.
            Some(b) => {
                let mut t = b + 1;
                while t < hi && prof(t) >= dense {
                    t += 1;
                }
                t
            }
            // No text line (e.g. the first system under a title): stop at the
            // first substantial ink above the zone where high notes live.
            None => {
                let mut y = hi - 1 - (1.6 * sp) as isize;
                while y >= 0 && prof(y) < dense {
                    y -= 1;
                }
                (y + 1).max(0)
            }
        };
        let bot = hi;
        let to_pt = |row: isize| (rg.r0 as isize + row) as f64 / s;
        let ceil = to_pt(top);
        let floor = st.top - 0.1 * st.space;

        // The line is centred between the text above and the staff. Notes
        // poking above the staff and descenders are left to per-chord
        // avoidance, so a single high note doesn't push the whole line
        // around. In a big gap, stay close to the staff.
        let max_dist = (need as f64 * 1.8).max(sp * 2.2);
        if (bot - top) as f64 > max_dist {
            top = bot - max_dist as isize;
        }
        let (gap_top, gap_bottom) = (to_pt(top), to_pt(bot));
        Fit {
            y: (gap_top + gap_bottom) / 2.0,
            gap_top,
            gap_bottom,
            ceil,
            floor,
            tight: bot - top < need,
        }
    }
}

fn text_region(dark: &[u8], w: usize, h: usize, s: f64, staves: &[Staff], i: usize) -> Region {
    let st = &staves[i];
    let top_px = st.top * s;
    let sp_px = st.space * s;
    let limit = top_px - 9.0 * sp_px;
    let r0 = staves[..i]
        .iter()
        .rev()
        .find(|o| o.bottom < st.top && o.x1.min(st.x1) - o.x0.max(st.x0) > 0.0)
        .map_or(limit, |o| limit.max(o.bottom * s + 0.5 * sp_px));
    let ri0 = (r0 as isize).max(0) as usize;
    let ri1 = (ri0 + 1)
        .max((top_px - 0.15 * sp_px).max(0.0) as usize)
        .min(h);
    let c0 = ((st.x0 * s) as usize).min(w);
    let c1 = ((st.x1 * s) as usize).min(w);
    let n = ri1 - ri0;

    // Letters in a word sit close together; stems and bar lines are isolated
    // thin strokes a note-width apart.
    let max_short = 2.max((0.4 * sp_px) as usize);
    let max_gap = 3.max((0.7 * sp_px) as isize);
    let mut prof = vec![0u32; n];
    let mut short_runs = vec![0u32; n];
    for k in 0..n {
        let y = ri0 + k;
        if y >= h {
            break;
        }
        let row = &dark[y * w..(y + 1) * w];
        let mut count = 0;
        let mut prev_end = isize::MIN / 2;
        let mut prev_short = false;
        let mut x = c0;
        while x < c1 {
            if row[x] == 0 {
                x += 1;
                continue;
            }
            let start = x;
            while x < c1 && row[x] == 1 {
                x += 1;
            }
            count += (x - start) as u32;
            let short = x - start <= max_short;
            if short && start as isize - prev_end <= max_gap {
                short_runs[k] += 1;
                if prev_short && short_runs[k] == 1 {
                    short_runs[k] += 1; // count the first stroke of the word too
                }
            }
            prev_end = x as isize;
            prev_short = short;
        }
        prof[k] = count;
    }

    // Text rows are cut into many thin, closely spaced strokes; note heads give
    // few wide runs and stems are far apart. Bridge single-row holes.
    let mut text: Vec<bool> = short_runs.iter().map(|&r| r >= 4).collect();
    for k in 1..n.saturating_sub(1) {
        if !text[k] && text[k - 1] && text[k + 1] {
            text[k] = true;
        }
    }
    Region {
        r0: ri0,
        prof,
        text,
    }
}

// ------------------------------------------------------------------ staff lines

#[derive(Debug, Clone, Copy)]
struct Line {
    y: f64,
    thick: f64,
    x0: f64,
    x1: f64,
}

fn find_lines(dark: &[u8], w: usize, h: usize) -> Vec<Line> {
    // Close small horizontal gaps (broken scan lines), then merge each row with
    // the one below so slightly tilted lines still give long runs.
    const GAP: usize = 2;
    let mut m = vec![0u8; w * h];
    for y in 0..h {
        let row = &dark[y * w..(y + 1) * w];
        let out = &mut m[y * w..(y + 1) * w];
        let mut last: Option<usize> = None;
        for x in 0..w {
            if row[x] == 1 {
                if let Some(l) = last {
                    let d = x - l;
                    if d > 1 && d <= GAP + 1 {
                        out[l + 1..x].fill(1);
                    }
                }
                out[x] = 1;
                last = Some(x);
            }
        }
    }
    for y in 0..h.saturating_sub(1) {
        let (a, b) = m[y * w..(y + 2) * w].split_at_mut(w);
        for (p, q) in a.iter_mut().zip(b.iter()) {
            *p |= *q;
        }
    }

    let min_run = 20.max((w as f64 * 0.05) as usize);
    let mut score = vec![0.0f64; h];
    let mut rx0 = vec![w as f64; h];
    let mut rx1 = vec![0.0f64; h];
    for y in 0..h {
        let row = &m[y * w..(y + 1) * w];
        let mut x = 0;
        while x < w {
            if row[x] == 0 {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && row[x] == 1 {
                x += 1;
            }
            let len = x - start;
            if len >= min_run {
                score[y] += len as f64;
                rx0[y] = rx0[y].min(start as f64);
                rx1[y] = rx1[y].max(x as f64);
            }
        }
    }

    let thr = ((min_run * 2) as f64).max(w as f64 * 0.15);
    let mut lines = Vec::new();
    let mut y = 0;
    while y < h {
        if score[y] < thr {
            y += 1;
            continue;
        }
        let start = y;
        let (mut sw, mut swy) = (0.0, 0.0);
        while y < h && score[y] >= thr {
            sw += score[y];
            swy += score[y] * y as f64;
            y += 1;
        }
        lines.push(Line {
            y: swy / sw + 0.5,
            thick: (y - start) as f64,
            x0: median(rx0[start..y].iter().copied()),
            x1: median(rx1[start..y].iter().copied()),
        });
    }
    lines
}

fn group_staves(lines: &[Line], h: usize) -> Vec<Vec<Line>> {
    let max_space = h as f64 * 0.03;
    let overlap = |a: &Line, b: &Line| {
        let inter = a.x1.min(b.x1) - a.x0.max(b.x0);
        inter / (a.x1 - a.x0).min(b.x1 - b.x0).max(1.0)
    };
    let mut groups = Vec::new();
    let mut cur: Vec<Line> = Vec::new();
    let flush = |cur: &mut Vec<Line>, groups: &mut Vec<Vec<Line>>| {
        if (3..=6).contains(&cur.len()) {
            groups.push(std::mem::take(cur));
        }
        cur.clear();
    };
    for ln in lines {
        let Some(prev) = cur.last() else {
            cur.push(*ln);
            continue;
        };
        let d = ln.y - prev.y;
        let mut ok = d >= 3.0 && d <= max_space && overlap(ln, prev) > 0.6;
        if ok && cur.len() >= 2 {
            let sp = (prev.y - cur[0].y) / (cur.len() - 1) as f64;
            ok = (d - sp).abs() <= 0.25 * sp + 1.0;
        }
        if ok && cur.len() < 6 {
            cur.push(*ln);
        } else {
            flush(&mut cur, &mut groups);
            cur.push(*ln);
        }
    }
    flush(&mut cur, &mut groups);
    groups
}

// ----------------------------------------------------------------- note columns

#[allow(clippy::too_many_arguments)]
fn note_columns(
    dark: &[u8],
    w: usize,
    h: usize,
    top: f64,
    bottom: f64,
    sp: f64,
    x0: f64,
    x1: f64,
    lines: &[Line],
) -> Vec<f64> {
    let clamp = |v: f64, hi: usize| (v as isize).clamp(0, hi as isize) as usize;
    let r0 = clamp(top - 2.0 * sp, h);
    let r1 = clamp(bottom + 1.5 * sp, h);
    let c0 = clamp(x0, w);
    let c1 = clamp(x1, w);
    if r1 <= r0 || c1 <= c0 {
        return Vec::new();
    }
    let bw = c1 - c0;
    let bh = r1 - r0;
    let mut band = vec![0u8; bw * bh];
    for y in 0..bh {
        band[y * bw..(y + 1) * bw].copy_from_slice(&dark[(r0 + y) * w + c0..(r0 + y) * w + c1]);
    }
    // Remove staff-line pixels, but keep note heads that straddle a line (dark
    // both just above and just below it).
    for ln in lines {
        let a = ((ln.y - ln.thick / 2.0 - 1.0) as isize - r0 as isize).max(0) as usize;
        let b =
            ((ln.y + ln.thick / 2.0 + 1.5) as isize - r0 as isize).clamp(0, bh as isize) as usize;
        if b <= a {
            continue;
        }
        for x in 0..bw {
            let keep = a >= 1 && b < bh && band[(a - 1) * bw + x] == 1 && band[b * bw + x] == 1;
            if !keep {
                for y in a..b {
                    band[y * bw + x] = 0;
                }
            }
        }
    }
    let mut col = vec![0u32; bw];
    for row in band.chunks_exact(bw) {
        for (c, &v) in col.iter_mut().zip(row) {
            *c += u32::from(v);
        }
    }

    let thr = (sp * 0.35).max(2.0);
    let min_w = 2.max((sp * 0.35) as usize);
    let head = (sp * 1.1).max(1.0);
    let mut xs = Vec::new();
    let mut c = 0;
    while c < bw {
        if f64::from(col[c]) < thr {
            c += 1;
            continue;
        }
        let start = c;
        let mut min_col = col[c];
        while c < bw && f64::from(col[c]) >= thr {
            min_col = min_col.min(col[c]);
            c += 1;
        }
        let width = c - start;
        if width < min_w {
            continue; // thin bar line, stem or noise
        }
        if f64::from(min_col) >= 0.85 * (bottom - top) && width < sp as usize {
            continue; // thick bar line: full staff height in every column
        }
        // Neumes written side by side merge into one wide cluster; split it
        // into roughly note-head sized pieces.
        let k = 1.max((width as f64 / head + 0.5) as usize);
        for i in 0..k {
            let x = (c0 + start) as f64 + (i as f64 + 0.5) * width as f64 / k as f64;
            // The clef sits right at the start of the staff.
            if x >= x0 + 1.6 * sp {
                xs.push(x);
            }
        }
    }
    xs
}

// ---------------------------------------------------------------------- bitmap

/// Packed 1-bit ink mask for fast rectangle counts.
#[derive(Debug)]
struct Bitmap {
    w: usize,
    h: usize,
    stride: usize,
    words: Vec<u64>,
}

impl Bitmap {
    fn from_mask(mask: &[u8], w: usize, h: usize) -> Bitmap {
        let stride = w.div_ceil(64);
        let mut words = vec![0u64; stride * h];
        for (y, row) in mask.chunks_exact(w).enumerate() {
            for (x, _) in row.iter().enumerate().filter(|&(_, &v)| v != 0) {
                words[y * stride + x / 64] |= 1 << (x % 64);
            }
        }
        Bitmap {
            w,
            h,
            stride,
            words,
        }
    }

    /// Set bits in columns `c0..c1` of rows `r0..r1`.
    fn count(&self, c0: usize, c1: usize, r0: usize, r1: usize) -> u32 {
        if c1 <= c0 || r1 <= r0 {
            return 0;
        }
        let (w0, w1) = (c0 / 64, (c1 - 1) / 64);
        let first = !0u64 << (c0 % 64);
        let last = !0u64 >> (63 - (c1 - 1) % 64);
        let mut n = 0;
        for y in r0..r1 {
            let row = &self.words[y * self.stride..(y + 1) * self.stride];
            if w0 == w1 {
                n += (row[w0] & first & last).count_ones();
            } else {
                n += (row[w0] & first).count_ones() + (row[w1] & last).count_ones();
                n += row[w0 + 1..w1].iter().map(|v| v.count_ones()).sum::<u32>();
            }
        }
        n
    }
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut v: Vec<f64> = values.collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staff_at_top_edge_has_bounded_text_region() {
        let mut gray = vec![255; 200 * 200];
        for y in [0, 4, 8, 12] {
            gray[y * 200 + 10..y * 200 + 190].fill(0);
        }
        let page = Page::analyze(&gray, 200, 200, 200.0, 200.0);
        assert_eq!(page.staves.len(), 1);
        assert!(page.fit_line(0, 6.5).y.is_finite());
    }

    #[test]
    fn bitmap_counts_match_naive() {
        let (w, h) = (150, 7);
        let mask: Vec<u8> = (0..w * h).map(|i| u8::from((i * 7919) % 5 < 2)).collect();
        let bm = Bitmap::from_mask(&mask, w, h);
        for &(c0, c1, r0, r1) in &[
            (0, 150, 0, 7),
            (3, 4, 1, 2),
            (60, 70, 0, 7),
            (63, 129, 2, 6),
            (64, 128, 0, 1),
        ] {
            let naive: u32 = (r0..r1)
                .flat_map(|y| (c0..c1).map(move |x| (x, y)))
                .map(|(x, y)| u32::from(mask[y * w + x]))
                .sum();
            assert_eq!(bm.count(c0, c1, r0, r1), naive, "{c0}..{c1} x {r0}..{r1}");
        }
    }
}
