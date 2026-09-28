//! Background threads for one open document: page analysis, and rendering
//! page images for display.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use chantedit::{analysis, pdf};
use gtk::glib;

pub enum Msg {
    Analysed {
        doc: u64,
        page: usize,
        result: Arc<analysis::Page>,
    },
    Rendered {
        doc: u64,
        page: usize,
        epoch: u64,
        image: Image,
    },
    Failed {
        doc: u64,
        message: String,
    },
}

/// Page pixels in `gdk::MemoryFormat::B8g8r8x8` layout.
pub struct Image {
    pub width: i32,
    pub height: i32,
    pub stride: usize,
    pub data: glib::Bytes,
}

struct Job {
    page: usize,
    scale: f64,
    epoch: u64,
}

/// Stops its threads when dropped.
pub struct Workers {
    jobs: mpsc::Sender<Job>,
    epoch: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
}

impl Workers {
    pub fn start(doc: u64, pdf_bytes: glib::Bytes, out: async_channel::Sender<Msg>) -> Workers {
        let cancelled = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(0));

        let (bytes, tx, stop) = (pdf_bytes.clone(), out.clone(), cancelled.clone());
        thread::Builder::new()
            .name("analysis".into())
            .spawn(move || {
                let fail = |message: String| {
                    let _ = tx.send_blocking(Msg::Failed { doc, message });
                };
                let pdf = match pdf::open(&bytes) {
                    Ok(p) => p,
                    Err(e) => return fail(e.to_string()),
                };
                for i in 0..pdf.n_pages() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Some(page) = pdf.page(i) else { continue };
                    match pdf::analyze_page(&page) {
                        Ok(a) => {
                            let msg = Msg::Analysed {
                                doc,
                                page: i as usize,
                                result: Arc::new(a),
                            };
                            if tx.send_blocking(msg).is_err() {
                                return;
                            }
                        }
                        Err(e) => fail(format!("page {}: {e}", i + 1)),
                    }
                }
            })
            .expect("spawn analysis thread");

        let (jobs, rx) = mpsc::channel::<Job>();
        let current = epoch.clone();
        thread::Builder::new()
            .name("render".into())
            .spawn(move || {
                let Ok(pdf) = pdf::open(&pdf_bytes) else {
                    return;
                };
                for job in rx {
                    if job.epoch != current.load(Ordering::Relaxed) {
                        continue;
                    }
                    let Some(page) = pdf.page(job.page as i32) else {
                        continue;
                    };
                    let Ok(r) = pdf::render(&page, job.scale, false) else {
                        continue;
                    };
                    let image = Image {
                        width: r.width as i32,
                        height: r.height as i32,
                        stride: r.stride,
                        data: glib::Bytes::from_owned(r.data),
                    };
                    let msg = Msg::Rendered {
                        doc,
                        page: job.page,
                        epoch: job.epoch,
                        image,
                    };
                    if out.send_blocking(msg).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn render thread");

        Workers {
            jobs,
            epoch,
            cancelled,
        }
    }

    pub fn set_epoch(&self, epoch: u64) {
        self.epoch.store(epoch, Ordering::Relaxed);
    }

    pub fn render(&self, page: usize, scale: f64, epoch: u64) {
        let _ = self.jobs.send(Job { page, scale, epoch });
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.epoch.store(u64::MAX, Ordering::Relaxed);
    }
}
