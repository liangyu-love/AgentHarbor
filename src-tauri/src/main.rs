#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().nth(1).as_deref() == Some("daemon-run-once") {
        eprintln!("Automatic account switching is not available in this source build.");
        std::process::exit(1);
    }

    agentharbor_lib::run()
}
