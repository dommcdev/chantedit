mod ui;

fn main() -> gtk::glib::ExitCode {
    if let Some(code) = chantedit::automation::run_cli() {
        return code;
    }
    ui::run()
}
