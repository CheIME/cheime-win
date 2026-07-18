//! Integration test: fake TIP → engine host roundtrip without real named pipes.
//!
//! Exercises the full protocol stack using in-memory byte channels:
//! 1. Encode KeyCommand via pipe writer
//! 2. Decode via pipe reader
//! 3. Feed to Session
//! 4. Verify PlatformAction + CandidateSnapshot response
//!
//! Does not require actual Windows named pipes — uses `Cursor<Vec<u8>>`.

use cheime_model::{
    CORE_PROTOCOL_VERSION, ClientInstanceId, DeploymentGeneration, Key, KeyEvent, KeyState,
    PlatformActionKind, Revision, Sequence, SessionEpoch, SessionId,
};
use cheime_pipeline::BuiltinPipeline;
use cheime_protocol::{EngineMessage, FrontendMessage, MessageHeader};
use cheime_session::Session;
use cheime_tip_core::{PipeReader, PipeWriter};
use cheime_wire::MessageCodec;
use std::io::Cursor;

fn codec() -> MessageCodec {
    MessageCodec::new(MessageCodec::DEFAULT_MAX)
}

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

fn test_pipeline() -> BuiltinPipeline {
    BuiltinPipeline::new([
        (String::from("ni"), String::from("你"), 100),
        (String::from("ni"), String::from("呢"), 50),
    ])
}

/// Write a message to a pipe writer, read it back from the reader, and verify it.
#[test]
fn single_key_roundtrip_and_response() {
    let c = codec();

    // 1. Build a KeyCommand
    let key_msg = FrontendMessage::KeyCommand {
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

    // 2. Write it through the pipe
    let buffer = Vec::new();
    let cursor = Cursor::new(buffer);
    let mut writer = PipeWriter::new(cursor);
    writer.write_message(&c, &key_msg).unwrap();
    writer.flush().unwrap();

    let written = writer.inner.into_inner();
    assert!(!written.is_empty(), "should have written bytes");

    // 3. Read it back
    let read_cursor = Cursor::new(written);
    let mut reader = PipeReader::new(read_cursor);
    let decoded: Option<FrontendMessage> = reader.try_read_frame(&c).unwrap();
    assert_eq!(decoded, Some(key_msg));
}

/// Full pipeline: encode → decode → Session → verify response.
#[test]
fn key_command_produces_expected_session_response() {
    let c = codec();

    // Encode a KeyCommand for 'n'
    let key_msg = FrontendMessage::KeyCommand {
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

    let buffer = Vec::new();
    let cursor = Cursor::new(buffer);
    let mut writer = PipeWriter::new(cursor);
    writer.write_message(&c, &key_msg).unwrap();
    let written = writer.inner.into_inner();

    let read_cursor = Cursor::new(written);
    let mut reader = PipeReader::new(read_cursor);
    let decoded: Option<FrontendMessage> = reader.try_read_frame(&c).unwrap();
    let decoded = decoded.unwrap();

    // Feed to session
    let mut session = Session::new(test_identity(), test_pipeline());
    let responses = session.handle(decoded).unwrap();

    // Should produce at least 2 messages: PlatformAction + CandidateSnapshot
    assert!(
        responses.len() >= 2,
        "expected at least PlatformAction + CandidateSnapshot"
    );

    let has_action = responses.iter().any(|m| {
        matches!(m, EngineMessage::PlatformAction { .. })
    });
    let has_snapshot = responses.iter().any(|m| {
        matches!(m, EngineMessage::CandidateSnapshot { .. })
    });

    assert!(has_action, "missing PlatformAction");
    assert!(has_snapshot, "missing CandidateSnapshot");
}

/// Verify that an encoded EngineMessage can be decoded correctly.
#[test]
fn engine_message_roundtrip_through_codec() {
    let c = codec();
    let snapshot = cheime_model::CandidateSnapshot {
        epoch: SessionEpoch::new(1),
        revision: Revision::new(1),
        deployment: DeploymentGeneration::new(1),
        preedit: String::from("ni"),
        cursor: 2,
        candidates: vec![cheime_model::Candidate {
            id: cheime_model::CandidateId::new(1),
            text: String::from("你"),
            annotation: Some(String::from("nǐ")),
            source: String::from("builtin"),
        }],
        highlighted: Some(cheime_model::CandidateId::new(1)),
        status: cheime_model::SessionStatus::Composing,
    };
    let msg = EngineMessage::CandidateSnapshot {
        header: test_identity(),
        snapshot,
    };

    let encoded = c.encode_engine(&msg).unwrap();
    let decoded = c.decode_engine(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

/// Verify commit flow: Enter commits, and confirming the commit clears composition.
#[test]
fn commit_and_confirm_via_session() {
    let mut session = Session::new(test_identity(), test_pipeline());

    // Type 'n', 'i'
    for (seq, ch, rev) in [(1, 'n', 0), (2, 'i', 1)] {
        session
            .handle(FrontendMessage::KeyCommand {
                header: MessageHeader {
                    sequence: Sequence::new(seq),
                    revision: Revision::new(rev),
                    ..test_identity()
                },
                event: KeyEvent {
                    key: Key::Character(ch),
                    state: KeyState::default(),
                },
            })
            .unwrap();
    }

    assert_eq!(session.composition(), "ni");

    // Enter → commit
    let outputs = session
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

    // Find commit action
    let commit = outputs.iter().find_map(|m| match m {
        EngineMessage::PlatformAction { action, .. }
            if matches!(&action.kind, PlatformActionKind::Commit { .. }) =>
        {
            Some(action.clone())
        }
        _ => None,
    });
    assert!(commit.is_some(), "should propose a commit");

    // Composition should NOT be cleared yet (pending confirmation)
    assert_eq!(session.composition(), "ni");

    // Confirm the commit
    let action_id = commit.unwrap().id;
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

    // Now composition should be clear
    assert_eq!(session.composition(), "");
}

/// Verify that a PlatformActionResult for an unknown action is rejected.
#[test]
fn unknown_action_rejected() {
    let mut session = Session::new(test_identity(), test_pipeline());
    let result = session.handle(FrontendMessage::PlatformActionResult {
        header: MessageHeader {
            sequence: Sequence::new(1),
            revision: Revision::new(0),
            ..test_identity()
        },
        result: cheime_model::PlatformActionResult {
            action_id: cheime_model::ActionId::new(999),
            outcome: cheime_model::PlatformActionOutcome::Applied,
        },
    });
    assert!(result.is_err());
}
