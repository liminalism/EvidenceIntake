#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![allow(missing_docs)]

#[cfg(target_os = "windows")]
fn main() {
    let database = std::env::args_os().nth(1).map_or_else(
        || std::path::PathBuf::from("evidence.db"),
        std::path::PathBuf::from,
    );
    evidence_intake::gui::winsafe::run(&database);
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!(
        "The gui-winsafe feature provides the Windows frontend only; use a future Linux adapter on this platform."
    );
}
