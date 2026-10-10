//! The agent behind Agent mode, whichever it is: OpenCode through Jig's
//! own client ([`crate::agent`]), or any agent that speaks the Agent Client
//! Protocol ([`crate::acp`]). The UI only sees these two types.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::acp;
use crate::agent::{
    AgentCommand, AgentEvent, AgentRequest, AgentServer, AgentSession, Conversation, Permissions,
    Turn,
};

/// A running agent, shared by every conversation with it.
#[derive(Clone)]
pub enum Backend {
    OpenCode(Arc<AgentServer>),
    Acp(Arc<acp::Connection>),
}

impl Backend {
    /// Its earlier conversations in `directory`, latest first.
    pub fn conversations(&self, directory: &Path) -> Result<Vec<Conversation>> {
        match self {
            Backend::OpenCode(server) => server.conversations(directory),
            Backend::Acp(connection) => connection.conversations(directory),
        }
    }

    /// The commands and skills it offers in `directory`. An ACP agent only
    /// says once a conversation has started, so this may be empty at first.
    pub fn commands(&self, directory: &Path) -> Result<Vec<AgentCommand>> {
        match self {
            Backend::OpenCode(server) => server.commands(directory),
            Backend::Acp(connection) => Ok(connection.commands()),
        }
    }

    /// A new conversation in `directory`, titled with what was asked where
    /// the agent takes a title.
    pub fn create(
        &self,
        directory: &Path,
        title: &str,
        permissions: &Permissions,
    ) -> Result<Session> {
        Ok(match self {
            Backend::OpenCode(server) => Session::OpenCode(AgentSession::create(
                server.clone(),
                directory,
                title,
                permissions,
            )?),
            Backend::Acp(connection) => {
                Session::Acp(acp::Session::create(connection.clone(), directory)?)
            }
        })
    }

    /// An earlier conversation, to load with [`Session::history`].
    pub fn open(&self, directory: &Path, id: &str) -> Result<Session> {
        Ok(match self {
            Backend::OpenCode(server) => {
                Session::OpenCode(AgentSession::open(server.clone(), directory, id)?)
            }
            Backend::Acp(connection) => {
                Session::Acp(acp::Session::open(connection.clone(), directory, id))
            }
        })
    }
}

/// One conversation. Cheap to clone, so the UI can answer requests and stop
/// the agent while another thread runs [`Session::run`].
#[derive(Clone)]
pub enum Session {
    OpenCode(AgentSession),
    Acp(acp::Session),
}

impl Session {
    pub fn id(&self) -> &str {
        match self {
            Session::OpenCode(session) => session.id(),
            Session::Acp(session) => session.id(),
        }
    }

    pub fn history(&self) -> Result<Vec<Turn>> {
        match self {
            Session::OpenCode(session) => session.history(),
            Session::Acp(session) => session.history(),
        }
    }

    pub fn run(&self, request: &AgentRequest, on_event: &dyn Fn(AgentEvent)) -> Result<()> {
        match self {
            Session::OpenCode(session) => session.run(request, on_event),
            Session::Acp(session) => session.run(request, on_event),
        }
    }

    pub fn reply(&self, request_id: &str, accept: bool, note: Option<&str>) -> Result<()> {
        match self {
            Session::OpenCode(session) => session.reply(request_id, accept, note),
            Session::Acp(session) => session.reply(request_id, accept, note),
        }
    }

    /// Let a request through, and the agent not ask about ones like it again.
    pub fn allow_always(&self, request_id: &str) -> Result<()> {
        match self {
            Session::OpenCode(session) => session.allow_always(request_id),
            Session::Acp(session) => session.allow_always(request_id),
        }
    }

    /// Answer the agent's questions; only OpenCode asks them this way.
    pub fn answer(&self, question_id: &str, answers: Option<&[String]>) -> Result<()> {
        match self {
            Session::OpenCode(session) => session.answer(question_id, answers),
            Session::Acp(_) => Ok(()),
        }
    }

    pub fn abort(&self) -> Result<()> {
        match self {
            Session::OpenCode(session) => session.abort(),
            Session::Acp(session) => session.abort(),
        }
    }
}
