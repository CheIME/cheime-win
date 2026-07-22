//! Session runner — wires up a single client connection through the CheIME engine.
//!
//! Each connected TIP client gets a session runner that:
//! 1. Takes `FrontendMessage` from the pipe reader
//! 2. Feeds it to `Session::handle()`
//! 3. Writes resulting `EngineMessage`s back to the pipe writer

use cheime_pipeline::ComposablePipeline;
use cheime_protocol::{FrontendMessage, MessageHeader};
use cheime_session::Session;
use cheime_tip_core::{PipeReader, PipeWriter};
use cheime_wire::MessageCodec;
use std::io::{Read, Write};
use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RunnerError {
    #[error("pipe error: {0}")]
    Pipe(#[from] cheime_tip_core::PipeError),
    #[error("session error: {0}")]
    Session(String),
}

/// Run the message loop for one client connection.
pub fn run_client_loop<R, W>(
    mut reader: PipeReader<R>,
    mut writer: PipeWriter<W>,
    codec: MessageCodec,
    pipeline: ComposablePipeline,
    identity: MessageHeader,
) -> Result<(), RunnerError>
where
    R: Read + Send,
    W: Write + Send,
{
    let mut session = Session::new(identity, pipeline);

    loop {
        let message: FrontendMessage = reader
            .read_message(&codec)
            .map_err(RunnerError::Pipe)?
            .ok_or(RunnerError::Pipe(cheime_tip_core::PipeError::Disconnected))?;

        let responses = session
            .handle(message)
            .map_err(|e| RunnerError::Session(e.to_string()))?;

        for response in responses {
            writer.write_message(&codec, &response)?;
        }
        writer.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheime_dictionary::{CompiledIndex, DictEntry};
    use cheime_model::{
        CORE_PROTOCOL_VERSION, ClientInstanceId, DeploymentGeneration, Key, KeyEvent, KeyState,
        Revision, Sequence, SessionEpoch, SessionId,
    };
    use cheime_pipeline::factory::PipelineFactory;
    use cheime_user_data::UserStore;
    use parking_lot::Mutex;
    use std::sync::Arc;

    fn test_config() -> cheime_config::schema::SchemaConfig {
        serde_yaml::from_str(
            r#"schema_version: 1
engine:
  segmentors:
    - type: pinyin_syllable
  translators:
    - type: table
      dictionary: test_dict
  filters:
    - type: uniquifier
"#,
        )
        .unwrap()
    }

    fn test_index() -> Arc<CompiledIndex> {
        let entries = vec![
            DictEntry {
                text: "你".into(),
                code: "ni".into(),
                weight: Some(100),
                stem: None,
            },
            DictEntry {
                text: "好".into(),
                code: "hao".into(),
                weight: Some(100),
                stem: None,
            },
        ];
        Arc::new(CompiledIndex::build(entries, DeploymentGeneration::new(1)))
    }

    fn test_pipeline() -> ComposablePipeline {
        let config = test_config();
        let index = test_index();
        let store = Arc::new(Mutex::new(UserStore::new("test")));
        PipelineFactory::build(&config, Some(store), Some(index), None).unwrap()
    }

    fn test_identity() -> MessageHeader {
        MessageHeader {
            protocol_version: CORE_PROTOCOL_VERSION,
            client: ClientInstanceId::new(1),
            session: SessionId::new(1),
            epoch: SessionEpoch::new(2),
            sequence: Sequence::new(0),
            revision: Revision::new(0),
            deployment: DeploymentGeneration::new(1),
        }
    }

    #[test]
    fn session_handles_key_and_returns_candidates() {
        let pipeline = test_pipeline();
        let mut session = Session::new(test_identity(), pipeline);

        let mut header = test_identity();
        header.sequence = Sequence::new(1);
        let msg = FrontendMessage::KeyCommand {
            header,
            event: KeyEvent {
                key: Key::Character('n'),
                state: KeyState::default(),
            },
        };
        let responses = session.handle(msg).unwrap();
        assert!(
            !responses.is_empty(),
            "should return at least one candidate snapshot"
        );
    }
}
