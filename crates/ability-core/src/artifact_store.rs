use crate::{BackupRunBinding, TargetKind};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

pub fn canonical_windows_zip_name(name: &str) -> Option<String> {
    if name.is_empty()
        || name.starts_with(['/', '\\'])
        || name.contains('\\')
        || name.contains(':')
        || name.chars().any(char::is_control)
    {
        return None;
    }
    let mut canonical = Vec::new();
    for component in name.split('/') {
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with(['.', ' '])
        {
            return None;
        }
        let basename = component
            .split_once('.')
            .map_or(component, |(basename, _)| basename);
        let device = basename.to_ascii_uppercase();
        if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || device.strip_prefix("COM").is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
            || device.strip_prefix("LPT").is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        {
            return None;
        }
        canonical.push(component.to_lowercase());
    }
    Some(canonical.join("/"))
}

#[derive(Debug, Error)]
pub enum ArtifactStoreError {
    #[error("artifact layout is unsafe")]
    UnsafeLayout,
    #[error("artifact tree contains an unexpected entry")]
    UnexpectedEntry,
    #[error("artifact deletion is supported only by the Windows desktop build")]
    UnsupportedPlatform,
    #[error("artifact deletion failed")]
    Io(#[source] std::io::Error),
}

pub struct ArtifactStore {
    root: PathBuf,
}

pub struct ArtifactBackupFile {
    zip_name: String,
    file: File,
}

impl ArtifactBackupFile {
    pub fn zip_name(&self) -> &str {
        &self.zip_name
    }
}

impl Read for ArtifactBackupFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.file.read(buffer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryArtifactCheckpoint {
    pub task_id: String,
    pub raw_artifact: bool,
}

impl ArtifactStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn quarantine_run_artifacts(
        &self,
        quarantine_token: Uuid,
        run_id: Uuid,
    ) -> Result<bool, ArtifactStoreError> {
        let source = self.root.join("runs").join(run_id.to_string());
        let quarantine_root = self
            .root
            .join(".delete-quarantine")
            .join(quarantine_token.to_string());
        let destination = quarantine_root.join("runs").join(run_id.to_string());
        if destination.exists() {
            return if source.exists() {
                Err(ArtifactStoreError::UnexpectedEntry)
            } else {
                Ok(true)
            };
        }
        if !source.exists() {
            return Ok(false);
        }
        reject_reparse_or_non_directory(&source)?;
        create_private_directory_chain(&self.root, &quarantine_root.join("runs"))?;
        fs::rename(&source, &destination).map_err(ArtifactStoreError::Io)?;
        Ok(true)
    }

    pub fn restore_quarantined_run_artifacts(
        &self,
        quarantine_token: Uuid,
        run_id: Uuid,
    ) -> Result<(), ArtifactStoreError> {
        let source = self
            .root
            .join(".delete-quarantine")
            .join(quarantine_token.to_string())
            .join("runs")
            .join(run_id.to_string());
        let destination = self.root.join("runs").join(run_id.to_string());
        if !source.exists() {
            return Ok(());
        }
        if destination.exists() {
            return Err(ArtifactStoreError::UnexpectedEntry);
        }
        reject_reparse_or_non_directory(&source)?;
        create_private_directory_chain(&self.root, &self.root.join("runs"))?;
        fs::rename(&source, &destination).map_err(ArtifactStoreError::Io)?;
        cleanup_quarantine_parents(&self.root, quarantine_token);
        Ok(())
    }

    pub fn delete_quarantined_run_artifacts(
        &self,
        quarantine_token: Uuid,
        run_id: Uuid,
        target: TargetKind,
    ) -> Result<bool, ArtifactStoreError> {
        let quarantine_root = self
            .root
            .join(".delete-quarantine")
            .join(quarantine_token.to_string());
        let quarantine = Self::new(quarantine_root);
        let removed = quarantine.delete_run_artifacts(run_id, target)?;
        cleanup_quarantine_parents(&self.root, quarantine_token);
        Ok(removed)
    }

    #[cfg(windows)]
    pub fn delete_run_artifacts(
        &self,
        run_id: Uuid,
        target: TargetKind,
    ) -> Result<bool, ArtifactStoreError> {
        windows::delete_run(&self.root, run_id, target)
    }

    #[cfg(windows)]
    pub fn prepare_recovery_artifacts(
        &self,
        run_id: Uuid,
        target: TargetKind,
        pack_task_ids: &[String],
        checkpoints: &[RecoveryArtifactCheckpoint],
    ) -> Result<(), ArtifactStoreError> {
        windows::prepare_recovery(&self.root, run_id, target, pack_task_ids, checkpoints)
    }

    #[cfg(windows)]
    pub fn open_backup_files(
        &self,
        expected_runs: &[BackupRunBinding],
    ) -> Result<Vec<ArtifactBackupFile>, ArtifactStoreError> {
        windows::open_backup_files(&self.root, expected_runs)
    }

    #[cfg(not(windows))]
    pub fn delete_run_artifacts(
        &self,
        _run_id: Uuid,
        _target: TargetKind,
    ) -> Result<bool, ArtifactStoreError> {
        Err(ArtifactStoreError::UnsupportedPlatform)
    }

    #[cfg(not(windows))]
    pub fn prepare_recovery_artifacts(
        &self,
        _run_id: Uuid,
        _target: TargetKind,
        _pack_task_ids: &[String],
        _checkpoints: &[RecoveryArtifactCheckpoint],
    ) -> Result<(), ArtifactStoreError> {
        Err(ArtifactStoreError::UnsupportedPlatform)
    }

    #[cfg(not(windows))]
    pub fn open_backup_files(
        &self,
        _expected_runs: &[BackupRunBinding],
    ) -> Result<Vec<ArtifactBackupFile>, ArtifactStoreError> {
        Err(ArtifactStoreError::UnsupportedPlatform)
    }
}

fn reject_reparse_or_non_directory(path: &std::path::Path) -> Result<(), ArtifactStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(ArtifactStoreError::Io)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ArtifactStoreError::UnsafeLayout);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ArtifactStoreError::UnsafeLayout);
        }
    }
    Ok(())
}

