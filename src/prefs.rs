//! Application preferences (not document data), in `~/.config/chantedit`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::document::Settings;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Applied to documents created from a PDF.
    pub defaults: Settings,
    pub last_dir: Option<PathBuf>,
    /// Used for exports when the score's folder is read-only.
    pub export_dir: Option<PathBuf>,
    pub zoom: Option<f64>,
    pub guides: bool,
}

fn path() -> PathBuf {
    gtk::glib::user_config_dir()
        .join("chantedit")
        .join("prefs.json")
}

impl Prefs {
    pub fn load() -> Prefs {
        let mut p: Prefs = fs::read(path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if !p.defaults.size.is_finite() || p.defaults.size <= 0.0 {
            p.defaults = Settings::default();
        }
        p.zoom = p.zoom.filter(|z| z.is_finite() && *z > 0.0);
        p
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = path();
        fs::create_dir_all(path.parent().expect("config path has a parent"))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path)
    }
}
