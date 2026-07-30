use crate::{BatchContractError, BatchTaskSessionBinding, ScanBatchPlan};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledBatchMember {
    pub ordinal: u32,
    pub target_position: u32,
    pub repetition_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchSchedule {
    pub policy_version: u32,
    pub seed: u64,
    pub plan_acknowledgement_hash: String,
    pub task_session_binding: BatchTaskSessionBinding,
    pub members: Vec<ScheduledBatchMember>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledMemberLifecycle {
    Runnable,
    Deferred,
    Reserved,
    Launching,
    Running,
    Terminal,
}

impl ScheduledMemberLifecycle {
    fn is_active(self) -> bool {
        matches!(self, Self::Reserved | Self::Launching | Self::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledMemberState {
    pub ordinal: u32,
    pub lifecycle: ScheduledMemberLifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision", content = "member")]
pub enum NextScheduledMember {
    Runnable(ScheduledBatchMember),
    BlockedByActive { ordinal: u32 },
    Exhausted,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BatchScheduleError {
    #[error(transparent)]
    InvalidPlan(#[from] BatchContractError),
    #[error("batch schedule dimensions overflow supported ordinals")]
    ArithmeticOverflow,
    #[error("member state vector does not match the immutable schedule")]
    InvalidMemberStateVector,
    #[error("more than one batch member is active")]
    MultipleActiveMembers,
}

pub fn build_batch_schedule(plan: &ScanBatchPlan) -> Result<BatchSchedule, BatchScheduleError> {
    let (repetitions, task_session_binding) = plan.validated_schedule_contract()?;
    let target_count =
        u32::try_from(plan.targets.len()).map_err(|_| BatchScheduleError::ArithmeticOverflow)?;
    if target_count == 0 || repetitions == 0 {
        return Err(BatchScheduleError::InvalidMemberStateVector);
    }
    let target_count_u64 = u64::from(target_count);
    let seed_offset = u32::try_from(plan.seed % target_count_u64)
        .map_err(|_| BatchScheduleError::ArithmeticOverflow)?;
    let member_count = target_count
        .checked_mul(repetitions)
        .ok_or(BatchScheduleError::ArithmeticOverflow)?;
    let mut members = Vec::with_capacity(
        usize::try_from(member_count).map_err(|_| BatchScheduleError::ArithmeticOverflow)?,
    );
    for repetition_index in 0..repetitions {
        let start = (seed_offset + repetition_index) % target_count;
        for step in 0..target_count {
            let target_position = if repetition_index % 2 == 0 {
                (start + step) % target_count
            } else {
                (start + target_count - (step % target_count)) % target_count
            };
            let ordinal = repetition_index
                .checked_mul(target_count)
                .and_then(|value| value.checked_add(step))
                .ok_or(BatchScheduleError::ArithmeticOverflow)?;
            members.push(ScheduledBatchMember {
                ordinal,
                target_position,
                repetition_index,
            });
        }
    }
    Ok(BatchSchedule {
        policy_version: plan.schedule_policy_version,
        seed: plan.seed,
        plan_acknowledgement_hash: plan.acknowledgement_hash.clone(),
        task_session_binding,
        members,
    })
}

pub fn select_next_scheduled_member(
    schedule: &BatchSchedule,
    states: &[ScheduledMemberState],
) -> Result<NextScheduledMember, BatchScheduleError> {
    if states.len() != schedule.members.len()
        || states
            .iter()
            .zip(&schedule.members)
            .any(|(state, member)| state.ordinal != member.ordinal)
    {
        return Err(BatchScheduleError::InvalidMemberStateVector);
    }
    let mut active = states
        .iter()
        .filter(|state| state.lifecycle.is_active())
        .map(|state| state.ordinal);
    if let Some(ordinal) = active.next() {
        if active.next().is_some() {
            return Err(BatchScheduleError::MultipleActiveMembers);
        }
        return Ok(NextScheduledMember::BlockedByActive { ordinal });
    }
    let next = states
        .iter()
        .position(|state| state.lifecycle == ScheduledMemberLifecycle::Runnable);
    Ok(match next {
        Some(index) => NextScheduledMember::Runnable(schedule.members[index].clone()),
        None => NextScheduledMember::Exhausted,
    })
}
