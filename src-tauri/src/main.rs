// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--install-runtime") {
        let code = easysrt_studio_lib::setup_manager::run_cli_bootstrap(&args);
        std::process::exit(code);
    }

    easysrt_studio_lib::run();
}