fn create_private_directory_chain(
    trusted_root: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), ArtifactStoreError> {
    if !destination.starts_with(trusted_root) {
        return Err(ArtifactStoreError::UnsafeLayout);
    }
    let mut current = trusted_root.to_path_buf();
    reject_reparse_or_non_directory(&current)?;
    for component in destination
        .strip_prefix(trusted_root)
        .map_err(|_| ArtifactStoreError::UnsafeLayout)?
        .components()
    {
        current.push(component);
        if current.exists() {
            reject_reparse_or_non_directory(&current)?;
        } else {
            fs::create_dir(&current).map_err(ArtifactStoreError::Io)?;
            reject_reparse_or_non_directory(&current)?;
        }
    }
    Ok(())
}

fn cleanup_quarantine_parents(root: &std::path::Path, quarantine_token: Uuid) {
    let token = root
        .join(".delete-quarantine")
        .join(quarantine_token.to_string());
    let _ = fs::remove_dir(token.join("runs"));
    let _ = fs::remove_dir(&token);
    let _ = fs::remove_dir(root.join(".delete-quarantine"));
}

#[cfg(windows)]
mod windows {
    use super::{
        ArtifactBackupFile, ArtifactStoreError, RecoveryArtifactCheckpoint,
        canonical_windows_zip_name,
    };
    use crate::{BackupRunBinding, TargetKind};
    use std::collections::{HashMap, HashSet};
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io;
    use std::mem::{offset_of, size_of, zeroed};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::ptr::{null, null_mut};
    use uuid::Uuid;
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_DIRECTORY_FILE, FILE_DISPOSITION_INFORMATION, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
        FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, FileDispositionInformation,
        NtCreateFile, NtSetInformationFile,
    };
    use windows_sys::Win32::Foundation::{
        ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError,
        STATUS_SUCCESS, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, DELETE, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY,
        FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE,
        FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetDriveTypeW,
        GetFileInformationByHandle, GetFileInformationByHandleEx, OPEN_EXISTING, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_CDROM, DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE,
        DRIVE_UNKNOWN,
    };

    const DIRECTORY_ACCESS: u32 =
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE;
    const DELETABLE_DIRECTORY_ACCESS: u32 = DIRECTORY_ACCESS | DELETE;
    const DELETABLE_FILE_ACCESS: u32 = FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE;
    const BACKUP_FILE_ACCESS: u32 = FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
    const SAFE_SHARING: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;

    #[derive(Clone, Copy)]
    enum TreePolicy {
        ManualRoot,
        CliRoot,
        Logs,
        Workspaces,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum HandleKind {
        File,
        Directory,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum SourceDrive {
        Allowed,
        Denied,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct VolumeAuthority {
        drive: u8,
        volume_serial_number: u32,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct HandleSnapshot {
        attributes: u32,
        volume_serial_number: u32,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    struct FileIdentity {
        volume_serial_number: u32,
        file_index: u64,
    }

    enum OpenedChild {
        File(File),
        Directory(File),
    }

    impl OpenedChild {
        fn kind(&self) -> HandleKind {
            match self {
                Self::File(_) => HandleKind::File,
                Self::Directory(_) => HandleKind::Directory,
            }
        }
    }

    pub fn delete_run(
        root: &Path,
        run_id: Uuid,
        target: TargetKind,
    ) -> Result<bool, ArtifactStoreError> {
        delete_run_inner(root, run_id, root_policy(target)?, || {})
    }

    fn delete_run_inner<F>(
        root: &Path,
        run_id: Uuid,
        policy: TreePolicy,
        after_preflight: F,
    ) -> Result<bool, ArtifactStoreError>
    where
        F: FnOnce(),
    {
        let (drive, components) = local_drive_components(root)?;
        let Some((root_handle, authority)) = open_existing_chain(drive, &components)? else {
            return Ok(false);
        };
        let Some(runs_handle) = open_optional_directory(
            &root_handle,
            OsStr::new("runs"),
            DIRECTORY_ACCESS,
            authority,
        )?
        else {
            return Ok(false);
        };
        let run_name = run_id.to_string();
        let Some(run_handle) = open_optional_directory(
            &runs_handle,
            OsStr::new(&run_name),
            DELETABLE_DIRECTORY_ACCESS,
            authority,
        )?
        else {
            return Ok(false);
        };
        preflight_tree(&run_handle, policy, authority)?;
        after_preflight();
        delete_tree(&run_handle, policy, authority)?;
        delete_handle(&run_handle).map_err(map_io)?;
        drop(run_handle);
        drop(runs_handle);
        drop(root_handle);
        Ok(true)
    }

    pub fn prepare_recovery(
        root: &Path,
        run_id: Uuid,
        target: TargetKind,
        pack_task_ids: &[String],
        checkpoints: &[RecoveryArtifactCheckpoint],
    ) -> Result<(), ArtifactStoreError> {
        let mut pack_ids = HashSet::with_capacity(pack_task_ids.len());
        for task_id in pack_task_ids {
            validate_name(OsStr::new(task_id))?;
            if !pack_ids.insert(task_id.as_str()) {
                return Err(ArtifactStoreError::UnexpectedEntry);
            }
        }

        let mut completed = HashMap::with_capacity(checkpoints.len());
        for checkpoint in checkpoints {
            validate_name(OsStr::new(&checkpoint.task_id))?;
            if !pack_ids.contains(checkpoint.task_id.as_str())
                || completed
                    .insert(checkpoint.task_id.as_str(), checkpoint.raw_artifact)
                    .is_some()
            {
                return Err(ArtifactStoreError::UnexpectedEntry);
            }
        }

        let (drive, components) = local_drive_components(root)?;
        let Some((root_handle, authority)) = open_existing_chain(drive, &components)? else {
            return Ok(());
        };
        let Some(runs_handle) = open_optional_directory(
            &root_handle,
            OsStr::new("runs"),
            DIRECTORY_ACCESS,
            authority,
        )?
        else {
            return Ok(());
        };
        let run_name = run_id.to_string();
        let Some(run_handle) = open_optional_directory(
            &runs_handle,
            OsStr::new(&run_name),
            DIRECTORY_ACCESS,
            authority,
        )?
        else {
            return Ok(());
        };
        match root_policy(target)? {
            TreePolicy::ManualRoot => {
                reconcile_manual(&run_handle, &pack_ids, &completed, authority)
            }
            TreePolicy::CliRoot => reconcile_cli(&run_handle, &pack_ids, &completed, authority),
            TreePolicy::Logs | TreePolicy::Workspaces => unreachable!("target root policy"),
        }
    }

    pub fn open_backup_files(
        root: &Path,
        expected_runs: &[BackupRunBinding],
    ) -> Result<Vec<ArtifactBackupFile>, ArtifactStoreError> {
        let mut expected = HashMap::with_capacity(expected_runs.len());
        for binding in expected_runs {
            if expected.insert(binding.id, binding.target).is_some() {
                return Err(ArtifactStoreError::UnexpectedEntry);
            }
        }
        let (drive, components) = local_drive_components(root)?;
        let Some((root_handle, authority)) = open_existing_chain(drive, &components)? else {
            return Ok(Vec::new());
        };
        for name in list_safe_directory(&root_handle)? {
            validate_name(&name)?;
            if name != OsStr::new("runs") {
                return Err(ArtifactStoreError::UnexpectedEntry);
            }
        }
        let Some(runs_handle) = open_optional_directory(
            &root_handle,
            OsStr::new("runs"),
            DIRECTORY_ACCESS,
            authority,
        )?
        else {
            return Ok(Vec::new());
        };

        let mut files = Vec::new();
        let mut zip_names = HashSet::new();
        let mut file_identities = HashSet::new();
        for name in list_safe_directory(&runs_handle)? {
            validate_name(&name)?;
            let value = name.to_str().ok_or(ArtifactStoreError::UnexpectedEntry)?;
            let run_id = Uuid::parse_str(value).map_err(|_| ArtifactStoreError::UnexpectedEntry)?;
            if run_id.to_string() != value {
                return Err(ArtifactStoreError::UnexpectedEntry);
            }
            let target = expected
                .get(&run_id)
                .copied()
                .ok_or(ArtifactStoreError::UnexpectedEntry)?;
            let run_handle =
                open_optional_directory(&runs_handle, &name, DIRECTORY_ACCESS, authority)?
                    .ok_or(ArtifactStoreError::UnexpectedEntry)?;
            let mut components = vec!["artifacts".to_owned(), "runs".to_owned(), value.to_owned()];
            collect_backup_tree(
                &run_handle,
                root_policy(target)?,
                &mut components,
                &mut zip_names,
                &mut file_identities,
                &mut files,
                authority,
            )?;
        }
        files.sort_by(|left, right| left.zip_name.cmp(&right.zip_name));
        Ok(files)
    }

    fn collect_backup_tree(
        directory: &File,
        policy: TreePolicy,
        components: &mut Vec<String>,
        zip_names: &mut HashSet<String>,
        file_identities: &mut HashSet<FileIdentity>,
        files: &mut Vec<ArtifactBackupFile>,
        authority: VolumeAuthority,
    ) -> Result<(), ArtifactStoreError> {
        for name in list_safe_directory(directory)? {
            validate_name(&name)?;
            let value = name
                .to_str()
                .ok_or(ArtifactStoreError::UnexpectedEntry)?
                .to_owned();
            let child = open_child_for_backup(directory, &name, authority)?;
            let child_policy = classify(policy, &name, child.kind())?;
            components.push(value);
            match child {
                OpenedChild::Directory(child) => {
                    collect_backup_tree(
                        &child,
                        child_policy,
                        components,
                        zip_names,
                        file_identities,
                        files,
                        authority,
                    )?;
                }
                OpenedChild::File(file) => {
                    let zip_name = components.join("/");
                    let canonical_zip_name = canonical_windows_zip_name(&zip_name)
                        .ok_or(ArtifactStoreError::UnexpectedEntry)?;
                    let identity = file_identity(&file, authority).map_err(map_io)?;
                    if !zip_names.insert(canonical_zip_name) || !file_identities.insert(identity) {
                        return Err(ArtifactStoreError::UnexpectedEntry);
                    }
                    files.push(ArtifactBackupFile { zip_name, file });
                }
            }
            components.pop();
        }
        Ok(())
    }

    #[cfg(test)]
    fn delete_run_with_after_preflight_hook<F>(
        root: &Path,
        run_id: Uuid,
        target: TargetKind,
        hook: F,
    ) -> Result<bool, ArtifactStoreError>
    where
        F: FnOnce(),
    {
        delete_run_inner(root, run_id, root_policy(target)?, hook)
    }

    fn preflight_tree(
        directory: &File,
        policy: TreePolicy,
        authority: VolumeAuthority,
    ) -> Result<(), ArtifactStoreError> {
        for name in list_safe_directory(directory)? {
            validate_name(&name)?;
            let child = open_child(directory, &name, false, authority)?;
            let child_policy = classify(policy, &name, child.kind())?;
            if let OpenedChild::Directory(child) = child {
                preflight_tree(&child, child_policy, authority)?;
            }
        }
        Ok(())
    }

    fn delete_tree(
        directory: &File,
        policy: TreePolicy,
        authority: VolumeAuthority,
    ) -> Result<(), ArtifactStoreError> {
        for name in list_safe_directory(directory)? {
            validate_name(&name)?;
            let child = open_child(directory, &name, true, authority)?;
            let child_policy = classify(policy, &name, child.kind())?;
            match child {
                OpenedChild::Directory(child) => {
                    delete_tree(&child, child_policy, authority)?;
                    delete_handle(&child).map_err(map_io)?;
                }
                OpenedChild::File(child) => delete_handle(&child).map_err(map_io)?,
            }
        }
        Ok(())
    }

    fn classify(
        policy: TreePolicy,
        name: &OsStr,
        kind: HandleKind,
    ) -> Result<TreePolicy, ArtifactStoreError> {
        let name = name.to_str().ok_or(ArtifactStoreError::UnexpectedEntry)?;
        match policy {
            TreePolicy::ManualRoot
                if kind == HandleKind::File
                    && (name.ends_with(".txt") || name.ends_with(".tmp")) =>
            {
                Ok(TreePolicy::ManualRoot)
            }
            TreePolicy::CliRoot if kind == HandleKind::Directory && name == "logs" => {
                Ok(TreePolicy::Logs)
            }
            TreePolicy::CliRoot if kind == HandleKind::Directory && name == "workspaces" => {
                Ok(TreePolicy::Workspaces)
            }
            TreePolicy::Logs if kind == HandleKind::File && name.ends_with(".log") => {
                Ok(TreePolicy::Logs)
            }
            TreePolicy::Workspaces => Ok(TreePolicy::Workspaces),
            _ => Err(ArtifactStoreError::UnexpectedEntry),
        }
    }

    fn root_policy(target: TargetKind) -> Result<TreePolicy, ArtifactStoreError> {
        Ok(match target {
            TargetKind::ChatGptClient | TargetKind::ClaudeClient => TreePolicy::ManualRoot,
            TargetKind::CodexCli | TargetKind::ClaudeCode => TreePolicy::CliRoot,
        })
    }

    fn reconcile_manual(
        directory: &File,
        pack_ids: &HashSet<&str>,
        completed: &HashMap<&str, bool>,
        authority: VolumeAuthority,
    ) -> Result<(), ArtifactStoreError> {
        for name in list_safe_directory(directory)? {
            validate_name(&name)?;
            let value = name.to_str().ok_or(ArtifactStoreError::UnexpectedEntry)?;
            let (task_id, preserve) = if let Some(task_id) = value.strip_suffix(".txt") {
                if !pack_ids.contains(task_id) {
                    return Err(ArtifactStoreError::UnexpectedEntry);
                }
                (task_id, completed.contains_key(task_id))
            } else {
                let Some(temporary) = value
                    .strip_prefix('.')
                    .and_then(|value| value.strip_suffix(".tmp"))
                else {
                    return Err(ArtifactStoreError::UnexpectedEntry);
                };
                let Some((task_id, nonce)) = temporary.rsplit_once('.') else {
                    return Err(ArtifactStoreError::UnexpectedEntry);
                };
                if !pack_ids.contains(task_id) || Uuid::parse_str(nonce).is_err() {
                    return Err(ArtifactStoreError::UnexpectedEntry);
                }
                (task_id, false)
            };
            let _ = task_id;
            match open_child(directory, &name, !preserve, authority)? {
                OpenedChild::File(child) if !preserve => delete_handle(&child).map_err(map_io)?,
                OpenedChild::File(_) => {}
                OpenedChild::Directory(_) => return Err(ArtifactStoreError::UnexpectedEntry),
            }
        }
        Ok(())
    }

    fn reconcile_cli(
        directory: &File,
        pack_ids: &HashSet<&str>,
        completed: &HashMap<&str, bool>,
        authority: VolumeAuthority,
    ) -> Result<(), ArtifactStoreError> {
        for name in list_safe_directory(directory)? {
            validate_name(&name)?;
            let value = name.to_str().ok_or(ArtifactStoreError::UnexpectedEntry)?;
            match (value, open_child(directory, &name, false, authority)?) {
                ("logs", OpenedChild::Directory(logs)) => {
                    reconcile_logs(&logs, pack_ids, completed, authority)?;
                }
                ("workspaces", OpenedChild::Directory(workspaces)) => {
                    reconcile_workspaces(&workspaces, pack_ids, completed, authority)?;
                }
                _ => return Err(ArtifactStoreError::UnexpectedEntry),
            }
        }
        Ok(())
    }

    fn reconcile_logs(
        directory: &File,
        pack_ids: &HashSet<&str>,
        completed: &HashMap<&str, bool>,
        authority: VolumeAuthority,
    ) -> Result<(), ArtifactStoreError> {
        for name in list_safe_directory(directory)? {
            validate_name(&name)?;
            let value = name.to_str().ok_or(ArtifactStoreError::UnexpectedEntry)?;
            let Some(task_id) = value.strip_suffix(".log") else {
                return Err(ArtifactStoreError::UnexpectedEntry);
            };
            if !pack_ids.contains(task_id) {
                return Err(ArtifactStoreError::UnexpectedEntry);
            }
            match completed.get(task_id) {
                Some(true) => match open_child(directory, &name, false, authority)? {
                    OpenedChild::File(_) => {}
                    OpenedChild::Directory(_) => {
                        return Err(ArtifactStoreError::UnexpectedEntry);
                    }
                },
                Some(false) => return Err(ArtifactStoreError::UnexpectedEntry),
                None => match open_child(directory, &name, true, authority)? {
                    OpenedChild::File(child) => delete_handle(&child).map_err(map_io)?,
                    OpenedChild::Directory(_) => {
                        return Err(ArtifactStoreError::UnexpectedEntry);
                    }
                },
            }
        }
        Ok(())
    }

    fn reconcile_workspaces(
        directory: &File,
        pack_ids: &HashSet<&str>,
        completed: &HashMap<&str, bool>,
        authority: VolumeAuthority,
    ) -> Result<(), ArtifactStoreError> {
        for name in list_safe_directory(directory)? {
            validate_name(&name)?;
            let task_id = name.to_str().ok_or(ArtifactStoreError::UnexpectedEntry)?;
            if !pack_ids.contains(task_id) {
                return Err(ArtifactStoreError::UnexpectedEntry);
            }
            let deleting = !completed.contains_key(task_id);
            if let OpenedChild::Directory(child) =
                open_child(directory, &name, deleting, authority)?
            {
                if deleting {
                    delete_tree(&child, TreePolicy::Workspaces, authority)?;
                    delete_handle(&child).map_err(map_io)?;
                } else {
                    preflight_tree(&child, TreePolicy::Workspaces, authority)?;
                }
            } else {
                return Err(ArtifactStoreError::UnexpectedEntry);
            }
        }
        Ok(())
    }

    fn validate_name(name: &OsStr) -> Result<(), ArtifactStoreError> {
        let value = name.to_str().ok_or(ArtifactStoreError::UnexpectedEntry)?;
        if value.is_empty()
            || matches!(value, "." | "..")
            || value.contains(['/', '\\', ':'])
            || value.chars().any(char::is_control)
            || canonical_windows_zip_name(value).is_none()
        {
            return Err(ArtifactStoreError::UnexpectedEntry);
        }
        native_component_length(name).map_err(map_io)?;
        Ok(())
    }

    fn local_drive_components(path: &Path) -> Result<(u8, Vec<OsString>), ArtifactStoreError> {
        if has_dot_component(path) {
            return Err(ArtifactStoreError::UnsafeLayout);
        }
        let mut parts = path.components();
        let drive = match parts.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) => letter,
                _ => return Err(ArtifactStoreError::UnsafeLayout),
            },
            _ => return Err(ArtifactStoreError::UnsafeLayout),
        };
        if !matches!(parts.next(), Some(Component::RootDir)) {
            return Err(ArtifactStoreError::UnsafeLayout);
        }
        let mut components = Vec::new();
        for part in parts {
            let Component::Normal(value) = part else {
                return Err(ArtifactStoreError::UnsafeLayout);
            };
            if value.is_empty() || value.to_string_lossy().contains(':') {
                return Err(ArtifactStoreError::UnsafeLayout);
            }
            native_component_length(value).map_err(map_io)?;
            components.push(value.to_os_string());
        }
        if components.is_empty() {
            return Err(ArtifactStoreError::UnsafeLayout);
        }
        Ok((drive, components))
    }

    fn has_dot_component(path: &Path) -> bool {
        let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let mut start = 0;
        for end in 0..=units.len() {
            if end != units.len() && !matches!(units[end], 47 | 92) {
                continue;
            }
            let part = &units[start..end];
            if part == [46] || part == [46, 46] {
                return true;
            }
            start = end + 1;
        }
        false
    }

    fn open_existing_chain(
        drive: u8,
        components: &[OsString],
    ) -> Result<Option<(File, VolumeAuthority)>, ArtifactStoreError> {
        require_allowed_source_drive(drive)?;
        let mut current = open_drive_root(drive).map_err(map_io)?;
        let root = inspect_handle(&current).map_err(map_io)?;
        let authority = VolumeAuthority {
            drive: drive.to_ascii_uppercase(),
            volume_serial_number: root.volume_serial_number,
        };
        validate_handle_snapshot(root, authority).map_err(map_io)?;
        for component in components {
            let Some(next) =
                open_optional_directory(&current, component, DIRECTORY_ACCESS, authority)?
            else {
                return Ok(None);
            };
            current = next;
        }
        Ok(Some((current, authority)))
    }

    fn open_drive_root(drive: u8) -> io::Result<File> {
        let root = PathBuf::from(format!("{}:\\", char::from(drive)));
        let wide = wide(root.as_os_str());
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                DIRECTORY_ACCESS,
                SAFE_SHARING,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_handle(handle as RawHandle) };
        Ok(file)
    }

    fn classify_source_drive(raw: u32) -> SourceDrive {
        match raw {
            DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_RAMDISK => SourceDrive::Allowed,
            DRIVE_REMOTE | DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR | DRIVE_CDROM => SourceDrive::Denied,
            _ => SourceDrive::Denied,
        }
    }

    fn require_allowed_source_drive(drive: u8) -> Result<(), ArtifactStoreError> {
        let root = PathBuf::from(format!("{}:\\", char::from(drive)));
        let wide = wide(root.as_os_str());
        let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
        if classify_source_drive(drive_type) == SourceDrive::Allowed {
            Ok(())
        } else {
            Err(ArtifactStoreError::UnsafeLayout)
        }
    }

    fn open_optional_directory(
        parent: &File,
        name: &OsStr,
        access: u32,
        authority: VolumeAuthority,
    ) -> Result<Option<File>, ArtifactStoreError> {
        match open_relative(
            parent,
            name,
            access,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            SAFE_SHARING,
        ) {
            Ok(file) => {
                let snapshot = inspect_handle(&file).map_err(map_io)?;
                validate_handle_snapshot(snapshot, authority).map_err(map_io)?;
                if handle_kind(snapshot) != HandleKind::Directory {
                    return Err(ArtifactStoreError::UnexpectedEntry);
                }
                Ok(Some(file))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(map_io(error)),
        }
    }

    fn open_child(
        parent: &File,
        name: &OsStr,
        deletable: bool,
        authority: VolumeAuthority,
    ) -> Result<OpenedChild, ArtifactStoreError> {
        let directory_access = if deletable {
            DELETABLE_DIRECTORY_ACCESS
        } else {
            DIRECTORY_ACCESS
        };
        match open_relative(
            parent,
            name,
            directory_access,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            SAFE_SHARING,
        ) {
            Ok(file) => {
                let snapshot = inspect_handle(&file).map_err(map_io)?;
                validate_handle_snapshot(snapshot, authority).map_err(map_io)?;
                if handle_kind(snapshot) != HandleKind::Directory {
                    return Err(ArtifactStoreError::UnexpectedEntry);
                }
                Ok(OpenedChild::Directory(file))
            }
            Err(_) => {
                let access = if deletable {
                    DELETABLE_FILE_ACCESS
                } else {
                    FILE_READ_ATTRIBUTES | SYNCHRONIZE
                };
                let file = open_relative(
                    parent,
                    name,
                    access,
                    FILE_NON_DIRECTORY_FILE
                        | FILE_OPEN_REPARSE_POINT
                        | FILE_SYNCHRONOUS_IO_NONALERT,
                    SAFE_SHARING,
                )
                .map_err(map_io)?;
                let snapshot = inspect_handle(&file).map_err(map_io)?;
                validate_handle_snapshot(snapshot, authority).map_err(map_io)?;
                if handle_kind(snapshot) != HandleKind::File {
                    return Err(ArtifactStoreError::UnexpectedEntry);
                }
                Ok(OpenedChild::File(file))
            }
        }
    }

    fn open_child_for_backup(
        parent: &File,
        name: &OsStr,
        authority: VolumeAuthority,
    ) -> Result<OpenedChild, ArtifactStoreError> {
        match open_relative(
            parent,
            name,
            DIRECTORY_ACCESS,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            SAFE_SHARING,
        ) {
            Ok(file) => {
                let snapshot = inspect_handle(&file).map_err(map_io)?;
                validate_handle_snapshot(snapshot, authority).map_err(map_io)?;
                if handle_kind(snapshot) != HandleKind::Directory {
                    return Err(ArtifactStoreError::UnexpectedEntry);
                }
                Ok(OpenedChild::Directory(file))
            }
            Err(_) => {
                let file = open_relative(
                    parent,
                    name,
                    BACKUP_FILE_ACCESS,
                    FILE_NON_DIRECTORY_FILE
                        | FILE_OPEN_REPARSE_POINT
                        | FILE_SYNCHRONOUS_IO_NONALERT,
                    SAFE_SHARING,
                )
                .map_err(map_io)?;
                let snapshot = inspect_handle(&file).map_err(map_io)?;
                validate_handle_snapshot(snapshot, authority).map_err(map_io)?;
                if handle_kind(snapshot) != HandleKind::File {
                    return Err(ArtifactStoreError::UnexpectedEntry);
                }
                Ok(OpenedChild::File(file))
            }
        }
    }

    fn list_directory(directory: &File) -> io::Result<Vec<OsString>> {
        const BUFFER_BYTES: usize = 64 * 1024;
        let mut names = Vec::new();
        let mut restart = true;
        loop {
            let mut buffer = vec![0_u64; BUFFER_BYTES / size_of::<u64>()];
            let class = if restart {
                FileIdBothDirectoryRestartInfo
            } else {
                FileIdBothDirectoryInfo
            };
            let success = unsafe {
                GetFileInformationByHandleEx(
                    directory.as_raw_handle() as _,
                    class,
                    buffer.as_mut_ptr().cast(),
                    BUFFER_BYTES as u32,
                )
            };
            if success == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                    break;
                }
                return Err(error);
            }
            restart = false;

            let bytes =
                unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), BUFFER_BYTES) };
            let mut offset = 0_usize;
            loop {
                let header_end = offset
                    .checked_add(offset_of!(FILE_ID_BOTH_DIR_INFO, FileName))
                    .ok_or_else(invalid_directory_information)?;
                if header_end > bytes.len() {
                    return Err(invalid_directory_information());
                }
                let information = unsafe {
                    std::ptr::read_unaligned(
                        bytes.as_ptr().add(offset).cast::<FILE_ID_BOTH_DIR_INFO>(),
                    )
                };
                let name_bytes = usize::try_from(information.FileNameLength)
                    .map_err(|_| invalid_directory_information())?;
                if name_bytes == 0 || name_bytes % size_of::<u16>() != 0 {
                    return Err(invalid_directory_information());
                }
                let name_end = header_end
                    .checked_add(name_bytes)
                    .ok_or_else(invalid_directory_information)?;
                if name_end > bytes.len() {
                    return Err(invalid_directory_information());
                }
                let units = unsafe {
                    std::slice::from_raw_parts(
                        bytes.as_ptr().add(header_end).cast::<u16>(),
                        name_bytes / size_of::<u16>(),
                    )
                };
                let name = OsString::from_wide(units);
                if !matches!(name.to_str(), Some(".") | Some("..")) {
                    names.push(name);
                }

                if information.NextEntryOffset == 0 {
                    break;
                }
                let next = usize::try_from(information.NextEntryOffset)
                    .map_err(|_| invalid_directory_information())?;
                if next < offset_of!(FILE_ID_BOTH_DIR_INFO, FileName) {
                    return Err(invalid_directory_information());
                }
                offset = offset
                    .checked_add(next)
                    .ok_or_else(invalid_directory_information)?;
                if offset >= bytes.len() {
                    return Err(invalid_directory_information());
                }
            }
        }
        Ok(names)
    }

    fn list_safe_directory(directory: &File) -> Result<Vec<OsString>, ArtifactStoreError> {
        let names = list_directory(directory).map_err(map_io)?;
        let mut canonical = HashSet::with_capacity(names.len());
        for name in &names {
            validate_name(name)?;
            let value = name.to_str().ok_or(ArtifactStoreError::UnexpectedEntry)?;
            let key =
                canonical_windows_zip_name(value).ok_or(ArtifactStoreError::UnexpectedEntry)?;
            if !canonical.insert(key) {
                return Err(ArtifactStoreError::UnexpectedEntry);
            }
        }
        Ok(names)
    }

    fn invalid_directory_information() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid directory information returned by Windows",
        )
    }

    fn open_relative(
        parent: &File,
        name: &OsStr,
        access: u32,
        options: u32,
        sharing: u32,
    ) -> io::Result<File> {
        let length = native_component_length(name)?;
        let mut storage = wide(name);
        let mut unicode = UNICODE_STRING {
            Length: length,
            MaximumLength: length,
            Buffer: storage.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle() as _,
            ObjectName: &mut unicode,
            Attributes: OBJ_CASE_INSENSITIVE,
            SecurityDescriptor: null(),
            SecurityQualityOfService: null(),
        };
        let mut handle = INVALID_HANDLE_VALUE;
        let mut status = unsafe { zeroed::<IO_STATUS_BLOCK>() };
        let result = unsafe {
            NtCreateFile(
                &mut handle,
                access,
                &attributes,
                &mut status,
                null(),
                FILE_ATTRIBUTE_NORMAL,
                sharing,
                FILE_OPEN,
                options,
                null(),
                0,
            )
        };
        if result != STATUS_SUCCESS {
            return Err(nt_error(result));
        }
        Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
    }

    fn inspect_handle(file: &File) -> io::Result<HandleSnapshot> {
        let mut information = unsafe { zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(HandleSnapshot {
            attributes: information.dwFileAttributes,
            volume_serial_number: information.dwVolumeSerialNumber,
        })
    }

    fn validate_handle_snapshot(
        snapshot: HandleSnapshot,
        authority: VolumeAuthority,
    ) -> io::Result<()> {
        let _ = authority.drive;
        if snapshot.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || snapshot.volume_serial_number != authority.volume_serial_number
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "reparse point"));
        }
        Ok(())
    }

    fn handle_kind(snapshot: HandleSnapshot) -> HandleKind {
        if snapshot.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            HandleKind::Directory
        } else {
            HandleKind::File
        }
    }

    fn file_identity(file: &File, authority: VolumeAuthority) -> io::Result<FileIdentity> {
        let mut information = unsafe { zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } == 0 {
            return Err(io::Error::last_os_error());
        }
        validate_handle_snapshot(
            HandleSnapshot {
                attributes: information.dwFileAttributes,
                volume_serial_number: information.dwVolumeSerialNumber,
            },
            authority,
        )?;
        if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "backup entry is not an ordinary file",
            ));
        }
        Ok(FileIdentity {
            volume_serial_number: information.dwVolumeSerialNumber,
            file_index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        })
    }

    fn delete_handle(file: &File) -> io::Result<()> {
        let information = FILE_DISPOSITION_INFORMATION { DeleteFile: true };
        let mut status = unsafe { zeroed::<IO_STATUS_BLOCK>() };
        let result = unsafe {
            NtSetInformationFile(
                file.as_raw_handle() as _,
                &mut status,
                (&information as *const FILE_DISPOSITION_INFORMATION).cast(),
                size_of::<FILE_DISPOSITION_INFORMATION>() as u32,
                FileDispositionInformation,
            )
        };
        if result == STATUS_SUCCESS {
            Ok(())
        } else {
            Err(nt_error(result))
        }
    }

    fn native_component_length(value: &OsStr) -> io::Result<u16> {
        let units = value.encode_wide().count();
        if units == 0 || units > 255 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid component length",
            ));
        }
        u16::try_from(units * size_of::<u16>())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "component overflow"))
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn nt_error(status: i32) -> io::Error {
        io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
    }

    fn map_io(error: io::Error) -> ArtifactStoreError {
        if error.kind() == io::ErrorKind::InvalidInput {
            ArtifactStoreError::UnsafeLayout
        } else {
            ArtifactStoreError::Io(error)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;
        use std::process::Command;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use tempfile::tempdir;

        #[test]
        fn source_drive_allowlist_accepts_only_fixed_removable_and_ramdisk() {
            assert_eq!(classify_source_drive(DRIVE_FIXED), SourceDrive::Allowed);
            assert_eq!(classify_source_drive(DRIVE_REMOVABLE), SourceDrive::Allowed);
            assert_eq!(classify_source_drive(DRIVE_RAMDISK), SourceDrive::Allowed);
            for denied in [
                DRIVE_REMOTE,
                DRIVE_UNKNOWN,
                DRIVE_NO_ROOT_DIR,
                DRIVE_CDROM,
                u32::MAX,
            ] {
                assert_eq!(classify_source_drive(denied), SourceDrive::Denied);
            }
        }

        #[test]
        fn source_handle_authority_rejects_reparse_and_cross_volume_handles() {
            let authority = VolumeAuthority {
                drive: b'C',
                volume_serial_number: 91,
            };
            assert!(
                validate_handle_snapshot(
                    HandleSnapshot {
                        attributes: FILE_ATTRIBUTE_NORMAL,
                        volume_serial_number: 91,
                    },
                    authority,
                )
                .is_ok()
            );
            assert!(
                validate_handle_snapshot(
                    HandleSnapshot {
                        attributes: FILE_ATTRIBUTE_REPARSE_POINT,
                        volume_serial_number: 91,
                    },
                    authority,
                )
                .is_err()
            );
            assert!(
                validate_handle_snapshot(
                    HandleSnapshot {
                        attributes: FILE_ATTRIBUTE_NORMAL,
                        volume_serial_number: 92,
                    },
                    authority,
                )
                .is_err()
            );
        }

        #[test]
        fn windows_zip_component_key_rejects_device_and_trailing_names_and_folds_case() {
            assert!(canonical_windows_zip_name("artifacts/runs/id/CON.txt").is_none());
            assert!(canonical_windows_zip_name("artifacts/runs/id/answer.txt.").is_none());
            assert!(canonical_windows_zip_name("artifacts/runs/id/answer.txt ").is_none());
            assert_eq!(
                canonical_windows_zip_name("artifacts/runs/id/Answer.TXT"),
                canonical_windows_zip_name("artifacts/runs/id/answer.txt")
            );
        }

        #[test]
        fn retained_run_handle_blocks_a_preflight_to_delete_junction_swap() {
            let directory = tempdir().unwrap();
            let root = directory.path().join("artifacts");
            let run_id = Uuid::new_v4();
            let run = root.join("runs").join(run_id.to_string());
            fs::create_dir_all(&run).unwrap();
            fs::write(run.join("answer.txt"), "raw").unwrap();
            let detached = directory.path().join("detached");
            let outside = tempdir().unwrap();
            let outside_path = outside.path().to_path_buf();
            let swapped = Arc::new(AtomicBool::new(false));
            let observed = swapped.clone();

            delete_run_with_after_preflight_hook(
                &root,
                run_id,
                TargetKind::ChatGptClient,
                move || {
                    if fs::rename(&run, &detached).is_ok() {
                        let status = Command::new("cmd")
                            .args([
                                "/C",
                                "mklink",
                                "/J",
                                run.to_str().unwrap(),
                                outside_path.to_str().unwrap(),
                            ])
                            .status()
                            .unwrap();
                        assert!(status.success());
                        observed.store(true, Ordering::SeqCst);
                    }
                },
            )
            .unwrap();

            assert!(!swapped.load(Ordering::SeqCst));
            assert!(!outside.path().join("answer.txt").exists());
        }

        #[test]
        fn every_descendant_is_reopened_relative_and_revalidated_after_preflight() {
            let directory = tempdir().unwrap();
            let root = directory.path().join("artifacts");
            let run_id = Uuid::new_v4();
            let run = root.join("runs").join(run_id.to_string());
            let task = run.join("workspaces").join("task-one");
            fs::create_dir_all(&task).unwrap();
            fs::write(task.join("owned.txt"), "owned").unwrap();
            let detached = directory.path().join("detached-task");
            let outside = tempdir().unwrap();
            fs::write(outside.path().join("outside.txt"), "outside-owned").unwrap();
            let outside_path = outside.path().to_path_buf();

            let result = delete_run_with_after_preflight_hook(
                &root,
                run_id,
                TargetKind::CodexCli,
                move || {
                    fs::rename(&task, &detached).unwrap();
                    let status = Command::new("cmd")
                        .args([
                            "/C",
                            "mklink",
                            "/J",
                            task.to_str().unwrap(),
                            outside_path.to_str().unwrap(),
                        ])
                        .status()
                        .unwrap();
                    assert!(status.success());
                },
            );

            assert!(matches!(result, Err(ArtifactStoreError::UnsafeLayout)));
            assert_eq!(
                fs::read_to_string(outside.path().join("outside.txt")).unwrap(),
                "outside-owned"
            );
        }
    }
}
