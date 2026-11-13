//! Peer authentication: a three-message mutual handshake keyed by the cluster secret.
//!
//! ```text
//! initiator                                   responder
//!   Hello{version, node_i, nonce_i, tag}  ->
//!                                         <-  HelloAck{version, node_r, nonce_r, tag}
//!   HelloConfirm{tag}                     ->
//! ```
//!
//! Tags are HMAC-SHA256 over a per-message label, the protocol version, the sender identity,
//! the sender nonce and the peer nonce. The ack binds the initiator's nonce and the confirm
//! binds the responder's nonce, so both sides prove they hold the secret *now* (no replay).
//! Verification is constant-time (`hmac::Mac::verify_slice`). Nonces are inputs: the caller
//! supplies 16 fresh random bytes. The handshake authenticates peers; confidentiality and
//! integrity of the following traffic are the transport's job (TLS).

use crate::error::ClusterError;
use crate::message::{Hello, Message};
use crate::node::{NodeId, ProtocolVersion, PROTOCOL_VERSION};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Minimum length of the cluster secret in bytes.
pub const MIN_SECRET_BYTES: usize = 16;

const LABEL_HELLO: &[u8] = b"mt-cluster/hello/v1";
const LABEL_ACK: &[u8] = b"mt-cluster/ack/v1";
const LABEL_CONFIRM: &[u8] = b"mt-cluster/confirm/v1";

/// The shared cluster secret. `Debug` never prints the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ClusterSecret(Vec<u8>);

impl ClusterSecret {
    /// Wraps a secret; refuses secrets shorter than [`MIN_SECRET_BYTES`].
    pub fn new(bytes: &[u8]) -> Result<Self, ClusterError> {
        if bytes.len() < MIN_SECRET_BYTES {
            return Err(ClusterError::SecretTooShort);
        }
        Ok(Self(bytes.to_vec()))
    }
}

impl core::fmt::Debug for ClusterSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ClusterSecret(<redacted>)")
    }
}

