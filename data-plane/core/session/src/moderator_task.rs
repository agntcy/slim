// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use tokio::sync::oneshot;
use tracing::debug;

use slim_datapath::api::ProtoMessage as Message;

// Local crate
use crate::errors::SessionError;

#[derive(Debug, Default)]
pub(crate) struct State {
    received: bool,
    timer_id: u32,
}

pub(crate) trait TaskUpdate {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn leave_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn leave_complete(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn welcome_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    // used on commit send message to the control channel name
    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    // used on commit send message to the legacy channel name
    fn commit_legacy_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    #[allow(dead_code)]
    fn proposal_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn task_complete(&self) -> bool;
}

fn unsupported_phase() -> SessionError {
    SessionError::ModeratorTaskUnsupportedPhase
}

#[derive(Debug)]
pub enum ModeratorTask {
    Add(AddParticipant),
    Remove(RemoveParticipant),
    // this task is used both on session close
    // and on disconnection detection. in both cases
    // we need to notify all the participant in the
    // group and wait for the acks
    CloseOrDisconnect(NotifyParticipants),
    #[allow(dead_code)]
    Update(UpdateParticipant),
}

impl ModeratorTask {
    /// Takes (moves) the underlying ack channel sender out of the task
    pub(crate) fn ack_tx_take(&mut self) -> Option<oneshot::Sender<Result<(), SessionError>>> {
        match self {
            ModeratorTask::Add(t) => t.ack_tx.take(),
            ModeratorTask::Remove(t) => t.ack_tx.take(),
            ModeratorTask::CloseOrDisconnect(t) => t.ack_tx.take(),
            ModeratorTask::Update(t) => t.ack_tx.take(),
        }
    }

    pub(crate) fn set_ack_msg(&mut self, msg: Message) {
        match self {
            ModeratorTask::Add(t) => t.ack_msg = Some(msg),
            ModeratorTask::Remove(t) => t.ack_msg = Some(msg),
            ModeratorTask::CloseOrDisconnect(t) => t.ack_msg = Some(msg),
            ModeratorTask::Update(t) => t.ack_msg = Some(msg),
        }
    }

