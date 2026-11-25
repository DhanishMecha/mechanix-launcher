use dbus::fake::FakeBus;

const BLUETOOTHD: &str = ":1.7";

use super::*;
use crate::bus::agent;

fn installed() -> FakeBus {
    let mut bus = FakeBus::new();
    bus.owners.insert(BLUEZ.into(), BLUETOOTHD.into());
    assert!(agent::install(&mut bus.app, BusId::Session));
    install(&mut bus.app, BusId::Session);
    bus.turn();
    bus
}

fn expect_manager_call(bus: &mut FakeBus, member: &str) -> Message {
    let call = bus.expect_call(member);
    assert_eq!(call.destination().unwrap().as_str(), BLUEZ);
    assert_eq!(call.path().unwrap().as_str(), MANAGER_PATH);
    assert_eq!(call.interface().unwrap().as_str(), AGENT_MANAGER);
    call
}

fn expect_register(bus: &mut FakeBus) -> Message {
    let call = expect_manager_call(bus, "RegisterAgent");
    let (path, capability) = call.body::<(ObjectPath, String)>().unwrap();
    assert_eq!(path.as_str(), AGENT_PATH);
    assert_eq!(capability, CAPABILITY);
    call
}

fn expect_request_default(bus: &mut FakeBus) -> Message {
    let call = expect_manager_call(bus, "RequestDefaultAgent");
    let (path,) = call.body::<(ObjectPath,)>().unwrap();
    assert_eq!(path.as_str(), AGENT_PATH);
    call
}

fn owner_changed(bus: &mut FakeBus, old: &str, new: &str) {
    bus.send(
        Message::signal("/org/freedesktop/DBus", DAEMON, "NameOwnerChanged")
            .with_sender(DAEMON)
            .with_body(&(BLUEZ, old, new)),
    );
    bus.turn();
}

fn agents_bluetoothd(bus: &FakeBus) -> Option<String> {
    agent::bluetoothd(&bus.app).map(|n| n.as_str().to_owned())
}

/// Registered with [`BLUETOOTHD`] and its default agent.
fn registered() -> FakeBus {
    let mut bus = installed();
    let call = expect_register(&mut bus);
    bus.reply(&call, &());
    bus.turn();
    let call = expect_request_default(&mut bus);
    bus.reply(&call, &());
    bus.turn();
    bus.received();
    bus
}

#[test]
fn registers_then_asks_to_be_the_default() {
    let bus = registered();
    assert_eq!(bus.app.resource::<Registration>().call, None);
    // The agent takes calls from whoever answered.
    assert_eq!(agents_bluetoothd(&bus).as_deref(), Some(BLUETOOTHD));
}

#[test]
fn a_refused_default_stays_registered() {
    let mut bus = installed();
    let call = expect_register(&mut bus);
    bus.reply(&call, &());
    bus.turn();
    let call = expect_request_default(&mut bus);
    bus.error(&call, "org.bluez.Error.AlreadyExists", "Already exists");
    bus.turn();
    assert_eq!(bus.app.resource::<Registration>().call, None);
    assert_eq!(agents_bluetoothd(&bus).as_deref(), Some(BLUETOOTHD));
}

#[test]
fn a_refused_registration_is_given_up() {
    let mut bus = installed();
    let call = expect_register(&mut bus);
    bus.received();
    bus.error(&call, "org.bluez.Error.AlreadyExists", "Already exists");
    bus.turn();
    assert_eq!(bus.app.resource::<Registration>().call, None);
    assert_eq!(agents_bluetoothd(&bus), None);
    assert!(bus.received().is_empty());
}

#[test]
fn bluetoothd_starting_for_the_registration_is_not_a_restart() {
    let mut bus = installed();
    let call = expect_register(&mut bus);
    // The registration activated bluetoothd: its name appears first.
    owner_changed(&mut bus, "", BLUETOOTHD);
    bus.reply(&call, &());
    bus.turn();
    assert!(
        !bus.received()
            .iter()
            .any(|m| m.member().is_some_and(|m| m.as_str() == "RegisterAgent"))
    );
    assert_eq!(agents_bluetoothd(&bus).as_deref(), Some(BLUETOOTHD));
}

#[test]
fn a_restarted_bluetoothd_is_registered_with_again() {
    let mut bus = registered();

    owner_changed(&mut bus, BLUETOOTHD, "");
    assert_eq!(agents_bluetoothd(&bus), None);
    assert!(bus.received().is_empty());

    bus.owners.insert(BLUEZ.into(), ":1.9".into());
    owner_changed(&mut bus, "", ":1.9");
    let call = expect_register(&mut bus);
    bus.reply(&call, &());
    bus.turn();
    assert_eq!(agents_bluetoothd(&bus).as_deref(), Some(":1.9"));
    expect_request_default(&mut bus);
}

#[test]
fn bluetoothd_appearing_late_is_registered_with() {
    let mut bus = installed();
    let call = expect_register(&mut bus);
    bus.received();
    bus.error(
        &call,
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.bluez was not provided",
    );
    bus.turn();

    owner_changed(&mut bus, "", BLUETOOTHD);
    let call = expect_register(&mut bus);
    bus.reply(&call, &());
    bus.turn();
    assert_eq!(agents_bluetoothd(&bus).as_deref(), Some(BLUETOOTHD));
}
