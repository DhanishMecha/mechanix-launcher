//! The remote device as the dialog shows it: its address at once, from
//! its object path, then its name and icon once `org.bluez.Device1`'s
//! properties are read.
//!
//! A read that fails, or that comes back after its request ended,
//! leaves the address shown.

use mecha_wayland::prelude::*;

use super::agent;
use super::interface::{BLUEZ, DEVICE_IFACE};
use crate::signals::DeviceInfo;

/// `GetAll` calls in flight, and the request each is for.
struct Reads {
    bus: BusId,
    calls: Vec<(CallId, u64)>,
}
impl Resource for Reads {}

pub(crate) fn install(app: &mut App, bus: BusId) {
    if app.has_resource::<Reads>() {
        return;
    }
    app.insert_resource(Reads {
        bus,
        calls: Vec::new(),
    });
    app.system(on_reply).system(on_disconnect);
}

/// What the path says until the device's properties are read:
/// `…/dev_AA_BB_CC_DD_EE_FF` is `AA:BB:CC:DD:EE:FF`.
pub(crate) fn from_path(path: ObjectPath) -> DeviceInfo {
    let address = path
        .as_str()
        .rsplit('/')
        .next()
        .and_then(|last| last.strip_prefix("dev_"))
        .map(|a| a.replace('_', ":"))
        .unwrap_or_else(|| path.as_str().to_owned());
    DeviceInfo {
        path,
        name: address.clone(),
        address,
        icon: None,
    }
}

/// Read the name and icon of request `id`'s device.
pub(crate) fn read(app: &mut App, id: u64, path: &ObjectPath) {
    let bus = app.resource::<Reads>().bus;
    let call = {
        let mut dbus = app.resource_mut::<DBus>();
        let Some(b) = dbus.bus(bus) else {
            return;
        };
        b.object(BLUEZ, path.clone()).get_all(DEVICE_IFACE)
    };
    app.resource_mut::<Reads>().calls.push((call, id));
}

/// `device` with what its properties say.
pub(crate) fn with_properties(mut device: DeviceInfo, props: &PropMap) -> DeviceInfo {
    let text = |key| match props.get_as::<String>(key) {
        Some(Ok(s)) if !s.trim().is_empty() => Some(s),
        _ => None,
    };
    // bluetoothd's Alias falls back to the address, dash-separated.
    let unnamed = device.address.replace(':', "-");
    if let Some(name) = text("Alias")
        .filter(|a| *a != unnamed && *a != device.address)
        .or_else(|| text("Name"))
    {
        device.name = name;
    }
    if let Some(address) = text("Address") {
        device.address = address;
    }
    device.icon = text("Icon");
    device
}

fn on_reply(app: &mut App, e: &MethodReply) {
    let id = {
        let mut reads = app.resource_mut::<Reads>();
        let Some(i) = reads.calls.iter().position(|(c, _)| *c == e.call) else {
            return;
        };
        reads.calls.remove(i).1
    };
    let props = match &e.result {
        Ok(msg) => match msg.body::<(PropMap,)>() {
            Ok((props,)) => props,
            Err(error) => {
                tracing::warn!("bluetooth: the device's properties: bad reply: {error:?}");
                return;
            }
        },
        Err(error) => {
            tracing::warn!("bluetooth: reading the device's properties: {error:?}");
            return;
        }
    };
    agent::update_device(app, id, |device| with_properties(device, &props));
}

fn on_disconnect(app: &mut App, e: &BusDisconnected) {
    let mut reads = app.resource_mut::<Reads>();
    if reads.bus == e.bus {
        reads.calls.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF";

    fn props(pairs: &[(&str, &str)]) -> PropMap {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::Str((*v).to_owned())))
            .collect()
    }

    fn phone() -> DeviceInfo {
        from_path(ObjectPath::from(PHONE))
    }

    #[test]
    fn the_path_gives_the_address() {
        let info = phone();
        assert_eq!(info.address, "AA:BB:CC:DD:EE:FF");
        assert_eq!(info.name, "AA:BB:CC:DD:EE:FF");
        assert_eq!(info.icon, None);
        assert_eq!(
            from_path(ObjectPath::from("/org/bluez")).address,
            "/org/bluez"
        );
    }

    #[test]
    fn the_alias_names_the_device() {
        let info = with_properties(
            phone(),
            &props(&[
                ("Alias", "Dhanish's OnePlus"),
                ("Name", "OnePlus 12"),
                ("Icon", "phone"),
            ]),
        );
        assert_eq!(info.name, "Dhanish's OnePlus");
        assert_eq!(info.icon.as_deref(), Some("phone"));
        assert_eq!(info.address, "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn an_alias_that_is_only_the_address_falls_back_to_the_name() {
        let info = with_properties(
            phone(),
            &props(&[("Alias", "AA-BB-CC-DD-EE-FF"), ("Name", "OnePlus 12")]),
        );
        assert_eq!(info.name, "OnePlus 12");
    }

    #[test]
    fn without_a_name_the_address_stays() {
        let info = with_properties(phone(), &props(&[("Alias", "AA-BB-CC-DD-EE-FF")]));
        assert_eq!(info.name, "AA:BB:CC:DD:EE:FF");
        let info = with_properties(phone(), &props(&[("Alias", "  "), ("Icon", "")]));
        assert_eq!(info.name, "AA:BB:CC:DD:EE:FF");
        assert_eq!(info.icon, None);
    }
}
