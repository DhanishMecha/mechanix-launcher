use mecha_wayland::mechanix_widgets::NativeText;
use mecha_wayland::mechanix_widgets::prelude::{FontBook, Text};

use super::prompt::{PASSKEY_ERROR, PIN_ERROR};
use super::view::*;
use super::*;
use crate::signals::{DeviceInfo, Reply};

const PHONE: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF";

/// What the dialog sent.
#[derive(Default)]
struct Sent {
    answers: Vec<(u64, Reply)>,
    aborts: Vec<u64>,
}
impl Resource for Sent {}

fn app() -> App {
    let mut app = App::new();
    app.add_module(RingModule::default())
        .add_module(TimerModule)
        .add_module(LayoutModule)
        .add_module(WindowModule)
        .add_module(PaintModule)
        .add_module(InteractivityModule)
        .add_module(MechanixTheme::dark())
        // The window's layer role; presentation's, which needs Wayland.
        .register_component::<Role>();
    let mut atlas = Atlas::new();
    let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
    app.insert_resource(atlas);
    app.insert_resource(FontBook::new(font));
    app.init_resource::<Sent>();
    install(&mut app);
    app.system(|app: &mut App, e: &PairingAnswer| {
        app.resource_mut::<Sent>()
            .answers
            .push((e.id, e.reply.clone()));
    })
    .system(|app: &mut App, e: &PairingAbort| {
        app.resource_mut::<Sent>().aborts.push(e.id);
    });
    app.tick();
    app
}

fn signal<S: Signal>(app: &mut App, s: S) {
    app.signal(s);
    app.flush();
    app.tick();
}

fn request(app: &mut App, id: u64, kind: RequestKind) {
    signal(
        app,
        PairingRequested(Request {
            id,
            device: DeviceInfo {
                path: ObjectPath::from(PHONE),
                address: "AA:BB:CC:DD:EE:FF".into(),
                name: "AA:BB:CC:DD:EE:FF".into(),
                icon: None,
            },
            kind,
        }),
    );
}

fn asking(kind: RequestKind) -> App {
    let mut app = app();
    request(&mut app, 1, kind);
    app
}

fn parts(app: &App) -> Parts {
    let view = app.resource::<Dialog>().view;
    app.widget::<PairingDialog>(view).unwrap().parts
}

fn emit_to<E: Event>(app: &mut App, node: impl Into<NodeId>, e: E) {
    app.emit(e, node.into());
    app.flush();
    app.tick();
}

fn click(app: &mut App, node: impl Into<NodeId>) {
    emit_to(
        app,
        node,
        Clicked {
            contact: ContactId::Mouse,
            position: Point::default(),
        },
    );
}

fn type_into(app: &mut App, field: Field, text: &str) {
    emit_to(
        app,
        field.input,
        InputEdit {
            commit: Some(text.into()),
            ..Default::default()
        },
    );
}

fn text_of(app: &App, h: Handle<Text>) -> String {
    let native = app.widget::<Text>(h).unwrap().native();
    app.widget::<NativeText>(native)
        .unwrap()
        .props
        .string
        .clone()
}

fn shown(app: &App, node: impl Into<NodeId>) -> bool {
    app.component::<LayoutStyle>(node.into()).unwrap().display == Display::Flex
}

fn note(app: &App, field: Field) -> Option<String> {
    shown(app, field.note).then(|| text_of(app, field.note))
}

fn ok_dimmed(app: &App) -> bool {
    app.widget::<Icon>(parts(app).ok_icon).unwrap().color().a < 1.0
}

fn sent(app: &App) -> &Sent {
    app.resource::<Sent>()
}

fn is_open(app: &App) -> bool {
    app.resource::<Dialog>().shown.is_some()
}

