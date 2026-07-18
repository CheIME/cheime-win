//! CheIME Windows Frontend — integration test suite.
//!
//! Tests the full round-trip from TIP→pipe→engine→pipe→TIP
//! without requiring actual Windows named pipes or TSF.

use cheime_model::{CORE_PROTOCOL_VERSION, ClientInstanceId, DeploymentGeneration, Revision, Sequence, SessionEpoch, SessionId};
use cheime_protocol::MessageHeader;

/// Create a minimal MessageHeader for testing.
pub fn test_header(sequence: u64, revision: u64) -> MessageHeader {
    MessageHeader {
        protocol_version: CORE_PROTOCOL_VERSION,
        client: ClientInstanceId::new(1),
        session: SessionId::new(1),
        epoch: SessionEpoch::new(1),
        sequence: Sequence::new(sequence),
        revision: Revision::new(revision),
        deployment: DeploymentGeneration::new(1),
    }
}
