mod app_state;
mod commands;
mod dto;

use app_state::AppState;
use commands::{
    cancel_run, delete_raw_artifacts, delete_run, delete_target_history, export_public_report,
    get_bootstrap, get_run_detail, list_runs, next_manual_step, resume_cli_run, resume_manual_run,
    start_cli_run, start_manual_run, submit_manual_answer,
};
use tauri::Manager;

macro_rules! command_inventory {
    ($consumer:ident) => {
        $consumer!(
            get_bootstrap,
            start_manual_run,
            next_manual_step,
            submit_manual_answer,
            start_cli_run,
            resume_manual_run,
            resume_cli_run,
            cancel_run,
            list_runs,
            get_run_detail,
            export_public_report,
            delete_raw_artifacts,
            delete_run,
            delete_target_history,
        )
    };
}

macro_rules! command_handler {
    ($($command:ident),+ $(,)?) => {
        tauri::generate_handler![$($command),+]
    };
}

#[cfg(test)]
macro_rules! command_names {
    ($($command:ident),+ $(,)?) => {
        &[$(stringify!($command)),+]
    };
}

#[cfg(test)]
const INVOKE_COMMANDS: &[&str] = command_inventory!(command_names);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = AppState::build(app).map_err(std::io::Error::other)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(command_inventory!(command_handler))
        .run(tauri::generate_context!())
        .expect("failed to run AI Ability Radar");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn invoke_surface_is_the_exact_reviewed_allowlist() {
        assert_eq!(
            INVOKE_COMMANDS,
            [
                "get_bootstrap",
                "start_manual_run",
                "next_manual_step",
                "submit_manual_answer",
                "start_cli_run",
                "resume_manual_run",
                "resume_cli_run",
                "cancel_run",
                "list_runs",
                "get_run_detail",
                "export_public_report",
                "delete_raw_artifacts",
                "delete_run",
                "delete_target_history",
            ]
        );
    }

    #[test]
    fn capability_contains_only_core_default() {
        let capability: Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert_eq!(capability["windows"], json!(["main"]));
        assert_eq!(capability["permissions"], json!(["core:default"]));
    }

    #[test]
    fn production_config_bundles_packs_and_has_a_restrictive_local_csp() {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["bundle"]["targets"], json!(["nsis", "msi"]));
        assert_eq!(
            config["bundle"]["resources"]["../../../benchmark-packs/"],
            "benchmark-packs/"
        );

        let csp = config["app"]["security"]["csp"].as_str().unwrap();
        assert!(csp.contains("default-src 'self'"));
        assert!(csp.contains("connect-src ipc: http://ipc.localhost"));
        assert!(csp.contains("object-src 'none'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(!csp.contains('*'));
        assert!(!csp.contains("https://"));
    }

    #[test]
    fn rust_backend_has_no_generic_opener_dependency() {
        assert!(!include_str!("../Cargo.toml").contains("tauri-plugin-opener"));
    }

    #[test]
    fn native_report_picker_is_registered_without_webview_file_authority() {
        let cargo = include_str!("../Cargo.toml");
        let source = include_str!("lib.rs");
        let capability: Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();

        assert!(cargo.contains("tauri-plugin-dialog"));
        assert!(source.contains(".plugin(tauri_plugin_dialog::init())"));
        assert_eq!(capability["permissions"], json!(["core:default"]));
        assert!(!cargo.contains("tauri-plugin-fs"));
        assert!(!capability.to_string().contains("dialog:"));
        assert!(!capability.to_string().contains("fs:"));
    }
}
