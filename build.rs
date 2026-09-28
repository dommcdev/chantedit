fn main() {
    // Export stamps the chords onto the original pages with libqpdf's job API.
    pkg_config::Config::new()
        .atleast_version("11.0")
        .probe("libqpdf")
        .expect("libqpdf development files are required (the qpdf package)");
}