    pub(crate) fn failure_message(&self, err: SessionError) -> SessionError {
        match self {
            ModeratorTask::Add(_) => SessionError::ModeratorTaskAddFailed {
                source: Box::new(err),
            },
            ModeratorTask::Remove(_) => SessionError::ModeratorTaskRemoveFailed {
                source: Box::new(err),
            },
            ModeratorTask::Update(_) => SessionError::ModeratorTaskUpdateFailed {
                source: Box::new(err),
            },
            ModeratorTask::CloseOrDisconnect(_) => SessionError::ModeratorTaskCloseFailed {
                source: Box::new(err),
            },
        }
    }
    pub(crate) fn ack_msg(&self) -> Option<&Message> {
        match self {
            ModeratorTask::Add(t) => t.ack_msg.as_ref(),
            ModeratorTask::Remove(t) => t.ack_msg.as_ref(),
            ModeratorTask::CloseOrDisconnect(t) => t.ack_msg.as_ref(),
            ModeratorTask::Update(t) => t.ack_msg.as_ref(),
        }
    }
}

impl TaskUpdate for ModeratorTask {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        println!("discovery start id {}", timer_id);
        match self {
            ModeratorTask::Add(task) => task.discovery_start(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        println!("discovery complete id {}", timer_id);
        match self {
            ModeratorTask::Add(task) => task.discovery_complete(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        println!("join start id {}", timer_id);
        match self {
            ModeratorTask::Add(task) => task.join_start(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        println!("join complete id {}", timer_id);
        match self {
            ModeratorTask::Add(task) => task.join_complete(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn leave_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Remove(task) => task.leave_start(timer_id),
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
        println!("commit start id {}", timer_id);
        match self {
            ModeratorTask::Add(task) => task.commit_start(timer_id),
            ModeratorTask::Remove(task) => task.commit_start(timer_id),
            ModeratorTask::Update(task) => task.commit_start(timer_id),
            ModeratorTask::CloseOrDisconnect(task) => task.commit_start(timer_id),
        }
    }

    fn commit_legacy_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        println!("commit legacy start id {}", timer_id);
        match self {
            ModeratorTask::Add(task) => task.commit_legacy_start(timer_id),
            ModeratorTask::Remove(task) => task.commit_legacy_start(timer_id),
            ModeratorTask::Update(task) => task.commit_legacy_start(timer_id),
            ModeratorTask::CloseOrDisconnect(task) => task.commit_legacy_start(timer_id),
        }
    }

    fn proposal_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::Update(task) => task.proposal_start(timer_id),
            _ => Err(unsupported_phase()),
        }
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        println!("phase completed id {}", timer_id);
        match self {
            ModeratorTask::Add(task) => task.update_phase_completed(timer_id),
            ModeratorTask::Remove(task) => task.update_phase_completed(timer_id),
            ModeratorTask::Update(task) => task.update_phase_completed(timer_id),
            ModeratorTask::CloseOrDisconnect(task) => task.update_phase_completed(timer_id),
        }
    }

    fn task_complete(&self) -> bool {
        match self {
            ModeratorTask::Add(task) => task.task_complete(),
            ModeratorTask::Remove(task) => task.task_complete(),
            ModeratorTask::Update(task) => task.task_complete(),
            ModeratorTask::CloseOrDisconnect(task) => task.task_complete(),
        }
    }
}

#[derive(Debug, Default)]
pub struct AddParticipant {
    discovery: State,
    join: State,
    welcome: State,
    commit: State,
    commit_legacy: Option<State>,
    /// Optional ack message to send back to the control plane upon completion
    ack_msg: Option<Message>,
    /// Optional ack notifier to signal when the invite operation completes (after welcome+commit)
    pub(crate) ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
}

impl AddParticipant {
    pub(crate) fn new(
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
        ack_msg: Option<Message>,
    ) -> Self {
        Self {
            discovery: Default::default(),
            join: Default::default(),
            welcome: Default::default(),
            commit: Default::default(),
            commit_legacy: None,
            ack_msg,
            ack_tx,
        }
    }
}

impl TaskUpdate for AddParticipant {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(
            %timer_id,
            "start discovery on AddParticipant task",
        );
        self.discovery.received = false;
        self.discovery.timer_id = timer_id;
        Ok(())
    }

    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.discovery.timer_id == timer_id {
            self.discovery.received = true;
            debug!(
                %timer_id,
                "discovery completed on AddParticipant task"
            );
            Ok(())
        } else {
            Err(SessionError::ModeratorTaskUnexpectedTimerId(timer_id))
        }
    }

    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(%timer_id, "start join on AddParticipant task");
        self.join.received = false;
        self.join.timer_id = timer_id;
        Ok(())
    }

    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.join.timer_id == timer_id {
            self.join.received = true;
            debug!(
                %timer_id,
                "join completed on AddParticipant task"
            );
            Ok(())
        } else {
            Err(SessionError::ModeratorTaskUnexpectedTimerId(timer_id))
        }
    }

    fn leave_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn leave_complete(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn welcome_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(%timer_id, "start welcome on AddParticipant task");
        self.welcome.received = false;
        self.welcome.timer_id = timer_id;
        Ok(())
    }

    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(%timer_id, "start commit on AddParticipant task");
        self.commit.received = false;
        self.commit.timer_id = timer_id;
        Ok(())
    }

    fn commit_legacy_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(%timer_id, "start commit legacy on AddParticipant task");
        let legacy_commit = self.commit_legacy.get_or_insert(State::default());
        legacy_commit.received = false;
        legacy_commit.timer_id = timer_id;
        Ok(())
    }

    fn proposal_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.welcome.timer_id == timer_id {
            self.welcome.received = true;
            debug!(
                %timer_id,
                "welcome completed on AddParticipant task",
            );
        } else if self.commit.timer_id == timer_id {
            self.commit.received = true;
            debug!(
                %timer_id,
                "commit completed on AddParticipant task",
            );
        } else if let Some(legacy_commit) = &mut self.commit_legacy
            && legacy_commit.timer_id == timer_id
        {
            debug!(
                %timer_id,
                "commit legacy completed on AddParticipant task",
            );
            legacy_commit.received = true;
        } else {
            return Err(SessionError::ModeratorTaskUnexpectedTimerId(timer_id));
        }

        // Ack only after both welcome and commit phases are done.
        if self.welcome.received && self.commit.received && self.commit_legacy.as_ref().is_none_or(|c| c.received) {
            if let Some(tx) = self.ack_tx.take() {
                let _ = tx.send(Ok(()));
            }
        }

        Ok(())
    }

    fn task_complete(&self) -> bool {
        println!("task complete check {} {} {} {} {}",
            self.discovery.received,
            self.join.received,
            self.welcome.received,
            self.commit.received,
            self.commit_legacy.as_ref().map_or(true, |c| c.received),
        );
        self.discovery.received
            && self.join.received
            && self.welcome.received
            && self.commit.received
            && self.commit_legacy.as_ref().is_none_or(|c| c.received)
    }
}

