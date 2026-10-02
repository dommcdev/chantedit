//! Core of ChantEdit: score analysis, chord layout, the `.ce` document format
//! and PDF export. The GTK front end lives in the `chantedit` binary.

pub mod analysis;
pub mod automation;
pub mod document;
pub mod export;
pub mod layout;
pub mod pdf;
pub mod prefs;
mod qpdf;
