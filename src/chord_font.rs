//! Pango chord text measured in preview units, independent of window zoom.

use gtk::{cairo, pango, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy)]
pub struct Metrics {
    pub width: f64,
    pub ascent: f64,
    pub ink_y0: f64,
    pub ink_y1: f64,
}

pub struct Text {
    pub layout: pango::Layout,
    pub metrics: Metrics,
}

pub struct ChordFont {
    size: f64,
    context: pango::Context,
    desc: pango::FontDescription,
    cache: RefCell<HashMap<String, Rc<Text>>>,
}

impl ChordFont {
    pub fn new(name: &str, size: f64) -> Self {
        let context = pangocairo::FontMap::default().create_context();
        let mut options = cairo::FontOptions::new().expect("cairo font options");
        options.set_hint_metrics(cairo::HintMetrics::Off);
        options.set_hint_style(cairo::HintStyle::None);
        pangocairo::functions::context_set_font_options(&context, Some(&options));
        context.set_round_glyph_positions(false);
        let mut desc = pango::FontDescription::from_string(name);
        desc.set_absolute_size(size * f64::from(pango::SCALE));
        Self {
            size,
            context,
            desc,
            cache: RefCell::new(HashMap::new()),
        }
    }

    pub fn size(&self) -> f64 {
        self.size
    }

    pub fn text(&self, text: &str) -> Rc<Text> {
        if let Some(found) = self.cache.borrow().get(text) {
            return found.clone();
        }
        let layout = pango::Layout::new(&self.context);
        layout.set_font_description(Some(&self.desc));
        layout.set_text(text);
        let (ink, logical) = layout.extents();
        let unit = |n| f64::from(n) / f64::from(pango::SCALE);
        let ascent = unit(layout.baseline());
        let text = Rc::new(Text {
            layout,
            metrics: Metrics {
                width: unit(logical.width()),
                ascent,
                ink_y0: unit(ink.y()) - ascent,
                ink_y1: unit(ink.y() + ink.height()) - ascent,
            },
        });
        self.cache
            .borrow_mut()
            .insert(text.layout.text().to_string(), text.clone());
        text
    }

    pub fn draw(&self, cr: &cairo::Context, text: &str, center: f64, baseline: f64) {
        let text = self.text(text);
        cr.move_to(
            center - text.metrics.width / 2.0,
            baseline - text.metrics.ascent,
        );
        pangocairo::functions::show_layout(cr, &text.layout);
    }
}
