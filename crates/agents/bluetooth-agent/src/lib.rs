//! The session's BlueZ pairing agent, in the launcher's process.
//!
//! bluetoothd asks the default agent to confirm, enter or show a code,
//! or to authorize, whenever a device pairs or connects. This crate holds
//! the whole agent:
//!
//! - `signals`: what the parts below say to each other;
//! - `bus`: the exported agent, and its registration with bluetoothd;
//! - `dialog`: the pairing sheet.

mod bus;
mod dialog;
mod signals;

use mecha_wayland::prelude::*;

pub use bus::interface::AGENT_PATH;
pub use signals::{
    DeviceInfo, MAX_PASSKEY, MAX_PIN_LEN, PairingAbort, PairingAnswer, PairingCancelled,
    PairingRequested, PairingUpdated, Reply, Request, RequestKind, format_passkey, valid_passkey,
    valid_pin,
};

/// The Bluetooth pairing agent. Installs after `RingModule`,
/// `DBusModule` with the system bus, the window and presentation
/// modules, the theme, the `Atlas` and the `FontBook`; and before the
/// `KeyboardModule`, so the on-screen keyboard shows over the dialog.
pub struct BluetoothAgentModule;

impl Module for BluetoothAgentModule {
    fn install(self, app: &mut App) {
        if !bus::agent::install(app, BusId::System) {
            return;
        }
        dialog::install(app);
        bus::register::install(app, BusId::System);
    }
}
