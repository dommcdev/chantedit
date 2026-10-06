//! Offline Exsurge/QuickJS SVG preview. No TeX, PDF or external process.

use gtk::{cairo, glib, pango, prelude::*};
use rquickjs::{Context, Function, Runtime};
use serde::Deserialize;

use crate::gabc;

#[derive(Clone, Debug, Deserialize)]
pub struct Position {
    pub index: usize,
    pub page: usize,
    pub x: f64,
    pub y: f64,
    pub note_y: f64,
}

#[derive(Deserialize)]
pub struct Page {
    pub svg: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Deserialize)]
pub struct Preview {
    pub pages: Vec<Page>,
    pub positions: Vec<Position>,
}

pub fn render(doc: &gabc::Document, width: f64) -> Result<Preview, String> {
    let runtime = Runtime::new().map_err(|e| e.to_string())?;
    runtime.set_memory_limit(64 * 1024 * 1024);
    runtime.set_max_stack_size(1024 * 1024);
    let started = std::time::Instant::now();
    runtime.set_interrupt_handler(Some(Box::new(move || started.elapsed().as_secs() > 10)));
    let context = Context::full(&runtime).map_err(|e| e.to_string())?;
    let font_context = pangocairo::FontMap::default().create_context();
    context.with(|ctx| {
        let result = (|| -> rquickjs::Result<String> {
            ctx.globals().set(
                "nativeMeasure",
                Function::new(ctx.clone(), move |font: String, text: String| {
                    measure(&font_context, &font, &text)
                })?,
            )?;
            ctx.eval::<(), _>(include_str!("../data/vendor/exsurge.min.js"))?;
            ctx.eval::<(), _>(include_str!("../data/chant-render.js"))?;
            let source = doc.preview_source();
            // Compute indices using the original header length, not normalized CRLF.
            let original_start = doc
                .music
                .split_inclusive('\n')
                .scan(0, |offset, line| {
                    *offset += line.len();
                    Some((line.trim() == "%%", *offset))
                })
                .find(|(separator, _)| *separator)
                .unwrap()
                .1;
            let body = &source[original_start..];
            // JavaScript indices count UTF-16 code units, Rust spans count bytes.
            let targets: Vec<usize> = doc
                .anchors
                .iter()
                .map(|a| doc.music[original_start..a.offset].encode_utf16().count())
                .collect();
            let code = format!(
                "renderChant({}, {}, {})",
                serde_json::to_string(body).unwrap(),
                serde_json::to_string(&targets).unwrap(),
                width
            );
            ctx.eval(code)
        })();
        result
            .map_err(|error| {
                if error.is_exception() {
                    let thrown = ctx.catch();
                    format!(
                        "Chant renderer: {}",
                        thrown
                            .as_exception()
                            .and_then(|e| e.message())
                            .unwrap_or_else(|| format!("{thrown:?}"))
                    )
                } else {
                    error.to_string()
                }
            })
            .and_then(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
    })
}

fn measure(context: &pango::Context, font: &str, text: &str) -> String {
    let tokens: Vec<&str> = font.split_whitespace().collect();
    let size = tokens
        .iter()
        .find_map(|s| s.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()))
        .unwrap_or(16.0);
    let mut desc = pango::FontDescription::from_string("serif");
    desc.set_absolute_size(size * f64::from(pango::SCALE));
    if font.contains("bold") {
        desc.set_weight(pango::Weight::Bold);
    }
    if font.contains("italic") {
        desc.set_style(pango::Style::Italic);
    }
    let layout = pango::Layout::new(context);
    layout.set_font_description(Some(&desc));
    layout.set_text(text);
    let (ink, logical) = layout.extents();
    let scale = f64::from(pango::SCALE);
    let baseline = f64::from(layout.baseline()) / scale;
    serde_json::json!({
        "width": f64::from(logical.width()) / scale,
        "actualBoundingBoxLeft": -f64::from(ink.x()) / scale,
        "actualBoundingBoxAscent": baseline - f64::from(ink.y()) / scale,
        "actualBoundingBoxDescent": f64::from(ink.y() + ink.height()) / scale - baseline
    })
    .to_string()
}

#[repr(C)]
struct Rectangle {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

unsafe extern "C" {
    fn rsvg_handle_new_from_data(
        data: *const u8,
        len: usize,
        error: *mut *mut glib::ffi::GError,
    ) -> *mut glib::gobject_ffi::GObject;
    fn rsvg_handle_render_document(
        handle: *mut glib::gobject_ffi::GObject,
        cr: *mut cairo::ffi::cairo_t,
        viewport: *const Rectangle,
        error: *mut *mut glib::ffi::GError,
    ) -> glib::ffi::gboolean;
}

/// A native librsvg handle, kept on the GTK thread and rendered at any zoom.
pub struct Svg(glib::Object);

impl Svg {
    pub fn new(source: &str) -> Result<Self, String> {
        use glib::translate::from_glib_full;
        let mut error = std::ptr::null_mut();
        // SAFETY: librsvg copies the supplied bytes; GLib owns the result/error.
        unsafe {
            let handle = rsvg_handle_new_from_data(source.as_ptr(), source.len(), &mut error);
            if !error.is_null() {
                return Err(from_glib_full::<_, glib::Error>(error).to_string());
            }
            if handle.is_null() {
                return Err("Could not load chant SVG".into());
            }
            Ok(Self(from_glib_full(handle)))
        }
    }

    pub fn draw(&self, cr: &cairo::Context, width: f64, height: f64) -> Result<(), String> {
        use glib::translate::{ToGlibPtr, from_glib_full};
        let viewport = Rectangle {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        let mut error = std::ptr::null_mut();
        // SAFETY: the live GObject and Cairo context outlive this synchronous call.
        unsafe {
            let ok = rsvg_handle_render_document(
                self.0.to_glib_none().0,
                cr.to_raw_none(),
                &viewport,
                &mut error,
            );
            if !error.is_null() {
                return Err(from_glib_full::<_, glib::Error>(error).to_string());
            }
            if ok == 0 {
                return Err("Could not draw chant SVG".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_unicode_comments_and_neumes_without_pdf_detection() {
        let source = "name:Test;\r\n%%\r\n(c4)  Dó(ghg/h.)\r\n% 😀 (fake)\r\n mi(hi)nus(g.) Repeat(hsss/hvv) (::)\r\n";
        let doc = gabc::Document::parse(source.into()).unwrap();
        let preview = render(&doc, 720.0).unwrap();
        assert_eq!(preview.positions.len(), doc.anchors.len());
        assert!(preview.positions.windows(2).all(|p| p[0].x < p[1].x));
        let svg = Svg::new(&preview.pages[0].svg).unwrap();
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, 720, 200).unwrap();
        svg.draw(
            &cairo::Context::new(&surface).unwrap(),
            720.0,
            preview.pages[0].height,
        )
        .unwrap();
    }
}
