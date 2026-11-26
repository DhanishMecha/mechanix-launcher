//! The signals between the agent's parts, and the request they carry.
//!
//! The bus side raises [`PairingRequested`], [`PairingDevice`],
//! [`PairingUpdated`] and [`PairingCancelled`]. The dialog answers with [`PairingAnswer`], or
//! [`PairingAbort`] to stop a pairing whose code is only being shown.

use mecha_wayland::prelude::*;

/// The largest passkey BlueZ takes or shows: six digits.
pub const MAX_PASSKEY: u32 = 999_999;
/// The longest legacy PIN BlueZ takes.
pub const MAX_PIN_LEN: usize = 16;

/// The remote device, as the dialog shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Its `org.bluez.Device1` object.
    pub path: ObjectPath,
    /// `XX:XX:XX:XX:XX:XX`.
    pub address: String,
    /// `Alias`, which falls back to `Name`, else the address.
    pub name: String,
    /// A freedesktop icon name, e.g. `phone`.
    pub icon: Option<String>,
}

/// What bluetoothd asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestKind {
    /// `RequestPinCode`: enter a legacy PIN. Answer [`Reply::PinCode`].
    PinCode,
    /// `DisplayPinCode`: show this PIN until cancelled.
    DisplayPinCode(String),
    /// `RequestPasskey`: enter the passkey. Answer [`Reply::Passkey`].
    Passkey,
    /// `DisplayPasskey`: show the passkey until cancelled; `entered`
    /// keys are typed on the remote side so far.
    DisplayPasskey { passkey: u32, entered: u16 },
    /// `RequestConfirmation`: does the passkey match the device's?
    Confirmation(u32),
    /// `RequestAuthorization`: allow an incoming pairing.
    Authorization,
    /// `AuthorizeService`: allow a connection to this service UUID.
    Service(String),
}

impl RequestKind {
    /// Only shown: bluetoothd took its reply already. It ends with
    /// `Cancel`, or with the device paired.
    pub fn is_display(&self) -> bool {
        matches!(
            self,
            RequestKind::DisplayPinCode(_) | RequestKind::DisplayPasskey { .. }
        )
    }
}

/// One call from bluetoothd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Tells this request from the ones before and after it.
    pub id: u64,
    pub device: DeviceInfo,
    pub kind: RequestKind,
}

/// The dialog's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    PinCode(String),
    Passkey(u32),
    /// Confirm or authorize.
    Accept,
    /// Refuse: `org.bluez.Error.Rejected`.
    Reject,
}

/// The active request: answer it with [`PairingAnswer`], or stop a
/// display with [`PairingAbort`].
#[derive(Debug, Clone)]
pub struct PairingRequested(pub Request);
impl Signal for PairingRequested {}

/// The remote side typed more keys of the shown passkey.
#[derive(Debug, Clone)]
pub struct PairingUpdated {
    pub id: u64,
    pub entered: u16,
}
impl Signal for PairingUpdated {}

/// The request's device was read: its name and icon, where the request
/// raised had only its address.
#[derive(Debug, Clone)]
pub struct PairingDevice {
    pub id: u64,
    pub device: DeviceInfo,
}
impl Signal for PairingDevice {}

/// bluetoothd cancelled the active request, or no longer needs its code
/// shown: it cancelled, or the device is paired.
#[derive(Debug, Clone)]
pub struct PairingCancelled {
    pub id: u64,
}
impl Signal for PairingCancelled {}

/// Answer the request with this id.
#[derive(Debug, Clone)]
pub struct PairingAnswer {
    pub id: u64,
    pub reply: Reply,
}
impl Signal for PairingAnswer {}

/// The user stopped the pairing whose code is shown: the bus side calls
/// the device's `CancelPairing`.
#[derive(Debug, Clone)]
pub struct PairingAbort {
    pub id: u64,
}
impl Signal for PairingAbort {}

/// A passkey as BlueZ shows it: six digits, zero-padded.
pub fn format_passkey(passkey: u32) -> String {
    format!("{passkey:06}")
}

/// A passkey BlueZ takes: 0 to 999999.
pub fn valid_passkey(passkey: u32) -> bool {
    passkey <= MAX_PASSKEY
}

/// A legacy PIN BlueZ takes: 1 to 16 ASCII letters or digits.
pub fn valid_pin(pin: &str) -> bool {
    (1..=MAX_PIN_LEN).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkeys_are_six_digits() {
        assert_eq!(format_passkey(0), "000000");
        assert_eq!(format_passkey(42), "000042");
        assert_eq!(format_passkey(MAX_PASSKEY), "999999");
    }

    #[test]
    fn passkeys_end_at_six_digits() {
        assert!(valid_passkey(0));
        assert!(valid_passkey(MAX_PASSKEY));
        assert!(!valid_passkey(MAX_PASSKEY + 1));
    }

    #[test]
    fn pins_are_one_to_sixteen_alphanumerics() {
        assert!(valid_pin("0"));
        assert!(valid_pin("abcDEF0123456789"));
        assert!(!valid_pin(""));
        assert!(!valid_pin("abcDEF01234567890"));
        assert!(!valid_pin("12 34"));
        assert!(!valid_pin("12é4"));
    }

    #[test]
    fn only_displays_are_display() {
        assert!(RequestKind::DisplayPinCode("1234".into()).is_display());
        assert!(
            RequestKind::DisplayPasskey {
                passkey: 1,
                entered: 0
            }
            .is_display()
        );
        assert!(!RequestKind::Confirmation(1).is_display());
        assert!(!RequestKind::PinCode.is_display());
    }
}
