//! Poppler helpers. Each thread that renders opens its own [`poppler::Document`]
//! from the shared PDF bytes.

use gtk::{cairo, glib};

use crate::analysis;

pub fn open(bytes: &glib::Bytes) -> Result<poppler::Document, glib::Error> {
    poppler::Document::from_bytes(bytes, None)
}

/// Page sizes in points.
pub fn page_sizes(doc: &poppler::Document) -> Vec<(f64, f64)> {
    (0..doc.n_pages())
        .map(|i| doc.page(i).map_or((612.0, 792.0), |p| p.size()))
        .collect()
}

/// A page rendered to 32-bit pixels (cairo RGB24 layout: B, G, R, X).
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub data: Vec<u8>,
}

/// Renders a page on white at `scale` pixels per point.
pub fn render(
    page: &poppler::Page,
    scale: f64,
    for_printing: bool,
) -> Result<Raster, cairo::Error> {
    let (pw, ph) = page.size();
    let width = ((pw * scale).round() as i32).max(1);
    let height = ((ph * scale).round() as i32).max(1);
    let mut surface = cairo::ImageSurface::create(cairo::Format::Rgb24, width, height)?;
    {
        let cr = cairo::Context::new(&surface)?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint()?;
        cr.scale(f64::from(width) / pw, f64::from(height) / ph);
        if for_printing {
            page.render_for_printing(&cr);
        } else {
            page.render(&cr);
        }
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface
        .data()
        .map_err(|_| cairo::Error::SurfaceFinished)?
        .to_vec();
    Ok(Raster {
        width: width as usize,
        height: height as usize,
        stride,
        data,
    })
}

/// Renders and analyses one page.
pub fn analyze_page(page: &poppler::Page) -> Result<analysis::Page, cairo::Error> {
    let r = render(page, analysis::DPI / 72.0, true)?;
    let mut gray = Vec::with_capacity(r.width * r.height);
    for row in r.data.chunks_exact(r.stride).take(r.height) {
        gray.extend(row[..r.width * 4].chunks_exact(4).map(|px| {
            let (b, g, r) = (u32::from(px[0]), u32::from(px[1]), u32::from(px[2]));
            ((b * 29 + g * 150 + r * 77) >> 8) as u8
        }));
    }
    let (pw, ph) = page.size();
    Ok(analysis::Page::analyze(&gray, r.width, r.height, pw, ph))
}