#[test]
fn a_confirmation_shows_the_device_and_the_code() {
    let app = asking(RequestKind::Confirmation(42));
    let p = parts(&app);
    assert_eq!(text_of(&app, p.title), "Bluetooth pairing");
    assert_eq!(text_of(&app, p.name), "AA:BB:CC:DD:EE:FF");
    // The same as the name: not shown twice.
    assert!(!shown(&app, p.address));
    assert!(shown(&app, p.code_box));
    assert_eq!(text_of(&app, p.code), "000042");
    assert!(!shown(&app, p.pin.root));
    assert!(!shown(&app, p.passkey.root));
    assert!(shown(&app, p.ok));
    assert!(!ok_dimmed(&app));
}

#[test]
fn the_check_accepts_and_closes() {
    let mut app = asking(RequestKind::Confirmation(42));
    let ok = parts(&app).ok;
    click(&mut app, ok);
    assert_eq!(sent(&app).answers, [(1, Reply::Accept)]);
    assert!(!is_open(&app));
}

#[test]
fn the_close_icon_rejects() {
    let mut app = asking(RequestKind::Authorization);
    let cancel = parts(&app).cancel;
    click(&mut app, cancel);
    assert_eq!(sent(&app).answers, [(1, Reply::Reject)]);
    assert!(!is_open(&app));
}

#[test]
fn a_service_is_named() {
    let app = asking(RequestKind::Service(
        "0000110b-0000-1000-8000-00805f9b34fb".into(),
    ));
    let p = parts(&app);
    assert_eq!(text_of(&app, p.title), "Bluetooth connection");
    assert_eq!(
        text_of(&app, p.message),
        "This device wants to use Audio Sink."
    );
    assert!(!shown(&app, p.code_box));
}

#[test]
fn a_passkey_is_typed_and_sent() {
    let mut app = asking(RequestKind::Passkey);
    let p = parts(&app);
    assert!(shown(&app, p.passkey.root));
    assert!(!shown(&app, p.pin.root));
    assert_eq!(
        app.widget::<Input>(p.passkey.input.id())
            .unwrap()
            .content_purpose(),
        ContentPurpose::Digits
    );
    // Nothing typed: ✓ dimmed, and sends nothing.
    assert!(ok_dimmed(&app));
    click(&mut app, p.ok);
    assert!(sent(&app).answers.is_empty());

    type_into(&mut app, p.passkey, "012345");
    assert!(!ok_dimmed(&app));
    emit_to(&mut app, p.passkey.input, InputSubmit);
    assert_eq!(sent(&app).answers, [(1, Reply::Passkey(12_345))]);
    assert!(!is_open(&app));
}

#[test]
fn a_bad_passkey_is_kept_with_an_error_until_edited() {
    let mut app = asking(RequestKind::Passkey);
    let p = parts(&app);
    type_into(&mut app, p.passkey, "1234567");
    click(&mut app, p.ok);
    assert!(sent(&app).answers.is_empty());
    assert!(is_open(&app));
    assert_eq!(note(&app, p.passkey).as_deref(), Some(PASSKEY_ERROR));
    assert!(shown(&app, p.passkey.alert));

    emit_to(
        &mut app,
        p.passkey.input,
        InputEdit {
            delete_before: 1,
            ..Default::default()
        },
    );
    assert_eq!(note(&app, p.passkey), None);
    click(&mut app, p.ok);
    assert_eq!(sent(&app).answers, [(1, Reply::Passkey(123_456))]);
}

#[test]
fn a_pin_is_typed_and_sent() {
    let mut app = asking(RequestKind::PinCode);
    let p = parts(&app);
    assert!(shown(&app, p.pin.root));
    type_into(&mut app, p.pin, "12 34");
    click(&mut app, p.ok);
    assert_eq!(note(&app, p.pin).as_deref(), Some(PIN_ERROR));

    type_into(&mut app, p.pin, "x");
    assert_eq!(note(&app, p.pin), None);
    emit_to(
        &mut app,
        p.pin.input,
        InputEdit {
            delete_before: 6,
            ..Default::default()
        },
    );
    type_into(&mut app, p.pin, "0000");
    click(&mut app, p.ok);
    assert_eq!(sent(&app).answers, [(1, Reply::PinCode("0000".into()))]);
}