fn mac(
    secret: &ClusterSecret,
    label: &[u8],
    protocol: ProtocolVersion,
    node: &NodeId,
    nonce: &[u8; 16],
    peer_nonce: &[u8; 16],
) -> Result<Hmac<Sha256>, ClusterError> {
    let mut m =
        Hmac::<Sha256>::new_from_slice(&secret.0).map_err(|_| ClusterError::SecretTooShort)?;
    m.update(label);
    m.update(&protocol.major.to_be_bytes());
    m.update(&protocol.minor.to_be_bytes());
    m.update(&node.id.to_be_bytes());
    m.update(&node.incarnation.to_be_bytes());
    m.update(
        &u64::try_from(node.addr.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    m.update(node.addr.as_bytes());
    m.update(nonce);
    m.update(peer_nonce);
    Ok(m)
}

fn tag(m: Hmac<Sha256>) -> [u8; 32] {
    m.finalize().into_bytes().into()
}

fn verify(m: Hmac<Sha256>, tag: &[u8; 32]) -> Result<(), ClusterError> {
    m.verify_slice(tag).map_err(|_| ClusterError::AuthFailed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    AwaitHello,
    AwaitAck,
    AwaitConfirm {
        peer: NodeId,
        peer_nonce: [u8; 16],
        peer_protocol: ProtocolVersion,
    },
    Established(NodeId),
    Failed,
}

/// Handshake state machine for one connection.
#[derive(Debug, Clone)]
pub struct Handshake {
    secret: ClusterSecret,
    me: NodeId,
    nonce: [u8; 16],
    state: State,
}

impl Handshake {
    /// Starts a handshake as the connecting side; returns the `Hello` to send.
    pub fn initiator(
        secret: ClusterSecret,
        me: NodeId,
        nonce: [u8; 16],
    ) -> Result<(Self, Message), ClusterError> {
        let t = tag(mac(
            &secret,
            LABEL_HELLO,
            PROTOCOL_VERSION,
            &me,
            &nonce,
            &[0; 16],
        )?);
        let hello = Message::Hello(Hello {
            protocol: PROTOCOL_VERSION,
            node: me.clone(),
            nonce,
            tag: t,
        });
        Ok((
            Self {
                secret,
                me,
                nonce,
                state: State::AwaitAck,
            },
            hello,
        ))
    }

    /// Starts a handshake as the accepting side; it waits for a `Hello`.
    #[must_use]
    pub fn responder(secret: ClusterSecret, me: NodeId, nonce: [u8; 16]) -> Self {
        Self {
            secret,
            me,
            nonce,
            state: State::AwaitHello,
        }
    }

    /// The authenticated peer, once the handshake is complete.
    #[must_use]
    pub fn peer(&self) -> Option<&NodeId> {
        match &self.state {
            State::Established(p) => Some(p),
            State::AwaitHello | State::AwaitAck | State::AwaitConfirm { .. } | State::Failed => {
                None
            }
        }
    }

    /// Feeds the next message from the peer. Returns the reply to send, if any.
    /// After any error the handshake is failed for good and the connection must be closed.
    pub fn handle(&mut self, msg: &Message) -> Result<Option<Message>, ClusterError> {
        let state = core::mem::replace(&mut self.state, State::Failed);
        let (next, reply) = self.step(state, msg)?;
        self.state = next;
        Ok(reply)
    }

    fn check_peer(&self, h: &Hello) -> Result<(), ClusterError> {
        if !PROTOCOL_VERSION.is_compatible(h.protocol) {
            return Err(ClusterError::IncompatibleProtocol {
                ours: PROTOCOL_VERSION,
                theirs: h.protocol,
            });
        }
        if h.node.id == self.me.id {
            return Err(ClusterError::SelfConnection);
        }
        Ok(())
    }

    fn step(&self, state: State, msg: &Message) -> Result<(State, Option<Message>), ClusterError> {
        match state {
            State::AwaitHello => {
                let Message::Hello(h) = msg else {
                    return Err(ClusterError::UnexpectedHandshake);
                };
                self.check_peer(h)?;
                verify(
                    mac(
                        &self.secret,
                        LABEL_HELLO,
                        h.protocol,
                        &h.node,
                        &h.nonce,
                        &[0; 16],
                    )?,
                    &h.tag,
                )?;
                let t = tag(mac(
                    &self.secret,
                    LABEL_ACK,
                    PROTOCOL_VERSION,
                    &self.me,
                    &self.nonce,
                    &h.nonce,
                )?);
                let ack = Message::HelloAck(Hello {
                    protocol: PROTOCOL_VERSION,
                    node: self.me.clone(),
                    nonce: self.nonce,
                    tag: t,
                });
                Ok((
                    State::AwaitConfirm {
                        peer: h.node.clone(),
                        peer_nonce: h.nonce,
                        peer_protocol: h.protocol,
                    },
                    Some(ack),
                ))
            }
            State::AwaitAck => {
                let Message::HelloAck(h) = msg else {
                    return Err(ClusterError::UnexpectedHandshake);
                };
                self.check_peer(h)?;
                verify(
                    mac(
                        &self.secret,
                        LABEL_ACK,
                        h.protocol,
                        &h.node,
                        &h.nonce,
                        &self.nonce,
                    )?,
                    &h.tag,
                )?;
                let t = tag(mac(
                    &self.secret,
                    LABEL_CONFIRM,
                    PROTOCOL_VERSION,
                    &self.me,
                    &self.nonce,
                    &h.nonce,
                )?);
                Ok((
                    State::Established(h.node.clone()),
                    Some(Message::HelloConfirm { tag: t }),
                ))
            }
            State::AwaitConfirm {
                peer,
                peer_nonce,
                peer_protocol,
            } => {
                let Message::HelloConfirm { tag: t } = msg else {
                    return Err(ClusterError::UnexpectedHandshake);
                };
                verify(
                    mac(
                        &self.secret,
                        LABEL_CONFIRM,
                        peer_protocol,
                        &peer,
                        &peer_nonce,
                        &self.nonce,
                    )?,
                    t,
                )?;
                Ok((State::Established(peer), None))
            }
            State::Established(_) | State::Failed => Err(ClusterError::UnexpectedHandshake),
        }
    }
}
