fn main() {
    pkg_config::Config::new()
        .probe("librsvg-2.0")
        .expect("librsvg development files are required");
}
