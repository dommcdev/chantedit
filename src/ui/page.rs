//! Widget showing one score page: the rendered page image plus an overlay
//! drawn by the window (guides, chords, cursor).

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

type Painter = Box<dyn Fn(&gtk::Snapshot)>;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    pub struct PageView {
        /// Page size in points.
        pub size: Cell<(f64, f64)>,
        pub zoom: Cell<f64>,
        pub texture: RefCell<Option<(gdk::Texture, f64)>>,
        pub painter: RefCell<Option<Painter>>,
    }

    impl Default for PageView {
        fn default() -> Self {
            PageView {
                size: Cell::new((612.0, 792.0)),
                zoom: Cell::new(1.0),
                texture: RefCell::default(),
                painter: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PageView {
        const NAME: &'static str = "ChantPageView";
        type Type = super::PageView;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("chantpage");
        }
    }

    impl ObjectImpl for PageView {}

    impl WidgetImpl for PageView {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let (w, h) = self.size.get();
            let pt = if orientation == gtk::Orientation::Horizontal {
                w
            } else {
                h
            };
            let px = (pt * self.zoom.get()).round() as i32;
            (px, px, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let bounds = graphene::Rect::new(0.0, 0.0, obj.width() as f32, obj.height() as f32);
            snapshot.append_color(&gdk::RGBA::WHITE, &bounds);
            if let Some((texture, _)) = &*self.texture.borrow() {
                snapshot.append_scaled_texture(texture, gsk::ScalingFilter::Trilinear, &bounds);
            }
            if let Some(paint) = &*self.painter.borrow() {
                paint(snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct PageView(ObjectSubclass<imp::PageView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PageView {
    pub fn new(width: f64, height: f64, zoom: f64) -> PageView {
        let obj: PageView = glib::Object::builder()
            .property("halign", gtk::Align::Center)
            .build();
        obj.imp().size.set((width, height));
        obj.imp().zoom.set(zoom);
        obj
    }

    pub fn page_size(&self) -> (f64, f64) {
        self.imp().size.get()
    }

    pub fn set_zoom(&self, zoom: f64) {
        if self.imp().zoom.replace(zoom) != zoom {
            self.queue_resize();
        }
    }

    /// Scale (device pixels per point) of the current page image, 0 if none.
    pub fn texture_scale(&self) -> f64 {
        self.imp().texture.borrow().as_ref().map_or(0.0, |t| t.1)
    }

    pub fn set_texture(&self, texture: Option<(gdk::Texture, f64)>) {
        self.imp().texture.replace(texture);
        self.queue_draw();
    }

    pub fn set_painter(&self, paint: impl Fn(&gtk::Snapshot) + 'static) {
        self.imp().painter.replace(Some(Box::new(paint)));
    }
}