#[derive(Debug, Default)]
pub struct RemoveParticipant {
    commit: State,
    // use to track messages on the legacy channel
    commit_legacy: Option<State>,
    leave: State,
    /// Optional ack message to send back to the control plane upon completion
    ack_msg: Option<Message>,
    /// Optional ack notifier to signal when the remove operation completes (after LeaveReply)
    pub(crate) ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
}

impl RemoveParticipant {
    pub(crate) fn new(
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
        ack_msg: Option<Message>,
    ) -> Self {
        Self {
            commit: Default::default(),
            commit_legacy: None,
            leave: Default::default(),
            ack_msg,
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
            %timer_id,
            "start leave on RemoveParticipant task",
        );
        self.leave.received = false;
        self.leave.timer_id = timer_id;
        Ok(())
    }

    fn leave_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.leave.timer_id == timer_id {
            self.leave.received = true;
            debug!(
                %timer_id,
                "leave completed on RemoveParticipant task",
            );

            // Signal success to the ack notifier if present (remove operation complete)
            if let Some(tx) = self.ack_tx.take() {
                let _ = tx.send(Ok(()));
            }

            Ok(())
        } else {
            Err(SessionError::ModeratorTaskUnexpectedTimerId(timer_id))
        }
    }

    fn welcome_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(
            %timer_id,
            "start commit on RemoveParticipanMls task",
        );
        self.commit.received = false;
        self.commit.timer_id = timer_id;
        Ok(())
    }

    fn commit_legacy_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(
            %timer_id,
            "start commit legacy on RemoveParticipanMls task",
        );
        let legacy_commit = self.commit_legacy.get_or_insert(State::default());
        legacy_commit.received = false;
        legacy_commit.timer_id = timer_id;
        Ok(())
    }

    fn proposal_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.commit.timer_id == timer_id {
            self.commit.received = true;
            debug!(
                %timer_id,
                "commit completed on RemoveParticipanMls task",
            );
            Ok(())
        } else if let Some(legacy_commit) = &mut self.commit_legacy
            && legacy_commit.timer_id == timer_id
        {
            legacy_commit.received = true;
            debug!(
                %timer_id,
                "legacy commit completed on RemoveParticipanMls task",
            );
            Ok(())
        } else {
            Err(SessionError::ModeratorTaskUnexpectedTimerId(timer_id))
        }
    }

    fn task_complete(&self) -> bool {
        self.commit.received
            && self.leave.received
            && self.commit_legacy.as_ref().is_none_or(|c| c.received)
    }
}

#[derive(Debug, Default)]
pub struct NotifyParticipants {
    notify: State,
    notify_legacy: Option<State>,
    /// Optional ack message to send back to the control plane upon completion
    ack_msg: Option<Message>,
    /// Optional ack notifier to signal when the notify operation completes
    pub(crate) ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
}

impl NotifyParticipants {
    pub(crate) fn new(
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
        ack_msg: Option<Message>,
    ) -> Self {
        Self {
            notify: Default::default(),
            notify_legacy: None,
            ack_msg,
            ack_tx,
        }
    }
}

impl TaskUpdate for NotifyParticipants {
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
        debug!(%timer_id, "start notify participants task");
        self.notify.received = false;
        self.notify.timer_id = timer_id;
        Ok(())
    }

    fn commit_legacy_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(%timer_id, "start notify participants legacy task");
        let legacy_notify = self.notify_legacy.get_or_insert(State::default());
        legacy_notify.received = false;
        legacy_notify.timer_id = timer_id;
        Ok(())
    }

    fn proposal_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(unsupported_phase())
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.notify.timer_id == timer_id {
            self.notify.received = true;
            debug!(
                %timer_id,
                "notify participants completed on NotifyParticipants task",
            );
        } else if let Some(legacy_notify) = &mut self.notify_legacy
            && legacy_notify.timer_id == timer_id
        {
            legacy_notify.received = true;
            debug!(
                %timer_id,
                "notify participants legacy completed on NotifyParticipants task",
            );
        } else {
            return Err(SessionError::ModeratorTaskUnexpectedTimerId(timer_id));
        }

        if self.notify.received
            && (self.notify_legacy.is_none() || self.notify_legacy.as_ref().unwrap().received)
        {
            // Signal success to the ack notifier if present (notify operation complete)
            if let Some(tx) = self.ack_tx.take() {
                let _ = tx.send(Ok(()));
            }
        }

        Ok(())
    }

    fn task_complete(&self) -> bool {
        self.notify.received
    }
}

