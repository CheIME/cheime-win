//! Session runner — wires up a single client connection through the CheIME engine.
//!
//! Each connected TIP client gets a session runner that:
//! 1. Takes `FrontendMessage` from the pipe reader
//! 2. Feeds it to `Session::handle()`
//! 3. Writes resulting `EngineMessage`s back to the pipe writer
//!
//! This is the core per-client loop in the engine host.

use cheime_pipeline::{BuiltinPipeline, InputPipeline};
use cheime_protocol::{FrontendMessage, MessageHeader};
use cheime_session::Session;
use cheime_tip_core::{PipeError, PipeReader, PipeWriter};
use cheime_wire::MessageCodec;
use std::io::{Read, Write};
use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[allow(dead_code)]
pub enum RunnerError {
    #[error("pipe error: {0}")]
    Pipe(#[from] PipeError),
    #[error("session error: {0}")]
    Session(String),
}

/// Run the message loop for one client connection.
///
/// Reads frames from `reader`, dispatches to `session`, writes responses to `writer`.
/// Returns when the pipe is disconnected or an unrecoverable error occurs.
#[allow(dead_code)]
pub fn run_client_loop<R, W>(
    mut reader: PipeReader<R>,
    mut writer: PipeWriter<W>,
    codec: MessageCodec,
    pipeline: impl InputPipeline,
    identity: MessageHeader,
) -> Result<(), RunnerError>
where
    R: Read + Send,
    W: Write + Send,
{
    let mut session = Session::new(identity, pipeline);

    loop {
        // Read the next frontend message
        let msg: Option<FrontendMessage> = match reader.try_read_frame(&codec) {
            Ok(Some(msg)) => Some(msg),
            Ok(None) => {
                // Need more data — in real named pipe, this means the
                // caller should retry. For the test loop, we break.
                continue;
            }
            Err(PipeError::Disconnected) => break,
            Err(e) => return Err(RunnerError::Pipe(e)),
        };

        if let Some(msg) = msg {
            // Feed to session
            let outputs = session
                .handle(msg)
                .map_err(|e| RunnerError::Session(e.to_string()))?;

            // Write each engine message back
            for out in outputs {
                writer.write_message(&codec, &out)?;
            }
            writer.flush()?;
        }
    }

    Ok(())
}

/// Create a minimal BuiltinPipeline for testing.
#[allow(dead_code)]
pub fn test_pipeline() -> BuiltinPipeline {
    BuiltinPipeline::new([
        (String::from("ni"), String::from("你"), 100),
        (String::from("ni"), String::from("呢"), 50),
        (String::from("hao"), String::from("好"), 80),
        (String::from("nihao"), String::from("你好"), 120),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheime_model::{
        CORE_PROTOCOL_VERSION, ClientInstanceId, DeploymentGeneration, Key, KeyEvent, KeyState,
        Revision, Sequence, SessionEpoch, SessionId,
    };
    use cheime_protocol::{EngineMessage, MessageHeader};

    fn test_identity() -> MessageHeader {
        MessageHeader {
            protocol_version: CORE_PROTOCOL_VERSION,
            client: ClientInstanceId::new(1),
            session: SessionId::new(1),
            epoch: SessionEpoch::new(1),
            sequence: Sequence::new(0),
            revision: Revision::new(0),
            deployment: DeploymentGeneration::new(1),
        }
    }

    #[test]
    fn key_n_produces_snapshot() {
        let msg = FrontendMessage::KeyCommand {
            header: MessageHeader {
                sequence: Sequence::new(1),
                revision: Revision::new(0),
                ..test_identity()
            },
            event: KeyEvent {
                key: Key::Character('n'),
                state: KeyState::default(),
            },
        };

        let mut session = Session::new(test_identity(), test_pipeline());
        let outputs = session.handle(msg).unwrap();
        assert!(!outputs.is_empty());

        // Should contain a PlatformAction (SetPreedit) and a CandidateSnapshot
        let has_preedit = outputs.iter().any(|m| {
            matches!(m, EngineMessage::PlatformAction { action, .. }
                if matches!(&action.kind, cheime_model::PlatformActionKind::SetPreedit { text, .. } if text == "n"))
        });
        assert!(has_preedit);

        let has_snapshot = outputs
            .iter()
            .any(|m| matches!(m, EngineMessage::CandidateSnapshot { .. }));
        assert!(has_snapshot);
    }

    #[test]
    fn full_ni_commit_flow() {
        let mut session = Session::new(test_identity(), test_pipeline());

        // 'n'
        session
            .handle(FrontendMessage::KeyCommand {
                header: MessageHeader {
                    sequence: Sequence::new(1),
                    revision: Revision::new(0),
                    ..test_identity()
                },
                event: KeyEvent {
                    key: Key::Character('n'),
                    state: KeyState::default(),
                },
            })
            .unwrap();

        // 'i'
        session
            .handle(FrontendMessage::KeyCommand {
                header: MessageHeader {
                    sequence: Sequence::new(2),
                    revision: Revision::new(1),
                    ..test_identity()
                },
                event: KeyEvent {
                    key: Key::Character('i'),
                    state: KeyState::default(),
                },
            })
            .unwrap();

        // Enter to commit
        let commit_out = session
            .handle(FrontendMessage::KeyCommand {
                header: MessageHeader {
                    sequence: Sequence::new(3),
                    revision: Revision::new(2),
                    ..test_identity()
                },
                event: KeyEvent {
                    key: Key::Enter,
                    state: KeyState::default(),
                },
            })
            .unwrap();

        // Should have a commit action
        let commit_action = commit_out.iter().find_map(|m| match m {
            EngineMessage::PlatformAction { action, .. }
                if matches!(
                    &action.kind,
                    cheime_model::PlatformActionKind::Commit { .. }
                ) =>
            {
                Some(action.clone())
            }
            _ => None,
        });
        assert!(commit_action.is_some(), "expected a commit PlatformAction");

        // Confirm the commit
        let action_id = commit_action.unwrap().id;
        session
            .handle(FrontendMessage::PlatformActionResult {
                header: MessageHeader {
                    sequence: Sequence::new(4),
                    revision: Revision::new(2),
                    ..test_identity()
                },
                result: cheime_model::PlatformActionResult {
                    action_id,
                    outcome: cheime_model::PlatformActionOutcome::Applied,
                },
            })
            .unwrap();

        // Composition should be cleared
        assert_eq!(session.composition(), "");
    }
}
