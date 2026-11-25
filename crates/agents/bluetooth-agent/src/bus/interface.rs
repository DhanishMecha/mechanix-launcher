//! The D-Bus names the agent uses: where it is exported, the interface
//! bluetoothd calls, and the errors it answers with.

use mecha_wayland::prelude::*;

pub(crate) const BLUEZ: &str = "org.bluez";
/// A remote device's interface, for `CancelPairing`.
pub(crate) const DEVICE_IFACE: &str = "org.bluez.Device1";

/// Where the agent is exported.
pub const AGENT_PATH: &str = "/org/mechanix/bluez/Agent";
pub(crate) const AGENT_IFACE: &str = "org.bluez.Agent1";

/// The user said no, or the answer did not fit the request.
pub(crate) const REJECTED: &str = "org.bluez.Error.Rejected";
/// The request was dropped before it was answered.
pub(crate) const CANCELED: &str = "org.bluez.Error.Canceled";
pub(crate) const ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";

/// `org.bluez.Agent1`, as bluetoothd calls it.
pub(crate) fn agent_spec() -> InterfaceSpec {
    InterfaceSpec::new(AGENT_IFACE)
        .method("Release", "", "")
        .method("RequestPinCode", "o", "s")
        .method("DisplayPinCode", "os", "")
        .method("RequestPasskey", "o", "u")
        .method("DisplayPasskey", "ouq", "")
        .method("RequestConfirmation", "ou", "")
        .method("RequestAuthorization", "o", "")
        .method("AuthorizeService", "os", "")
        .method("Cancel", "", "")
}