#[test]
fn a_shown_passkey_counts_the_keys_typed_and_close_stops_it() {
    let mut app = asking(RequestKind::DisplayPasskey {
        passkey: 999_999,
        entered: 0,
    });
    let p = parts(&app);
    assert_eq!(text_of(&app, p.code), "999999");
    assert!(!shown(&app, p.progress));
    // Nothing to answer.
    assert!(!shown(&app, p.ok));

    signal(&mut app, PairingUpdated { id: 1, entered: 4 });
    assert!(shown(&app, p.progress));
    assert_eq!(text_of(&app, p.progress), "4 of 6 digits typed");

    click(&mut app, p.cancel);
    assert_eq!(sent(&app).aborts, [1]);
    assert!(sent(&app).answers.is_empty());
    assert!(!is_open(&app));
}

#[test]
fn cancelling_closes_and_the_next_request_starts_empty() {
    let mut app = asking(RequestKind::Passkey);
    let p = parts(&app);
    type_into(&mut app, p.passkey, "12");
    signal(&mut app, PairingCancelled { id: 1 });
    assert!(!is_open(&app));
    assert_eq!(
        app.widget::<Input>(p.passkey.input.id()).unwrap().text(),
        ""
    );

    request(&mut app, 2, RequestKind::Passkey);
    assert!(ok_dimmed(&app));
    assert!(shown(&app, p.passkey.placeholder));
}

#[test]
fn another_requests_signals_are_ignored() {
    let mut app = asking(RequestKind::DisplayPasskey {
        passkey: 1,
        entered: 0,
    });
    signal(&mut app, PairingUpdated { id: 9, entered: 3 });
    signal(&mut app, PairingCancelled { id: 9 });
    assert!(is_open(&app));
    assert!(!shown(&app, parts(&app).progress));
}

#[test]
fn the_device_is_named_once_read_and_typing_survives_it() {
    let mut app = asking(RequestKind::Passkey);
    let p = parts(&app);
    type_into(&mut app, p.passkey, "12");

    let mut device = app.resource::<Dialog>().shown.clone().unwrap().device;
    device.name = "OnePlus 12".into();
    signal(&mut app, PairingDevice { id: 1, device });

    assert_eq!(text_of(&app, p.name), "OnePlus 12");
    // The address shows under a name.
    assert!(shown(&app, p.address));
    assert_eq!(text_of(&app, p.address), "AA:BB:CC:DD:EE:FF");
    assert_eq!(
        app.widget::<Input>(p.passkey.input.id()).unwrap().text(),
        "12"
    );
}

#[test]
fn another_requests_device_is_ignored() {
    let mut app = asking(RequestKind::Authorization);
    let mut device = app.resource::<Dialog>().shown.clone().unwrap().device;
    device.name = "Someone else".into();
    signal(&mut app, PairingDevice { id: 9, device });
    assert_eq!(text_of(&app, parts(&app).name), "AA:BB:CC:DD:EE:FF");
}

fn avatar(app: &App) -> SpriteId {
    let view = app.resource::<Dialog>().view;
    app.widget::<PairingDialog>(view).unwrap().avatar
}

fn device_named(app: &mut App, id: u64, icon: Option<&str>) {
    let mut device = app.resource::<Dialog>().shown.clone().unwrap().device;
    device.icon = icon.map(str::to_owned);
    signal(app, PairingDevice { id, device });
}

#[test]
fn the_avatar_shows_the_kind_of_device() {
    let mut app = asking(RequestKind::Confirmation(42));
    let icons = *app.resource::<Icons>();
    // Bluetooth's until the device is read.
    assert_eq!(avatar(&app), icons.bluetooth);

    device_named(&mut app, 1, Some("phone"));
    assert_eq!(avatar(&app), icons.devices[DeviceKind::Phone as usize]);

    // Back to Bluetooth's once closed, and for a kind without its own.
    signal(&mut app, PairingCancelled { id: 1 });
    assert_eq!(avatar(&app), icons.bluetooth);
    request(&mut app, 2, RequestKind::Authorization);
    device_named(&mut app, 2, Some("modem"));
    assert_eq!(avatar(&app), icons.bluetooth);
}
