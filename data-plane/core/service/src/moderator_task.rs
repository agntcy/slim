// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::SessionError;

#[derive(Debug, Default)]
pub(crate) struct State {
    recevied: bool,
    timer_id: u32,
}

pub trait TaskUpdate {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn welcome_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn mls_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError>;
    fn task_complete(&self) -> bool;
}

#[derive(Debug)]
pub enum ModeratorTask {
    AddParticipant(AddParticipant),
    AddParticipantMls(AddParticipantMls),
    RemoveParticipant(RemoveParticipant),
    RemoveParticipantMls(RemoveParticipantMls),
}

impl TaskUpdate for ModeratorTask {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::AddParticipant(task) => task.discovery_start(timer_id),
            ModeratorTask::AddParticipantMls(task) => task.discovery_start(timer_id),
            ModeratorTask::RemoveParticipant(task) => todo!(),
            ModeratorTask::RemoveParticipantMls(task) => todo!(),
        }
    }

    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::AddParticipant(task) => task.discovery_complete(timer_id),
            ModeratorTask::AddParticipantMls(task) => task.discovery_complete(timer_id),
            ModeratorTask::RemoveParticipant(task) => todo!(),
            ModeratorTask::RemoveParticipantMls(task) => todo!(),
        }
    }

    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::AddParticipant(task) => task.join_start(timer_id),
            ModeratorTask::AddParticipantMls(task) => task.join_start(timer_id),
            ModeratorTask::RemoveParticipant(task) => todo!(),
            ModeratorTask::RemoveParticipantMls(task) => todo!(),
        }
    }

    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::AddParticipant(task) => task.join_complete(timer_id),
            ModeratorTask::AddParticipantMls(task) => task.join_complete(timer_id),
            ModeratorTask::RemoveParticipant(task) => todo!(),
            ModeratorTask::RemoveParticipantMls(task) => todo!(),
        }
    }

    fn welcome_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::AddParticipant(_task) => Err(SessionError::ModeratorTask(
                        "welcome start cannot be executed".to_string(),
                    )),
            ModeratorTask::AddParticipantMls(task) => task.welcome_start(timer_id),
            ModeratorTask::RemoveParticipant(task) => todo!(),
            ModeratorTask::RemoveParticipantMls(task) => todo!(),
        }
    }

    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::AddParticipant(_task) => Err(SessionError::ModeratorTask(
                        "commit start cannot be executed".to_string(),
                    )),
            ModeratorTask::AddParticipantMls(task) => task.commit_start(timer_id),
            ModeratorTask::RemoveParticipant(task) => todo!(),
            ModeratorTask::RemoveParticipantMls(task) => todo!(),
        }
    }

    fn mls_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        match self {
            ModeratorTask::AddParticipant(_task) => Err(SessionError::ModeratorTask(
                        "mls phase complete cannot be executed".to_string(),
                    )),
            ModeratorTask::AddParticipantMls(task) => task.mls_phase_completed(timer_id),
            ModeratorTask::RemoveParticipant(task) => todo!(),
            ModeratorTask::RemoveParticipantMls(task) => todo!(),
        }
    }

    fn task_complete(&self) -> bool {
        match self {
            ModeratorTask::AddParticipant(task) => task.task_complete(),
            ModeratorTask::AddParticipantMls(task) => task.task_complete(),
            ModeratorTask::RemoveParticipant(task) => todo!(),
            ModeratorTask::RemoveParticipantMls(task) => todo!(),
        }
    }
}

#[derive(Debug, Default)]
pub struct AddParticipant {
    discovery: State,
    join: State,
}

impl TaskUpdate for AddParticipant {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        self.discovery.recevied = false;
        self.discovery.timer_id = timer_id;
        Ok(())
    }

    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.discovery.timer_id == timer_id {
            self.discovery.recevied = true;
            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        self.join.recevied = false;
        self.join.timer_id = timer_id;
        Ok(())
    }

    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.join.timer_id == timer_id {
            self.join.recevied = true;
            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn welcome_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(SessionError::ModeratorTask(
            "this phase is not supported in this task".to_string(),
        ))
    }

    fn commit_start(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(SessionError::ModeratorTask(
            "this phase is not supported in this task".to_string(),
        ))
    }

    fn mls_phase_completed(&mut self, _timer_id: u32) -> Result<(), SessionError> {
        Err(SessionError::ModeratorTask(
            "this phase is not supported in this task".to_string(),
        ))
    }

    fn task_complete(&self) -> bool {
        self.discovery.recevied && self.join.recevied
    }
}

#[derive(Debug, Default)]
pub struct AddParticipantMls {
    discovery: State,
    join: State,
    welcome: State,
    commit: State,
}

impl TaskUpdate for AddParticipantMls {
    fn discovery_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        self.discovery.recevied = false;
        self.discovery.timer_id = timer_id;
        Ok(())
    }

    fn discovery_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.discovery.timer_id == timer_id {
            self.discovery.recevied = true;
            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn join_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        self.join.recevied = false;
        self.join.timer_id = timer_id;
        Ok(())
    }

    fn join_complete(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.join.timer_id == timer_id {
            self.join.recevied = true;
            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn welcome_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        self.welcome.recevied = false;
        self.welcome.timer_id = timer_id;
        Ok(())
    }

    fn commit_start(&mut self, timer_id: u32) -> Result<(), SessionError> {
        self.commit.recevied = false;
        self.commit.timer_id = timer_id;
        Ok(())
    }

    fn mls_phase_completed(&mut self, timer_id: u32) -> Result<(), SessionError> {
        if self.welcome.timer_id == timer_id {
            self.welcome.recevied = true;
            Ok(())
        } else if self.commit.timer_id == timer_id {
            self.commit.recevied = true;
            Ok(())
        } else {
            Err(SessionError::ModeratorTask(
                "unexpected timer id".to_string(),
            ))
        }
    }

    fn task_complete(&self) -> bool {
        self.discovery.recevied
            && self.join.recevied
            && self.welcome.recevied
            && self.commit.recevied
    }
}

#[derive(Debug, Default)]
pub struct RemoveParticipant {
    leave: State,
}

#[derive(Debug, Default)]
pub struct RemoveParticipantMls {
    commit: State,
    leave: State,
}

#[derive(Debug, Default)]
pub struct UpdateParticipantMls {
    proposal: State,
    commit: State,
}
