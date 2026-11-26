use dbus::Body;
use dbus::fake::{FakeBus, UNIQUE};

use super::*;
use crate::bus::interface::{AGENT_IFACE, BLUEZ, DEVICE_IFACE};
use crate::signals::MAX_PASSKEY;

const BLUETOOTHD: &str = ":1.7";
const PHONE: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF";
const WATCH: &str = "/org/bluez/hci0/dev_11_22_33_44_55_66";

/// The signals the agent raised, in order.
#[derive(Default)]
struct Seen(Vec<Saw>);
impl Resource for Seen {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Saw {
    Requested(Request),
    Updated(u64, u16),
    Cancelled(u64),
    /// A device read: its name.
    Device(u64, String),
}

fn agent() -> FakeBus {
    let mut bus = FakeBus::new();
    assert!(install(&mut bus.app, BusId::Session));
    bus.app
        .init_resource::<Seen>()
        .system(|app: &mut App, e: &PairingRequested| {
            app.resource_mut::<Seen>()
                .0
                .push(Saw::Requested(e.0.clone()));
        })
        .system(|app: &mut App, e: &PairingUpdated| {
            app.resource_mut::<Seen>()
                .0
                .push(Saw::Updated(e.id, e.entered));
        })
        .system(|app: &mut App, e: &PairingCancelled| {
            app.resource_mut::<Seen>().0.push(Saw::Cancelled(e.id));
        })
        .system(|app: &mut App, e: &PairingDevice| {
            app.resource_mut::<Seen>()
                .0
                .push(Saw::Device(e.id, e.device.name.clone()));
        });
    set_bluetoothd(&mut bus.app, Some(BusName::new(BLUETOOTHD).unwrap()));
    bus.turn();
    bus.received();
    bus
}

fn seen(bus: &mut FakeBus) -> Vec<Saw> {
    std::mem::take(&mut bus.app.resource_mut::<Seen>().0)
}

fn call_from(sender: &'static str, member: &'static str) -> Message {
    Message::method_call(AGENT_PATH, member)
        .with_interface(AGENT_IFACE)
        .with_destination(UNIQUE)
        .with_sender(sender)
}

fn send<B: Body>(bus: &mut FakeBus, member: &'static str, body: &B) -> u32 {
    let serial = bus.send(call_from(BLUETOOTHD, member).with_body(body));
    bus.turn();
    serial
}

fn send_empty(bus: &mut FakeBus, member: &'static str) -> u32 {
    let serial = bus.send(call_from(BLUETOOTHD, member));
    bus.turn();
    serial
}

fn path(p: &'static str) -> ObjectPath {
    ObjectPath::from(p)
}

fn answer_with(bus: &mut FakeBus, id: u64, reply: Reply) {
    bus.app.signal(PairingAnswer { id, reply });
    bus.turn();
}

/// The answer to the call with `serial`, if any.
fn answer(received: &[Message], serial: u32) -> Option<&Message> {
    received.iter().find(|m| m.reply_serial() == Some(serial))
}

fn error_name(m: &Message) -> &str {
    m.error_name().map_or("", |n| n.as_str())
}

/// The one request raised since last asked.
fn requested(bus: &mut FakeBus) -> Request {
    match seen(bus).as_slice() {
        [Saw::Requested(r)] => r.clone(),
        other => panic!("expected one request, saw {other:?}"),
    }
}

/// The `GetAll` the agent sent for `device`'s properties.
fn expect_read(bus: &mut FakeBus, device: &str) -> Message {
    let call = bus.expect_call("GetAll");
    assert_eq!(call.destination().unwrap().as_str(), BLUEZ);
    assert_eq!(call.path().unwrap().as_str(), device);
    assert_eq!(call.body::<(String,)>().unwrap().0, DEVICE_IFACE);
    call
}

fn named(alias: &str) -> PropMap {
    PropMap::from([
        ("Alias".to_owned(), Value::Str(alias.into())),
        ("Icon".to_owned(), Value::Str("phone".into())),
    ])
}

#[test]
fn a_request_reads_its_device_and_names_it() {
    let mut bus = agent();
    send(&mut bus, "RequestConfirmation", &(path(PHONE), 42u32));
    let request = requested(&mut bus);
    // Raised at once, with the address.
    assert_eq!(request.device.name, "AA:BB:CC:DD:EE:FF");

    let read = expect_read(&mut bus, PHONE);
    bus.reply(&read, &(named("OnePlus 12"),));
    bus.turn();
    assert_eq!(
        seen(&mut bus),
        [Saw::Device(request.id, "OnePlus 12".into())]
    );
    let agent = bus.app.resource::<Agent>();
    let kept = &agent.active.as_ref().unwrap().request.device;
    assert_eq!(kept.name, "OnePlus 12");
    assert_eq!(kept.icon.as_deref(), Some("phone"));
}

#[test]
fn a_shown_code_reads_its_device_too() {
    let mut bus = agent();
    send(&mut bus, "DisplayPinCode", &(path(WATCH), "1234"));
    let id = requested(&mut bus).id;
    let read = expect_read(&mut bus, WATCH);
    bus.reply(&read, &(named("Watch"),));
    bus.turn();
    assert_eq!(seen(&mut bus), [Saw::Device(id, "Watch".into())]);
}

#[test]
fn a_failed_read_keeps_the_address() {
    let mut bus = agent();
    send(&mut bus, "RequestAuthorization", &(path(PHONE),));
    requested(&mut bus);
    let read = expect_read(&mut bus, PHONE);
    bus.error(&read, "org.freedesktop.DBus.Error.UnknownObject", "gone");
    bus.turn();
    assert!(seen(&mut bus).is_empty());
}

#[test]
fn a_read_after_the_request_ended_is_dropped() {
    let mut bus = agent();
    send(&mut bus, "RequestAuthorization", &(path(PHONE),));
    requested(&mut bus);
    let read = expect_read(&mut bus, PHONE);
    send_empty(&mut bus, "Cancel");
    seen(&mut bus);
    bus.reply(&read, &(named("OnePlus 12"),));
    bus.turn();
    assert!(seen(&mut bus).is_empty());
}

#[test]
fn only_bluetoothd_may_call() {
    let mut bus = agent();
    let serial = bus.send(call_from(":1.99", "RequestAuthorization").with_body(&(path(PHONE),)));
    bus.turn();
    let received = bus.received();
    assert_eq!(
        error_name(answer(&received, serial).unwrap()),
        ACCESS_DENIED
    );
    assert!(seen(&mut bus).is_empty());
}

#[test]
fn no_calls_are_taken_before_registration() {
    let mut bus = agent();
    set_bluetoothd(&mut bus.app, None);
    let serial = send(&mut bus, "RequestAuthorization", &(path(PHONE),));
    let received = bus.received();
    assert_eq!(
        error_name(answer(&received, serial).unwrap()),
        ACCESS_DENIED
    );
}

/// A call to send, and the request it should raise.
type Case = (&'static str, fn(&mut FakeBus) -> u32, RequestKind);

#[test]
fn each_request_is_decoded_and_kept_unanswered() {
    let cases: [Case; 5] = [
        (
            "RequestPinCode",
            |b| send(b, "RequestPinCode", &(path(PHONE),)),
            RequestKind::PinCode,
        ),
        (
            "RequestPasskey",
            |b| send(b, "RequestPasskey", &(path(PHONE),)),
            RequestKind::Passkey,
        ),
        (
            "RequestConfirmation",
            |b| send(b, "RequestConfirmation", &(path(PHONE), 42u32)),
            RequestKind::Confirmation(42),
        ),
        (
            "RequestAuthorization",
            |b| send(b, "RequestAuthorization", &(path(PHONE),)),
            RequestKind::Authorization,
        ),
        (
            "AuthorizeService",
            |b| {
                send(
                    b,
                    "AuthorizeService",
                    &(path(PHONE), "0000110b-0000-1000-8000-00805f9b34fb"),
                )
            },
            RequestKind::Service("0000110b-0000-1000-8000-00805f9b34fb".into()),
        ),
    ];
    for (member, make, kind) in cases {
        let mut bus = agent();
        let serial = make(&mut bus);
        assert!(answer(&bus.received(), serial).is_none(), "{member}");
        let r = requested(&mut bus);
        assert_eq!(r.kind, kind, "{member}");
        assert_eq!(r.device.path, PHONE, "{member}");
        assert_eq!(r.device.address, "AA:BB:CC:DD:EE:FF", "{member}");
    }
}

#[test]
fn a_pin_is_answered() {
    let mut bus = agent();
    let serial = send(&mut bus, "RequestPinCode", &(path(PHONE),));
    let id = requested(&mut bus).id;
    answer_with(&mut bus, id, Reply::PinCode("0000".into()));
    let received = bus.received();
    let reply = answer(&received, serial).unwrap();
    assert_eq!(reply.kind(), MessageKind::MethodReturn);
    assert_eq!(reply.body::<(String,)>().unwrap().0, "0000");
}

#[test]
fn a_passkey_is_answered() {
    let mut bus = agent();
    let serial = send(&mut bus, "RequestPasskey", &(path(PHONE),));
    let id = requested(&mut bus).id;
    answer_with(&mut bus, id, Reply::Passkey(123_456));
    let received = bus.received();
    let reply = answer(&received, serial).unwrap();
    assert_eq!(reply.body::<(u32,)>().unwrap().0, 123_456);
}

#[test]
fn a_confirmation_is_accepted_or_rejected() {
    let mut bus = agent();
    let serial = send(&mut bus, "RequestConfirmation", &(path(PHONE), 42u32));
    let id = requested(&mut bus).id;
    answer_with(&mut bus, id, Reply::Accept);
    let received = bus.received();
    assert_eq!(
        answer(&received, serial).unwrap().kind(),
        MessageKind::MethodReturn
    );

    let serial = send(&mut bus, "RequestConfirmation", &(path(PHONE), 42u32));
    let id = requested(&mut bus).id;
    answer_with(&mut bus, id, Reply::Reject);
    let received = bus.received();
    assert_eq!(error_name(answer(&received, serial).unwrap()), REJECTED);
}

#[test]
fn an_invalid_or_unfitting_answer_is_rejected() {
    for reply in [
        Reply::PinCode(String::new()),
        Reply::PinCode("12 34".into()),
        Reply::Passkey(123),
        Reply::Accept,
    ] {
        let mut bus = agent();
        let serial = send(&mut bus, "RequestPinCode", &(path(PHONE),));
        let id = requested(&mut bus).id;
        answer_with(&mut bus, id, reply.clone());
        let received = bus.received();
        assert_eq!(
            error_name(answer(&received, serial).unwrap()),
            REJECTED,
            "{reply:?}"
        );
    }
    let mut bus = agent();
    let serial = send(&mut bus, "RequestPasskey", &(path(PHONE),));
    let id = requested(&mut bus).id;
    answer_with(&mut bus, id, Reply::Passkey(MAX_PASSKEY + 1));
    let received = bus.received();
    assert_eq!(error_name(answer(&received, serial).unwrap()), REJECTED);
}

#[test]
fn an_answer_for_another_id_changes_nothing() {
    let mut bus = agent();
    let serial = send(&mut bus, "RequestAuthorization", &(path(PHONE),));
    let id = requested(&mut bus).id;
    answer_with(&mut bus, id + 100, Reply::Accept);
    assert!(answer(&bus.received(), serial).is_none());
    answer_with(&mut bus, id, Reply::Accept);
    assert!(answer(&bus.received(), serial).is_some());
}

#[test]
fn a_second_request_is_refused_while_one_is_open() {
    let mut bus = agent();
    let first = send(&mut bus, "RequestConfirmation", &(path(PHONE), 1u32));
    let id = requested(&mut bus).id;
    let second = send(&mut bus, "RequestAuthorization", &(path(WATCH),));
    let received = bus.received();
    assert_eq!(error_name(answer(&received, second).unwrap()), CANCELED);
    assert!(answer(&received, first).is_none());
    assert!(seen(&mut bus).is_empty());

    // The first is still answerable.
    answer_with(&mut bus, id, Reply::Accept);
    assert!(answer(&bus.received(), first).is_some());
}

#[test]
fn cancel_ends_the_request_and_the_shown_code() {
    let mut bus = agent();
    let serial = send(&mut bus, "RequestConfirmation", &(path(PHONE), 1u32));
    let request = requested(&mut bus).id;
    send(&mut bus, "DisplayPinCode", &(path(WATCH), "1234"));
    let shown = requested(&mut bus).id;
    bus.received();

    let cancel = send_empty(&mut bus, "Cancel");
    let received = bus.received();
    assert_eq!(
        answer(&received, cancel).unwrap().kind(),
        MessageKind::MethodReturn
    );
    assert_eq!(error_name(answer(&received, serial).unwrap()), CANCELED);
    assert_eq!(
        seen(&mut bus),
        [Saw::Cancelled(request), Saw::Cancelled(shown)]
    );

    // A late answer goes nowhere.
    answer_with(&mut bus, request, Reply::Accept);
    assert!(bus.received().is_empty());
}

#[test]
fn release_ends_everything_and_stops_taking_calls() {
    let mut bus = agent();
    let serial = send(&mut bus, "RequestAuthorization", &(path(PHONE),));
    let id = requested(&mut bus).id;
    let release = send_empty(&mut bus, "Release");
    let received = bus.received();
    assert_eq!(
        answer(&received, release).unwrap().kind(),
        MessageKind::MethodReturn
    );
    assert_eq!(error_name(answer(&received, serial).unwrap()), CANCELED);
    assert_eq!(seen(&mut bus), [Saw::Cancelled(id)]);

    let after = send(&mut bus, "RequestAuthorization", &(path(PHONE),));
    let received = bus.received();
    assert_eq!(error_name(answer(&received, after).unwrap()), ACCESS_DENIED);
}

#[test]
fn display_calls_are_answered_at_once_and_take_no_slot() {
    let mut bus = agent();
    let display = send(&mut bus, "DisplayPasskey", &(path(PHONE), 42u32, 0u16));
    let received = bus.received();
    assert_eq!(
        answer(&received, display).unwrap().kind(),
        MessageKind::MethodReturn
    );
    let shown = requested(&mut bus);
    assert_eq!(
        shown.kind,
        RequestKind::DisplayPasskey {
            passkey: 42,
            entered: 0
        }
    );

    // A request still gets the slot.
    let serial = send(&mut bus, "RequestAuthorization", &(path(WATCH),));
    assert!(answer(&bus.received(), serial).is_none());
    assert_eq!(requested(&mut bus).kind, RequestKind::Authorization);
}

#[test]
fn the_same_passkey_again_moves_entered() {
    let mut bus = agent();
    send(&mut bus, "DisplayPasskey", &(path(PHONE), 42u32, 0u16));
    let id = requested(&mut bus).id;
    send(&mut bus, "DisplayPasskey", &(path(PHONE), 42u32, 3u16));
    assert_eq!(seen(&mut bus), [Saw::Updated(id, 3)]);
    let agent = bus.app.resource::<Agent>();
    assert_eq!(
        agent.shown.as_ref().unwrap().kind,
        RequestKind::DisplayPasskey {
            passkey: 42,
            entered: 3
        }
    );
}

#[test]
fn a_new_bluetoothd_drops_every_request() {
    let mut bus = agent();
    let serial = send(&mut bus, "RequestAuthorization", &(path(PHONE),));
    let id = requested(&mut bus).id;
    set_bluetoothd(&mut bus.app, Some(BusName::new(":1.8").unwrap()));
    bus.turn();
    let received = bus.received();
    assert_eq!(error_name(answer(&received, serial).unwrap()), CANCELED);
    assert_eq!(seen(&mut bus), [Saw::Cancelled(id)]);
}

#[test]
fn a_wrong_signature_never_reaches_the_agent() {
    let mut bus = agent();
    let serial = send(&mut bus, "RequestConfirmation", &(path(PHONE),));
    let received = bus.received();
    assert_eq!(
        error_name(answer(&received, serial).unwrap()),
        "org.freedesktop.DBus.Error.InvalidArgs"
    );
    assert!(seen(&mut bus).is_empty());
}

#[test]
fn stopping_a_shown_code_cancels_the_pairing() {
    let mut bus = agent();
    send(&mut bus, "DisplayPinCode", &(path(WATCH), "1234"));
    let id = requested(&mut bus).id;
    bus.received();

    bus.app.signal(PairingAbort { id });
    bus.turn();
    let call = bus.expect_call("CancelPairing");
    assert_eq!(call.destination().unwrap().as_str(), BLUEZ);
    assert_eq!(call.path().unwrap().as_str(), WATCH);
    assert_eq!(call.interface().unwrap().as_str(), DEVICE_IFACE);

    // The code is gone: bluetoothd's Cancel after it closes nothing.
    send_empty(&mut bus, "Cancel");
    assert!(seen(&mut bus).is_empty());

    bus.error(&call, "org.bluez.Error.DoesNotExist", "No such pairing");
    bus.turn();
    assert_eq!(bus.app.resource::<Agent>().aborting, None);
}

#[test]
fn stopping_another_code_does_nothing() {
    let mut bus = agent();
    send(&mut bus, "DisplayPinCode", &(path(WATCH), "1234"));
    let id = requested(&mut bus).id;
    bus.received();

    bus.app.signal(PairingAbort { id: id + 1 });
    bus.turn();
    assert!(bus.received().is_empty());
    assert!(bus.app.resource::<Agent>().shown.is_some());
}

/// `PropertiesChanged` for `device`'s `Device1`, as the agent's watch
/// hears it.
fn device_changed(bus: &mut FakeBus, device: &'static str, props: PropMap) {
    let watch = bus.app.resource::<Agent>().paired.expect("a watch");
    bus.app.signal(SignalReceived {
        bus: BusId::Session,
        matches: [watch].into_iter().collect(),
        msg: Message::signal(
            path(device),
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .with_body(&(DEVICE_IFACE, props, Vec::<String>::new())),
    });
    bus.turn();
}

fn paired(value: bool) -> PropMap {
    PropMap::from([("Paired".into(), Value::Bool(value))])
}

/// bluetoothd sends no `Cancel` after a shown code: the device turning
/// paired closes it.
#[test]
fn a_shown_code_closes_once_the_device_is_paired() {
    let mut bus = agent();
    send(&mut bus, "DisplayPasskey", &(path(PHONE), 42u32, 0u16));
    let id = requested(&mut bus).id;
    seen(&mut bus);

    device_changed(&mut bus, PHONE, paired(true));
    assert_eq!(seen(&mut bus), vec![Saw::Cancelled(id)]);
    let agent = bus.app.resource::<Agent>();
    assert!(agent.shown.is_none());
    assert!(agent.paired.is_none());
}

#[test]
fn other_changes_leave_the_code_shown() {
    let mut bus = agent();
    send(&mut bus, "DisplayPinCode", &(path(WATCH), "1234"));
    requested(&mut bus);
    seen(&mut bus);

    device_changed(&mut bus, WATCH, paired(false));
    device_changed(
        &mut bus,
        WATCH,
        PropMap::from([("Connected".into(), Value::Bool(true))]),
    );
    assert!(seen(&mut bus).is_empty());
    assert!(bus.app.resource::<Agent>().shown.is_some());
}

#[test]
fn a_signal_for_another_watch_is_ignored() {
    let mut bus = agent();
    send(&mut bus, "DisplayPasskey", &(path(PHONE), 42u32, 0u16));
    requested(&mut bus);
    seen(&mut bus);

    let other = bus
        .app
        .resource_mut::<DBus>()
        .bus(BusId::Session)
        .unwrap()
        .watch_properties(BLUEZ, path(WATCH), DEVICE_IFACE);
    bus.app.signal(SignalReceived {
        bus: BusId::Session,
        matches: [other].into_iter().collect(),
        msg: Message::signal(
            path(WATCH),
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .with_body(&(DEVICE_IFACE, paired(true), Vec::<String>::new())),
    });
    bus.turn();
    assert!(seen(&mut bus).is_empty());
    assert!(bus.app.resource::<Agent>().shown.is_some());
}

/// However the code ends, its device is no longer watched.
#[test]
fn the_watch_ends_with_the_code() {
    let mut bus = agent();
    send(&mut bus, "DisplayPinCode", &(path(WATCH), "1234"));
    let id = requested(&mut bus).id;
    assert!(bus.app.resource::<Agent>().paired.is_some());
    bus.app.signal(PairingAbort { id });
    bus.turn();
    assert!(bus.app.resource::<Agent>().paired.is_none());

    send(&mut bus, "DisplayPinCode", &(path(WATCH), "1234"));
    requested(&mut bus);
    assert!(bus.app.resource::<Agent>().paired.is_some());
    send_empty(&mut bus, "Cancel");
    assert!(bus.app.resource::<Agent>().paired.is_none());
}
