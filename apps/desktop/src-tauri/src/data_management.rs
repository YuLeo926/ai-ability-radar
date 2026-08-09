use crate::app_state::RunOperationRegistry;
use ability_core::{
    canonical_windows_zip_name, ArtifactStore, ArtifactStoreError, BatchMemberStatus, BatchStatus,
    RunRepository, RunStatus, ScanDeletionIntent, ScanDeletionPhase, StorageError,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DataError {
    #[error("local storage operation failed")]
    Storage(#[from] StorageError),
    #[error("artifact operation failed")]
    Artifact(#[from] ArtifactStoreError),
    #[error("backup stream failed")]
    Io(#[from] io::Error),
    #[error("backup manifest failed")]
    Json(#[from] serde_json::Error),
    #[error("local data is busy")]
    Busy,
    #[error("backup layout is invalid")]
    InvalidLayout,
}

pub(crate) fn prune_expired_artifacts(
    repository: &RunRepository,
    store: &ArtifactStore,
    operations: &RunOperationRegistry,
    now: DateTime<Utc>,
) -> Result<u32, DataError> {
    let candidates = repository.retention_candidates(now)?;
    let _claims = operations
        .claim(candidates.iter().map(|candidate| candidate.id))
        .map_err(|_| DataError::Busy)?;
    let mut removed = 0_u32;
    for candidate in candidates {
        store.delete_run_artifacts(candidate.id, candidate.target.kind)?;
        repository.clear_retention_candidate(&candidate, now)?;
        removed = removed.checked_add(1).ok_or(DataError::InvalidLayout)?;
    }
    Ok(removed)
}

pub(crate) fn delete_batch_data(
    repository: &RunRepository,
    store: &ArtifactStore,
    operations: &RunOperationRegistry,
    batch_id: Uuid,
    delete_owned_runs: bool,
    now: DateTime<Utc>,
) -> Result<bool, DataError> {
    if !delete_owned_runs {
        return repository
            .unlink_batch(batch_id)
            .map_err(DataError::Storage);
    }
    let Some(batch) = repository.get_batch(batch_id)? else {
        return Ok(false);
    };
    if batch.status == BatchStatus::Running
        || batch.members.iter().any(|member| {
            matches!(
                member.status,
                BatchMemberStatus::Reserved
                    | BatchMemberStatus::Launching
                    | BatchMemberStatus::Running
            )
        })
    {
        return Err(DataError::Busy);
    }
    let mut bindings = Vec::new();
    for run_id in batch.members.iter().filter_map(|member| member.run_id) {
        let run = repository
            .get_run(run_id)?
            .ok_or(StorageError::RunNotFound(run_id))?;
        if run.status == RunStatus::Running {
            return Err(DataError::Busy);
        }
        bindings.push((run_id, run.target.kind));
    }
    bindings.sort_unstable_by_key(|binding| binding.0);
    bindings.dedup_by_key(|binding| binding.0);
    if bindings.is_empty() {
        return repository
            .delete_batch(batch_id)
            .map_err(DataError::Storage);
    }
    let _claim = operations
        .claim(bindings.iter().map(|binding| binding.0))
        .map_err(|_| DataError::Busy)?;
    let quarantine_token = Uuid::new_v4();
    let intents = bindings
        .iter()
        .map(|(run_id, target)| ScanDeletionIntent {
            id: Uuid::new_v4(),
            batch_id,
            run_id: *run_id,
            quarantine_token,
            target: *target,
            phase: ScanDeletionPhase::IntentRecorded,
            created_at: now,
            updated_at: now,
        })
        .collect::<Vec<_>>();
    for intent in &intents {
        repository.insert_scan_deletion_intent(intent)?;
    }
    for intent in &intents {
        store.quarantine_run_artifacts(intent.quarantine_token, intent.run_id)?;
        repository.update_scan_deletion_intent_phase(
            intent.id,
            ScanDeletionPhase::ArtifactsQuarantined,
            now,
        )?;
    }
    let run_ids = bindings.iter().map(|binding| binding.0).collect::<Vec<_>>();
    repository.delete_batch_with_owned_runs(batch_id, &run_ids)?;
    for intent in &intents {
        repository.update_scan_deletion_intent_phase(
            intent.id,
            ScanDeletionPhase::DatabaseCommitted,
            now,
        )?;
    }
    for intent in &intents {
        store.delete_quarantined_run_artifacts(
            intent.quarantine_token,
            intent.run_id,
            intent.target,
        )?;
        repository.delete_scan_deletion_intent(intent.id)?;
    }
    Ok(true)
}

pub(crate) fn reconcile_batch_deletions(
    repository: &RunRepository,
    store: &ArtifactStore,
) -> Result<u32, DataError> {
    let intents = repository.list_scan_deletion_intents()?;
    let mut reconciled = 0_u32;
    for intent in intents {
        if repository.get_batch(intent.batch_id)?.is_some() {
            store.restore_quarantined_run_artifacts(intent.quarantine_token, intent.run_id)?;
        } else {
            store.delete_quarantined_run_artifacts(
                intent.quarantine_token,
                intent.run_id,
                intent.target,
            )?;
        }
        repository.delete_scan_deletion_intent(intent.id)?;
        reconciled = reconciled.checked_add(1).ok_or(DataError::InvalidLayout)?;
    }
    Ok(reconciled)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest<'a> {
    schema_version: u32,
    created_at: &'a str,
    app_version: &'a str,
    contains_raw_answers_and_logs: bool,
    encrypted: bool,
}

pub(crate) fn create_full_backup<R: Read + Seek, W: Write>(
    repository: &RunRepository,
    store: &ArtifactStore,
    snapshot_path: &Path,
    snapshot_reader: &mut R,
    destination: &mut W,
    now: DateTime<Utc>,
    app_version: &str,
) -> Result<(), DataError> {
    let runs = repository.snapshot_to_backup_file(snapshot_path)?;
    snapshot_reader.seek(SeekFrom::Start(0))?;
    let artifacts = store.open_backup_files(&runs)?;
    let created_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    let manifest = serde_json::to_vec_pretty(&BackupManifest {
        schema_version: 1,
        created_at: &created_at,
        app_version,
        contains_raw_answers_and_logs: true,
        encrypted: false,
    })?;

    let mut archive = StreamingZip::new(destination);
    archive.add_reader("ability-radar.sqlite", snapshot_reader)?;
    archive.add_bytes("backup-manifest.json", &manifest)?;
    for mut artifact in artifacts {
        let name = artifact.zip_name().to_owned();
        archive.add_reader(&name, &mut artifact)?;
    }
    archive.finish()?;
    Ok(())
}

struct ZipEntry {
    name: Vec<u8>,
    crc32: u32,
    size: u32,
    local_offset: u32,
}

struct StreamingZip<'a, W> {
    writer: &'a mut W,
    position: u64,
    names: HashSet<String>,
    entries: Vec<ZipEntry>,
}

impl<'a, W: Write> StreamingZip<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            position: 0,
            names: HashSet::new(),
            entries: Vec::new(),
        }
    }

    fn add_bytes(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.add_entry(name, |entry| entry.write_all(bytes))
    }

    fn add_reader<R: Read>(&mut self, name: &str, reader: &mut R) -> io::Result<()> {
        self.add_entry(name, |entry| {
            io::copy(reader, entry)?;
            Ok(())
        })
    }

    fn add_entry<F>(&mut self, name: &str, produce: F) -> io::Result<()>
    where
        F: FnOnce(&mut ZipEntryWriter<'_, W>) -> io::Result<()>,
    {
        let canonical_name = validate_zip_name(name)?;
        if !self.names.insert(canonical_name) {
            return Err(invalid_zip());
        }
        let name = name.as_bytes().to_vec();
        let name_length = u16::try_from(name.len()).map_err(|_| invalid_zip())?;
        let local_offset = u32::try_from(self.position).map_err(|_| invalid_zip())?;
        self.write_u32(0x0403_4b50)?;
        self.write_u16(20)?;
        self.write_u16(0x0808)?;
        self.write_u16(0)?;
        self.write_u16(0)?;
        self.write_u16(0)?;
        self.write_u32(0)?;
        self.write_u32(0)?;
        self.write_u32(0)?;
        self.write_u16(name_length)?;
        self.write_u16(0)?;
        self.write_all(&name)?;

        let (crc32, size) = {
            let mut entry = ZipEntryWriter {
                writer: self.writer,
                position: &mut self.position,
                crc32: Crc32::new(),
                size: 0,
            };
            produce(&mut entry)?;
            (entry.crc32.finish(), entry.size)
        };
        let size = u32::try_from(size).map_err(|_| invalid_zip())?;
        self.write_u32(0x0807_4b50)?;
        self.write_u32(crc32)?;
        self.write_u32(size)?;
        self.write_u32(size)?;
        self.entries.push(ZipEntry {
            name,
            crc32,
            size,
            local_offset,
        });
        Ok(())
    }

    fn finish(mut self) -> io::Result<()> {
        let central_offset = u32::try_from(self.position).map_err(|_| invalid_zip())?;
        let entries = std::mem::take(&mut self.entries);
        for entry in &entries {
            let name_length = u16::try_from(entry.name.len()).map_err(|_| invalid_zip())?;
            self.write_u32(0x0201_4b50)?;
            self.write_u16(20)?;
            self.write_u16(20)?;
            self.write_u16(0x0808)?;
            self.write_u16(0)?;
            self.write_u16(0)?;
            self.write_u16(0)?;
            self.write_u32(entry.crc32)?;
            self.write_u32(entry.size)?;
            self.write_u32(entry.size)?;
            self.write_u16(name_length)?;
            self.write_u16(0)?;
            self.write_u16(0)?;
            self.write_u16(0)?;
            self.write_u16(0)?;
            self.write_u32(0)?;
            self.write_u32(entry.local_offset)?;
            self.write_all(&entry.name)?;
        }
        let central_size =
            u32::try_from(self.position - u64::from(central_offset)).map_err(|_| invalid_zip())?;
        let count = u16::try_from(entries.len()).map_err(|_| invalid_zip())?;
        self.write_u32(0x0605_4b50)?;
        self.write_u16(0)?;
        self.write_u16(0)?;
        self.write_u16(count)?;
        self.write_u16(count)?;
        self.write_u32(central_size)?;
        self.write_u32(central_offset)?;
        self.write_u16(0)?;
        self.writer.flush()
    }

    fn write_u16(&mut self, value: u16) -> io::Result<()> {
        self.write_all(&value.to_le_bytes())
    }

    fn write_u32(&mut self, value: u32) -> io::Result<()> {
        self.write_all(&value.to_le_bytes())
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.position = self
            .position
            .checked_add(bytes.len() as u64)
            .ok_or_else(invalid_zip)?;
        Ok(())
    }
}

struct ZipEntryWriter<'a, W> {
    writer: &'a mut W,
    position: &'a mut u64,
    crc32: Crc32,
    size: u64,
}

impl<W: Write> Write for ZipEntryWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.writer.write(bytes)?;
        let written_bytes = &bytes[..written];
        self.crc32.update(written_bytes);
        self.size = self
            .size
            .checked_add(written as u64)
            .ok_or_else(invalid_zip)?;
        *self.position = self
            .position
            .checked_add(written as u64)
            .ok_or_else(invalid_zip)?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

struct Crc32(u32);

impl Crc32 {
    fn new() -> Self {
        Self(u32::MAX)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0_u32.wrapping_sub(self.0 & 1);
                self.0 = (self.0 >> 1) ^ (0xedb8_8320 & mask);
            }
        }
    }

    fn finish(self) -> u32 {
        !self.0
    }
}

fn validate_zip_name(name: &str) -> io::Result<String> {
    if name.is_empty()
        || name.starts_with(['/', '\\'])
        || name.contains('\\')
        || name.contains(':')
        || name.chars().any(char::is_control)
        || name
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
    {
        return Err(invalid_zip());
    }
    let allowed_top_level = matches!(name, "ability-radar.sqlite" | "backup-manifest.json")
        || name.starts_with("artifacts/runs/");
    if !allowed_top_level {
        return Err(invalid_zip());
    }
    canonical_windows_zip_name(name).ok_or_else(invalid_zip)
}

fn invalid_zip() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid backup archive layout")
}

#[cfg(test)]
mod tests {
    use super::StreamingZip;

    #[test]
    fn streaming_zip_refuses_windows_case_collisions_before_writing_an_entry() {
        let mut bytes = Vec::new();
        let mut archive = StreamingZip::new(&mut bytes);
        archive
            .add_bytes("artifacts/runs/id/Answer.txt", b"first")
            .unwrap();
        let length_before_collision = archive.position;

        assert!(archive
            .add_bytes("artifacts/runs/id/answer.TXT", b"second")
            .is_err());
        assert_eq!(archive.position, length_before_collision);
    }
}
