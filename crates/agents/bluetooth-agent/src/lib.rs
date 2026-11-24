//! The session's BlueZ pairing agent, in the launcher's process.
//!
//! bluetoothd asks the default agent to confirm, enter or show a code,
//! or to authorize, whenever a device pairs or connects. This crate holds
//! the whole agent:
//!
//! - `signals`: what the parts below say to each other.

mod signals;

pub use signals::{
    DeviceInfo, PairingAbort, PairingAnswer, PairingCancelled, PairingRequested, PairingUpdated,
    Reply, Request, RequestKind, format_passkey, valid_passkey, valid_pin,
};
