use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AgentCoreError, AgentCoreResult};
use crate::run_message::{MessageRole, RunMessage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnLoopState {
    Idle,
    Ready,
    Running,
    Stopping,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnInput {
    pub turn_id: Uuid,
    pub user_message: RunMessage,
}

#[derive(Debug, Clone)]
pub struct TurnLoop {
    state: TurnLoopState,
    user_message_buffer: VecDeque<RunMessage>,
    active_turn_id: Option<Uuid>,
    stop_requested: bool,
}

impl TurnLoop {
    pub fn new() -> Self {
        Self {
            state: TurnLoopState::Idle,
            user_message_buffer: VecDeque::new(),
            active_turn_id: None,
            stop_requested: false,
        }
    }

    pub fn state(&self) -> TurnLoopState {
        self.state
    }

    pub fn active_turn_id(&self) -> Option<Uuid> {
        self.active_turn_id
    }

    pub fn buffer_len(&self) -> usize {
        self.user_message_buffer.len()
    }

    pub fn submit_user_message(&mut self, message: RunMessage) -> AgentCoreResult<()> {
        if message.role != MessageRole::User {
            return Err(AgentCoreError::InvalidInput(
                "turn loop only accepts role=user messages".to_string(),
            ));
        }

        self.user_message_buffer.push_back(message);
        if matches!(self.state, TurnLoopState::Idle) {
            self.state = TurnLoopState::Ready;
        }
        Ok(())
    }

    pub fn prepare_turn(&mut self) -> AgentCoreResult<Option<TurnInput>> {
        if self.stop_requested
            || matches!(self.state, TurnLoopState::Stopping | TurnLoopState::Stopped)
        {
            return Ok(None);
        }

        if self.active_turn_id.is_some() {
            return Err(AgentCoreError::InvalidInput(
                "cannot prepare a new turn while another turn is running".to_string(),
            ));
        }

        let Some(user_message) = self.user_message_buffer.pop_front() else {
            self.state = TurnLoopState::Idle;
            return Ok(None);
        };

        let turn_id = Uuid::new_v4();
        self.active_turn_id = Some(turn_id);
        self.state = TurnLoopState::Running;
        Ok(Some(TurnInput {
            turn_id,
            user_message,
        }))
    }

    pub fn finish_turn(&mut self, turn_id: Uuid) -> AgentCoreResult<()> {
        self.require_active_turn(turn_id)?;
        self.active_turn_id = None;

        if self.stop_requested {
            self.state = TurnLoopState::Stopped;
        } else if self.user_message_buffer.is_empty() {
            self.state = TurnLoopState::Idle;
        } else {
            self.state = TurnLoopState::Ready;
        }

        Ok(())
    }

    pub fn abort_turn(&mut self, turn_id: Uuid) -> AgentCoreResult<()> {
        self.require_active_turn(turn_id)?;
        self.active_turn_id = None;
        self.state = TurnLoopState::Stopped;
        self.stop_requested = true;
        Ok(())
    }

    pub fn request_stop(&mut self) {
        self.stop_requested = true;
        self.state = if self.active_turn_id.is_some() {
            TurnLoopState::Stopping
        } else {
            TurnLoopState::Stopped
        };
    }

    pub fn clear_stop(&mut self) {
        self.stop_requested = false;
        if self.active_turn_id.is_none() {
            self.state = if self.user_message_buffer.is_empty() {
                TurnLoopState::Idle
            } else {
                TurnLoopState::Ready
            };
        }
    }

    pub fn stop_requested(&self) -> bool {
        self.stop_requested
    }

    fn require_active_turn(&self, turn_id: Uuid) -> AgentCoreResult<()> {
        match self.active_turn_id {
            Some(active_turn_id) if active_turn_id == turn_id => Ok(()),
            Some(active_turn_id) => Err(AgentCoreError::InvalidInput(format!(
                "turn id mismatch: active={active_turn_id}, got={turn_id}"
            ))),
            None => Err(AgentCoreError::InvalidInput(
                "no active turn is running".to_string(),
            )),
        }
    }
}

impl Default for TurnLoop {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_block::ContentBlock;

    #[test]
    fn prepares_user_message_as_turn_input() {
        let mut turn_loop = TurnLoop::new();
        let message = RunMessage::user(vec![ContentBlock::text("hello")]).unwrap();

        turn_loop.submit_user_message(message.clone()).unwrap();
        let turn = turn_loop.prepare_turn().unwrap().unwrap();

        assert_eq!(turn.user_message, message);
        assert_eq!(turn_loop.state(), TurnLoopState::Running);
    }

    #[test]
    fn stop_prevents_new_turns() {
        let mut turn_loop = TurnLoop::new();
        turn_loop
            .submit_user_message(RunMessage::user(vec![ContentBlock::text("hello")]).unwrap())
            .unwrap();

        turn_loop.request_stop();

        assert!(turn_loop.prepare_turn().unwrap().is_none());
        assert_eq!(turn_loop.state(), TurnLoopState::Stopped);
    }
}
