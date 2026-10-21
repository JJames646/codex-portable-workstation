#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--write-runtime-build-id")
    {
        let Some(destination) = arguments.get(index + 1) else {
            std::process::exit(4);
        };
        if std::fs::write(
            destination,
            format!("{}\n", yunshu_three_engine_workstation::runtime_build_id()),
        )
        .is_err()
        {
            std::process::exit(5);
        }
        return;
    }
    if std::env::args_os().any(|argument| argument == "--verify-embedded-ui") {
        std::process::exit(
            if yunshu_three_engine_workstation::embedded_frontend_enabled() {
                0
            } else {
                3
            },
        );
    }
    if std::env::args_os().any(|argument| argument == "--phase5-smoke") {
        if yunshu_three_engine_workstation::run_smoke().is_err() {
            std::process::exit(2);
        }
        return;
    }
    yunshu_three_engine_workstation::run()
}
