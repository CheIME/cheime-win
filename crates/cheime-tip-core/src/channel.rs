use cheime_model::{CandidateSnapshot, PlatformAction};
use cheime_protocol::FrontendMessage;
use std::sync::mpsc;

/// Bounded channel for TSF→I/O thread communication.
///
/// TSF callbacks (on the host UI thread) push `FrontendMessage` values
/// via `try_send`. The dedicated I/O thread receives them and writes
/// framed messages to the named pipe.
pub struct TipChannel {
    sender: mpsc::SyncSender<FrontendMessage>,
    receiver: Option<mpsc::Receiver<FrontendMessage>>,
}

impl TipChannel {
    /// Create a new bounded channel with the given capacity.
    pub fn new(bound: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel(bound);
        Self {
            sender,
            receiver: Some(receiver),
        }
    }

    /// Try to enqueue a message without blocking.
    ///
    /// Returns `Ok(())` on success, or `Err(TrySendError)` if the channel
    /// is full or the receiver has been dropped.
    pub fn try_send(
        &self,
        msg: FrontendMessage,
    ) -> Result<(), mpsc::TrySendError<FrontendMessage>> {
        self.sender.try_send(msg)
    }

    /// Clone the underlying `SyncSender` for use from window callbacks.
    pub fn clone_sender(&self) -> mpsc::SyncSender<FrontendMessage> {
        self.sender.clone()
    }

    /// Take the receiver half of the channel.
    ///
    /// Returns `None` if already taken. The receiver should be moved
    /// to the I/O thread.
    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<FrontendMessage>> {
        self.receiver.take()
    }
}

/// Messages dispatched from the I/O thread back to the TIP (UI thread)
/// via `PostMessageW` with custom window messages.
#[derive(Clone, Debug)]
pub enum DispatchMessage {
    /// New candidate snapshot to render.
    Snapshot(CandidateSnapshot),
    /// Platform action to apply in a TSF edit session.
    PlatformAction(PlatformAction),
    /// Connection/engine status notification.
    Status { connected: bool, detail: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheime_model::{
        CORE_PROTOCOL_VERSION, ClientInstanceId, DeploymentGeneration, Key, KeyEvent, KeyState,
        Revision, Sequence, SessionEpoch, SessionId,
    };
    use cheime_protocol::MessageHeader;

    fn key_message(c: char, seq: u64) -> FrontendMessage {
        FrontendMessage::KeyCommand {
            header: MessageHeader {
                protocol_version: CORE_PROTOCOL_VERSION,
                client: ClientInstanceId::new(1),
                session: SessionId::new(2),
                epoch: SessionEpoch::new(3),
                sequence: Sequence::new(seq),
                revision: Revision::new(5),
                deployment: DeploymentGeneration::new(6),
            },
            event: KeyEvent {
                key: Key::Character(c),
                state: KeyState::default(),
            },
        }
    }

    #[test]
    fn send_and_receive_preserves_order() {
        let mut channel = TipChannel::new(16);
        let rx = channel.take_receiver().unwrap();

        channel.try_send(key_message('n', 1)).unwrap();
        channel.try_send(key_message('i', 2)).unwrap();
        channel.try_send(key_message('h', 3)).unwrap();

        let msgs: Vec<FrontendMessage> = rx.try_iter().collect();
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn bounded_channel_rejects_when_full() {
        let channel = TipChannel::new(1);
        // Fill the channel
        channel.try_send(key_message('a', 1)).unwrap();
        // Second send should fail
        let result = channel.try_send(key_message('b', 2));
        assert!(result.is_err());
    }

    #[test]
    fn receiver_taken_twice_returns_none() {
        let mut channel = TipChannel::new(4);
        assert!(channel.take_receiver().is_some());
        assert!(channel.take_receiver().is_none());
    }
}
