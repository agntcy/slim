// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use tokio::sync::oneshot;
use tracing::debug;

// Local crate
use crate::errors::SessionError;

#[derive(Debug, Default)]
pub(crate) struct State {
    received: bool,
    timer_id: u32,
}

pub trait TaskUpdate {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn leave_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn leave_complete(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn welcome_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    #[allow(dead_code)]
    fn proposal_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn task_complete(&self) -> bool;
}

fn unsupported_phase() -> SessionError {
    SessionError::ModeratorTask("this phase is not supported in this task".to_string())
}

#[derive(Debug)]
pub enum ModeratorTask {
    Add(AddParticipant),
    Remove(RemoveParticipant),
    Close(CloseGroup),
    #[allow(dead_code)]
    Update(UpdateParticipant),
}

impl TaskUpdate for ModeratorTask {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Add(task) => task.discovery_start(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Add(task) => task.discovery_complete(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Add(task) => task.join_start(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Add(task) => task.join_complete(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn leave_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Remove(task) => task.leave_start(timer_id),
            ModeratorTask::Close(task) => task.leave_start(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn leave_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Remove(task) => task.leave_complete(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn welcome_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Add(task) => task.welcome_start(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Add(task) => task.commit_start(timer_id),
            ModeratorTask::Remove(task) => task.commit_start(timer_id),
            ModeratorTask::Update(task) => task.commit_start(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn proposal_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Update(task) => task.proposal_start(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Add(task) => task.update_phase_completed(timer_id),
            ModeratorTask::Remove(task) => task.update_phase_completed(timer_id),
            ModeratorTask::Update(task) => task.update_phase_completed(timer_id),
            ModeratorTask::Close(task) => task.update_phase_completed(timer_id),
        }
    }

    fn task_complete(&self) -> bool {
        match self {
            ModeratorTask::Add(task) => task.task_complete(),
            ModeratorTask::Remove(task) => task.task_complete(),
            ModeratorTask::Update(task) => task.task_complete(),
            ModeratorTask::Close(task) => task.task_complete(),
        }
    }
}

#[derive(Debug, Default)]
pub struct AddParticipant {
    discovery: State,
    join: State,
    welcome: State,
    commit: State,
    /// Optional ack notifier to signal when the invite operation completes (after JoinReply)
    pub(crate) ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
}

impl AddParticipant {
    pub(crate) fn new(ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>) -> Self {
        Self {
            discovery: Default::default(),
            join: Default::default(),
            welcome: Default::default(),
            commit: Default::default(),
            ack_tx,
        }
    }
}

impl TaskUpdate for AddParticipant {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(
            "start discovery on AddParticipan task, timer id {}",
            timer_id
        );
        self.discovery.received = false;
        self.discovery.timer_id = timer_id;
        Ok(())
    }

    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.discovery.timer_id == timer_id {
            self.discovery.received = true;
            debug!(
                "discovery completed on AddParticipan task, timer id {}",
                timer_id
            );
            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!("start join on AddParticipan task, timer id {}", timer_id);
        self.join.received = false;
        self.join.timer_id = timer_id;
        Ok(())
    }

    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.join.timer_id == timer_id {
            self.join.received = true;
            debug!(
                "join completed on AddParticipan task, timer id {}",
                timer_id
            );

            // Signal success to the ack notifier if present (invite operation complete)
            if let Some(tx) = self.ack_tx.take() {
                let _ = tx.send(Ok(()));
            }

            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn leave_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn leave_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn welcome_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!("start welcome on AddParticipan task, timer id {}", timer_id);
        self.welcome.received = false;
        self.welcome.timer_id = timer_id;
        Ok(())
    }

    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!("start commit on AddParticipan task, timer id {}", timer_id);
        self.commit.received = false;
        self.commit.timer_id = timer_id;
        Ok(())
    }

    fn proposal_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.welcome.timer_id == timer_id {
            self.welcome.received = true;
            debug!(
                "welcome completed on AddParticipan task, timer id {}",
                timer_id
            );
            Ok(())
        } else if self.commit.timer_id == timer_id {
            self.commit.received = true;
            debug!(
                "commit completed on AddParticipan task, timer id {}",
                timer_id
            );
            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn task_complete(&self) -> bool {
        self.discovery.received
            && self.join.received
            && self.welcome.received
            && self.commit.received
    }
}

#[derive(Debug, Default)]
pub struct RemoveParticipant {
    commit: State,
    leave: State,
    /// Optional ack notifier to signal when the remove operation completes (after LeaveReply)
    pub(crate) ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
}

impl RemoveParticipant {
    pub(crate) fn new(ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>) -> Self {
        Self {
            commit: Default::default(),
            leave: Default::default(),
            ack_tx,
        }
    }
}

impl TaskUpdate for RemoveParticipant {
    fn discovery_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn discovery_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn join_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn join_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn leave_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(
            "start leave on RemoveParticipant task, timer id {}",
            timer_id
        );
        self.leave.received = false;
        self.leave.timer_id = timer_id;
        Ok(())
    }

    fn leave_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.leave.timer_id == timer_id {
            self.leave.received = true;
            debug!(
                "leave completed on RemoveParticipant task, timer id {}",
                timer_id
            );

            // Signal success to the ack notifier if present (remove operation complete)
            if let Some(tx) = self.ack_tx.take() {
                let _ = tx.send(Ok(()));
            }

            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn welcome_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(
            "start commit on RemoveParticipanMls task, timer id {}",
            timer_id
        );
        self.commit.received = false;
        self.commit.timer_id = timer_id;
        Ok(())
    }

    fn proposal_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.commit.timer_id == timer_id {
            self.commit.received = true;
            debug!(
                "commit completed on RemoveParticipanMls task, timer id {}",
                timer_id
            );
            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn task_complete(&self) -> bool {
        self.commit.received && self.leave.received
    }
}

#[derive(Debug, Default)]
pub struct CloseGroup {
    close: State,
    /// Optional ack notifier to signal when the remove operation completes (after LeaveReply)
    pub(crate) ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
}

impl CloseGroup {
    pub(crate) fn new(ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>) -> Self {
        Self {
            close: Default::default(),
            ack_tx,
        }
    }
}

impl TaskUpdate for CloseGroup {
    fn discovery_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn discovery_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn join_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn join_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn leave_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!("start close session task, timer id {}", timer_id);
        self.close.received = false;
        self.close.timer_id = timer_id;
        Ok(())
    }

    fn leave_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn welcome_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn commit_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn proposal_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.close.timer_id == timer_id {
            self.close.received = true;
            debug!("close completed on CloseGroup task, timer id {}", timer_id);

            // Signal success to the ack notifier if present (remove operation complete)
            if let Some(tx) = self.ack_tx.take() {
                let _ = tx.send(Ok(()));
            }

            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn task_complete(&self) -> bool {
        self.close.received
    }
}

#[derive(Debug, Default)]
pub struct UpdateParticipant {
    proposal: State,
    commit: State,
    /// Optional ack notifier to signal when the update operation completes
    pub(crate) ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
}

impl TaskUpdate for UpdateParticipant {
    fn discovery_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn discovery_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn join_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn join_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn leave_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn leave_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn welcome_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(
            "start commit on UpdateParticipanMls task, timer id {}",
            timer_id
        );
        self.commit.received = false;
        self.commit.timer_id = timer_id;
        Ok(())
    }

    fn proposal_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(
            "start proposal on UpdateParticipanMls task, timer id {}",
            timer_id
        );
        self.proposal.received = false;
        self.proposal.timer_id = timer_id;
        Ok(())
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.proposal.timer_id == timer_id {
            self.proposal.received = true;
            debug!(
                "proposal completed on UpdateParticipanMls task, timer id {}",
                timer_id
            );
            Ok(())
        } else if self.commit.timer_id == timer_id {
            self.commit.received = true;
            debug!(
                "commit completed on UpdateParticipanMls task, timer id {}",
                timer_id
            );
            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn task_complete(&self) -> bool {
        self.proposal.received && self.commit.received
    }
}

#[cfg(test)]
mod tests {
    use tracing_test::traced_test;

    use super::*;

    #[test]
    #[traced_test]
    fn test_add_participant() {
        let mut task = ModeratorTask::Add(AddParticipant::default());
        assert!(!task.task_complete());

        let timer_id = 10;
        task.discovery_start(timer_id)
            .expect("error on discovery start");
        assert!(!task.task_complete());

        let mut res = task.discovery_complete(timer_id + 1);
        assert_eq!(
            res,
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        );

        res = task.leave_start(timer_id);
        assert_eq!(res, Err(unsupported_phase()));

        task.discovery_complete(timer_id)
            .expect("error on discovery complete");
        assert!(!task.task_complete());

        task.join_start(timer_id + 1).expect("error on join start");
        assert!(!task.task_complete());

        task.join_complete(timer_id + 1)
            .expect("error on join complete");
        assert!(!task.task_complete());

        task.welcome_start(timer_id + 2)
            .expect("error on weclome start");
        assert!(!task.task_complete());

        task.commit_start(timer_id + 3)
            .expect("error on commit start");
        assert!(!task.task_complete());

        task.update_phase_completed(timer_id + 2)
            .expect("error mls complete (welcome)");
        assert!(!task.task_complete());

        task.update_phase_completed(timer_id + 3)
            .expect("error mls complete (commit)");
        assert!(task.task_complete());
    }

    #[test]
    #[traced_test]
    fn test_remove_participant() {
        let mut task = ModeratorTask::Remove(RemoveParticipant::default());
        assert!(!task.task_complete());

        let timer_id = 10;
        task.commit_start(timer_id).expect("error on commit start");
        assert!(!task.task_complete());

        task.update_phase_completed(timer_id)
            .expect("error on commit completed");
        assert!(!task.task_complete());

        task.leave_start(timer_id + 1)
            .expect("error on leave start");
        assert!(!task.task_complete());

        let mut res = task.leave_complete(timer_id + 2);
        assert_eq!(
            res,
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        );

        res = task.discovery_start(timer_id);
        assert_eq!(res, Err(unsupported_phase()));

        task.leave_complete(timer_id + 1)
            .expect("error on leave complete");
        assert!(task.task_complete());
    }

    #[test]
    #[traced_test]
    fn test_update_participant_mls() {
        let mut task = ModeratorTask::Update(UpdateParticipant::default());
        assert!(!task.task_complete());

        let timer_id = 10;
        task.commit_start(timer_id).expect("error on commit start");
        assert!(!task.task_complete());

        task.update_phase_completed(timer_id)
            .expect("error on commit completed");
        assert!(!task.task_complete());

        task.proposal_start(timer_id)
            .expect("error on proposal completed");
        assert!(!task.task_complete());

        task.update_phase_completed(timer_id)
            .expect("error on proposal completed");
        assert!(task.task_complete());
    }
}
