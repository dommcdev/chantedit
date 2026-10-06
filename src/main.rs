mod ui;

fn main() -> gtk::glib::ExitCode {
    if let Some(code) = chantedit::cli::run() {
        return code;
    }
    ui::run()
}
