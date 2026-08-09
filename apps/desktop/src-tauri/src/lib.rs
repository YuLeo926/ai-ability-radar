mod app_state;
mod batch_commands;
mod batch_runner;
mod client_selection;
mod commands;
mod data_management;
mod dto;

#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod data_management_tests;

use app_state::AppState;
use batch_commands::{
    authorize_batch_execution, authorize_batch_retry, begin_guided_batch_member, cancel_batch,
    create_acknowledged_batch, decline_guided_batch_attestation, delete_batch, estimate_batch,
    estimate_batch_retry, get_batch, get_batch_analysis, get_next_guided_member, list_batches,
    pause_batch, resume_batch, start_batch, submit_guided_batch_answer,
};
use commands::{
    cancel_run, delete_raw_artifacts, delete_run, delete_target_history, detect_client_selection,
    export_full_backup, export_public_batch_report, export_public_report, get_bootstrap,
    get_data_settings, get_run_detail, interrupt_manual_run, list_runs, next_manual_step,
    resume_cli_run, resume_manual_run, set_raw_retention, start_cli_run, start_manual_run,
    submit_manual_answer,
};
use tauri::Manager;

macro_rules! command_inventory {
    ($consumer:ident) => {
        $consumer!(
            get_bootstrap,
            detect_client_selection,
            start_manual_run,
            next_manual_step,
            submit_manual_answer,
            start_cli_run,
            resume_manual_run,
            resume_cli_run,
            cancel_run,
            interrupt_manual_run,
            list_runs,
            get_run_detail,
            export_public_report,
            export_public_batch_report,
            delete_raw_artifacts,
            delete_run,
            delete_target_history,
            get_data_settings,
            set_raw_retention,
            export_full_backup,
            estimate_batch,
            create_acknowledged_batch,
            get_batch,
            get_batch_analysis,
            list_batches,
            delete_batch,
            authorize_batch_execution,
            estimate_batch_retry,
            authorize_batch_retry,
            start_batch,
            resume_batch,
            pause_batch,
            cancel_batch,
            get_next_guided_member,
            begin_guided_batch_member,
            submit_guided_batch_answer,
            decline_guided_batch_attestation,
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
                "detect_client_selection",
                "start_manual_run",
                "next_manual_step",
                "submit_manual_answer",
                "start_cli_run",
                "resume_manual_run",
                "resume_cli_run",
                "cancel_run",
                "interrupt_manual_run",
                "list_runs",
                "get_run_detail",
                "export_public_report",
                "export_public_batch_report",
                "delete_raw_artifacts",
                "delete_run",
                "delete_target_history",
                "get_data_settings",
                "set_raw_retention",
                "export_full_backup",
                "estimate_batch",
                "create_acknowledged_batch",
                "get_batch",
                "get_batch_analysis",
                "list_batches",
                "delete_batch",
                "authorize_batch_execution",
                "estimate_batch_retry",
                "authorize_batch_retry",
                "start_batch",
                "resume_batch",
                "pause_batch",
                "cancel_batch",
                "get_next_guided_member",
                "begin_guided_batch_member",
                "submit_guided_batch_answer",
                "decline_guided_batch_attestation",
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

    #[test]
    fn cross_platform_backup_path_resolves_only_paired_cfg_helpers() {
        let source = include_str!("commands.rs");
        assert!(source.contains("#[cfg(windows)]\nfn write_full_backup_to_destination("));
        assert!(source.contains("#[cfg(not(windows))]\nfn write_full_backup_to_destination("));
        let export_body = source
            .split("fn export_full_backup_to_selected_path(")
            .nth(1)
            .unwrap()
            .split("fn validate_backup_destination(")
            .next()
            .unwrap();
        assert!(export_body.contains("write_full_backup_to_destination("));
        assert!(!export_body.contains("windows_report_file"));

        let non_windows_helper = source
            .split("#[cfg(not(windows))]\nfn write_full_backup_to_destination(")
            .nth(1)
            .unwrap()
            .split("#[cfg(windows)]\nfn write_new_backup")
            .next()
            .unwrap();
        assert!(non_windows_helper.contains("当前版本仅支持在 Windows 上安全导出完整备份。"));
        assert!(!non_windows_helper.contains("windows_report_file"));
        assert!(!non_windows_helper.contains("create_full_backup"));
        assert!(!non_windows_helper.contains("snapshot"));
        assert!(!non_windows_helper.contains("std::fs"));
    }
}