#[derive(Debug, Default)]
pub struct UpdateParticipant {
    proposal: State,
    commit: State,
    commit_legacy: Option<State>,
    /// Optional ack message to send back to the control plane upon completion
    ack_msg: Option<Message>,
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
            %timer_id,
            "start commit on UpdateParticipanMls task",
        );
        self.commit.received = false;
        self.commit.timer_id = timer_id;
        Ok(())
    }

    fn commit_legacy_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(
            %timer_id,
            "start commit legacy on UpdateParticipanMls task",
        );
        let legacy_commit = self.commit_legacy.get_or_insert(State::default());
        legacy_commit.received = false;
        legacy_commit.timer_id = timer_id;
        Ok(())
    }

    fn proposal_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        debug!(%timer_id,
            "start proposal on UpdateParticipanMls task",
        );
        self.proposal.received = false;
        self.proposal.timer_id = timer_id;
        Ok(())
    }

    fn update_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.proposal.timer_id == timer_id {
            self.proposal.received = true;
            debug!(
                %timer_id,
                "proposal completed on UpdateParticipanMls task",
            );
            Ok(())
        } else if self.commit.timer_id == timer_id {
            self.commit.received = true;
            debug!(
                %timer_id,
                "commit completed on UpdateParticipanMls task",
            );
            Ok(())
        } else if let Some(legacy_commit) = &mut self.commit_legacy
            && legacy_commit.timer_id == timer_id
        {
            legacy_commit.received = true;
            debug!(
                %timer_id,
                "legacy commit completed on UpdateParticipanMls task",
            );
            Ok(())
        } else {
            Err(SessionError::ModeratorTaskUnexpectedTimerId(timer_id))
        }
    }

    fn task_complete(&self) -> bool {
        self.proposal.received
            && self.commit.received
            && self.commit_legacy.as_ref().is_none_or(|c| c.received)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    #[derive(Debug)]
    enum StepExpectation {
        Ok,
        UnexpectedTimerId(u32),
        UnsupportedPhase,
    }

    struct Step {
        name: &'static str,
        action: Box<dyn Fn(&mut ModeratorTask) -> Result<(), SessionError>>,
        expectation: StepExpectation,
        expect_complete: bool,
    }

    impl Step {
        fn ok<F: 'static + Fn(&mut ModeratorTask) -> Result<(), SessionError>>(
            name: &'static str,
            f: F,
            expect_complete: bool,
        ) -> Self {
            Step {
                name,
                action: Box::new(f),
                expectation: StepExpectation::Ok,
                expect_complete,
            }
        }
        fn unexpected_timer<F: 'static + Fn(&mut ModeratorTask) -> Result<(), SessionError>>(
            name: &'static str,
            f: F,
            id: u32,
            expect_complete: bool,
        ) -> Self {
            Step {
                name,
                action: Box::new(f),
                expectation: StepExpectation::UnexpectedTimerId(id),
                expect_complete,
            }
        }
        fn unsupported<F: 'static + Fn(&mut ModeratorTask) -> Result<(), SessionError>>(
            name: &'static str,
            f: F,
            expect_complete: bool,
        ) -> Self {
            Step {
                name,
                action: Box::new(f),
                expectation: StepExpectation::UnsupportedPhase,
                expect_complete,
            }
        }
    }

    fn run_scenario(mut task: ModeratorTask, steps: Vec<Step>) {
        assert!(!task.task_complete(), "task should start incomplete");
        for (i, step) in steps.into_iter().enumerate() {
            let res = (step.action)(&mut task);
            match step.expectation {
                StepExpectation::Ok => {
                    if let Err(e) = res {
                        panic!("step {} ({}) expected Ok, got Err {:?}", i, step.name, e);
                    }
                }
                StepExpectation::UnexpectedTimerId(expected_id) => match res {
                    Err(SessionError::ModeratorTaskUnexpectedTimerId(actual_id)) => {
                        assert_eq!(
                            actual_id, expected_id,
                            "step {} ({}) unexpected timer id mismatch",
                            i, step.name
                        );
                    }
                    other => panic!(
                        "step {} ({}) expected ModeratorTaskUnexpectedTimerId({}), got {:?}",
                        i, step.name, expected_id, other
                    ),
                },
                StepExpectation::UnsupportedPhase => match res {
                    Err(SessionError::ModeratorTaskUnsupportedPhase) => {}
                    other => {
                        panic!(
                            "step {} ({}) expected ModeratorTaskUnsupportedPhase, got {:?}",
                            i, step.name, other
                        );
                    }
                },
            }
            assert_eq!(
                task.task_complete(),
                step.expect_complete,
                "step {} ({}) completion mismatch",
                i,
                step.name
            );
        }
    }

    #[test]
    #[traced_test]
    fn test_add_participant_scenario() {
        let base = 10;
        run_scenario(
            ModeratorTask::Add(AddParticipant::default()),
            vec![
                Step::ok("discovery_start", move |t| t.discovery_start(base), false),
                Step::unexpected_timer(
                    "discovery_complete_wrong",
                    move |t| t.discovery_complete(base + 1),
                    base + 1,
                    false,
                ),
                Step::unsupported(
                    "leave_start_unsupported",
                    move |t| t.leave_start(base),
                    false,
                ),
                Step::ok(
                    "discovery_complete_ok",
                    move |t| t.discovery_complete(base),
                    false,
                ),
                Step::ok("join_start", move |t| t.join_start(base + 1), false),
                Step::ok("join_complete", move |t| t.join_complete(base + 1), false),
                Step::ok("welcome_start", move |t| t.welcome_start(base + 2), false),
                Step::ok("commit_start", move |t| t.commit_start(base + 3), false),
                Step::ok(
                    "welcome_phase_completed",
                    move |t| t.update_phase_completed(base + 2),
                    false,
                ),
                Step::ok(
                    "commit_phase_completed",
                    move |t| t.update_phase_completed(base + 3),
                    true,
                ),
            ],
        );
    }

    #[test]
    #[traced_test]
    fn test_remove_participant_scenario() {
        let base = 10;
        run_scenario(
            ModeratorTask::Remove(RemoveParticipant::default()),
            vec![
                Step::ok("commit_start", move |t| t.commit_start(base), false),
                Step::ok(
                    "commit_completed",
                    move |t| t.update_phase_completed(base),
                    false,
                ),
                Step::ok("leave_start", move |t| t.leave_start(base + 1), false),
                Step::unexpected_timer(
                    "leave_complete_wrong",
                    move |t| t.leave_complete(base + 2),
                    base + 2,
                    false,
                ),
                Step::unsupported(
                    "discovery_start_unsupported",
                    move |t| t.discovery_start(base),
                    false,
                ),
                Step::ok(
                    "leave_complete_ok",
                    move |t| t.leave_complete(base + 1),
                    true,
                ),
            ],
        );
    }

    #[test]
    #[traced_test]
    fn test_update_participant_mls_scenario() {
        let base = 10;
        run_scenario(
            ModeratorTask::Update(UpdateParticipant::default()),
            vec![
                Step::ok("commit_start", move |t| t.commit_start(base), false),
                Step::ok(
                    "commit_completed",
                    move |t| t.update_phase_completed(base),
                    false,
                ),
                Step::ok("proposal_start", move |t| t.proposal_start(base), false),
                Step::ok(
                    "proposal_completed",
                    move |t| t.update_phase_completed(base),
                    true,
                ),
            ],
        );
    }

    #[test]
    #[traced_test]
    fn test_add_participant_ack_after_welcome_and_commit() {
        let base = 10u32;
        let (tx, mut rx) = tokio::sync::oneshot::channel::<Result<(), SessionError>>();
        let mut task = ModeratorTask::Add(AddParticipant::new(Some(tx), None));

        task.discovery_start(base).unwrap();
        task.discovery_complete(base).unwrap();
        task.join_start(base + 1).unwrap();
        task.join_complete(base + 1).unwrap();
        // ack must NOT fire on join
        assert!(rx.try_recv().is_err());

        task.welcome_start(base + 2).unwrap();
        task.commit_start(base + 3).unwrap();

        // only welcome done — still no ack
        task.update_phase_completed(base + 2).unwrap();
        assert!(rx.try_recv().is_err());

        // commit done — both phases complete, ack fires
        task.update_phase_completed(base + 3).unwrap();
        assert!(rx.try_recv().is_ok());

        assert!(task.task_complete());
    }

    #[test]
    #[traced_test]
    fn test_close_group() {
        let mut task = ModeratorTask::CloseOrDisconnect(NotifyParticipants::default());
        assert!(!task.task_complete());

        let timer_id = 10;
        task.commit_start(timer_id).expect("error on commit start");
        assert!(!task.task_complete());

        let mut res = task.update_phase_completed(timer_id + 1);
        assert!(res.is_err_and(|e| matches!(e, SessionError::ModeratorTaskUnexpectedTimerId(_))));

        res = task.discovery_start(timer_id);
        assert!(res.is_err_and(|e| matches!(e, SessionError::ModeratorTaskUnsupportedPhase)));

        task.update_phase_completed(timer_id)
            .expect("error on notify completed");
        assert!(task.task_complete());
    }
}
