//! The agent on the bus: exports `org.bluez.Agent1`.
//!
//! bluetoothd keeps one request per agent in flight, so there is one
//! active slot: a request call kept unanswered until the dialog answers.
//! One arriving while the slot is taken gets `Canceled`. Only the
//! registered bluetoothd may call; others get `AccessDenied`. PINs and
//! passkeys are never logged.
//!
//! The display calls are answered at once and kept apart, as the code
//! shown; they never take the slot. bluetoothd counts a code shown as
//! done once answered (`DisplayPasskey` waits for no answer,
//! `DisplayPinCode` frees its slot on it), so it sends no `Cancel` for
//! it, even when the pairing fails, and may ask something else while it
//! still shows. So the newest wins: a request, or another code, takes the
//! place of the code shown. A `DisplayPasskey` for the same device and
//! passkey only moves the keys typed.
//!
//! Stopping a shown code ([`PairingAbort`]) calls the device's
//! `CancelPairing`. The device's `Paired` is watched while a code shows:
//! turning true, the code is done.
//!
//! A device paired with a code the user checked is trusted: once its
//! `Paired` turns true, its `Trusted` is set, so bluetoothd lets it
//! reconnect, and connect its services, without asking again. That is a
//! code shown (typed on the device), entered (PIN, passkey) or confirmed.
//! A pairing allowed without a code (`RequestAuthorization`) is not
//! trusted: its services still ask. A failed pairing tells the agent
//! nothing, so the trust is dropped by anything that comes after it: a
//! new request or code, as well as a rejection, a stop or a `Cancel`.
//! Known gap: if nothing comes after it, and the same device is then
//! paired without asking the agent (a "just works" pairing started from
//! this side, which bluetoothd accepts by itself), that pairing is
//! trusted too, as the agent cannot tell it from the checked one.

use mecha_wayland::dbus::DecodeError;
use mecha_wayland::prelude::*;

use super::device;
use super::interface::{
    ACCESS_DENIED, AGENT_PATH, BLUEZ, CANCELED, DEVICE_IFACE, REJECTED, agent_spec,
};
use crate::signals::{
    DeviceInfo, PairingAbort, PairingAnswer, PairingCancelled, PairingDevice, PairingRequested,
    PairingUpdated, Reply, Request, RequestKind, valid_passkey, valid_pin,
};

/// A request, and its call, kept unanswered.
struct PendingCall {
    request: Request,
    call: IncomingCall,
}

pub(crate) struct Agent {
    bus: BusId,
    export: Option<ExportId>,
    /// The only caller taken: bluetoothd's unique name, while
    /// registered. Set by `register`, which reads it back from here.
    bluetoothd: Option<BusName>,
    next_id: u64,
    active: Option<PendingCall>,
    /// The PIN or passkey shown, already answered.
    shown: Option<Request>,
    /// `CancelPairing` in flight, and the device it stops.
    aborting: Option<(CallId, ObjectPath)>,
    /// The watch on the device being paired, for its `Paired`.
    paired: Option<PairedWatch>,
    /// Setting `Trusted` in flight, and the device it trusts.
    trusting: Option<(CallId, ObjectPath)>,
}
impl Resource for Agent {}

/// A device whose `Paired` is watched: a code shown for it, or a code
/// for it accepted.
struct PairedWatch {
    watch: MatchId,
    path: ObjectPath,
    /// Set its `Trusted` once paired.
    trust: bool,
}

/// Export the agent on `bus`; false if that failed. Installs after
/// `DBusModule`.
pub(crate) fn install(app: &mut App, bus: BusId) -> bool {
    if app.has_resource::<Agent>() {
        return app.resource::<Agent>().export.is_some();
    }
    let export = {
        let mut dbus = app.resource_mut::<DBus>();
        match dbus.bus(bus) {
            Some(mut b) => b
                .export(AGENT_PATH, agent_spec())
                .inspect_err(|e| tracing::error!("bluetooth: exporting the agent: {e:?}"))
                .ok(),
            None => {
                tracing::error!("bluetooth: the bus is not connected, no pairing agent");
                None
            }
        }
    };
    let exported = export.is_some();
    app.insert_resource(Agent {
        bus,
        export,
        bluetoothd: None,
        next_id: 1,
        active: None,
        shown: None,
        aborting: None,
        paired: None,
        trusting: None,
    });
    app.system(on_call)
        .system(on_answer)
        .system(on_abort)
        .system(on_abort_reply)
        .system(on_paired)
        .system(on_trust_reply)
        .system(on_disconnect);
    device::install(app, bus);
    exported
}

fn on_call(app: &mut App, e: &MethodCallReceived) {
    let bluetoothd = {
        let agent = app.resource::<Agent>();
        if agent.export != Some(e.export) {
            return;
        }
        agent.bluetoothd.clone()
    };
    if bluetoothd.is_none() || e.call.sender() != bluetoothd.as_ref() {
        tracing::warn!(
            "bluetooth: refused {} from {:?}: not bluetoothd",
            e.call.member(),
            e.call.sender()
        );
        app.resource_mut::<DBus>().reply_error(
            &e.call,
            ACCESS_DENIED,
            "Only bluetoothd may call the agent",
        );
        return;
    }
    let call = &e.call;
    match decode(call) {
        Ok(Some(AgentCall::Release)) => release(app, call),
        Ok(Some(AgentCall::Cancel)) => cancel(app, call),
        Ok(Some(AgentCall::Pairing(path, kind))) if kind.is_display() => {
            app.resource_mut::<DBus>().reply(call, &());
            show(app, path, kind);
        }
        Ok(Some(AgentCall::Pairing(path, kind))) => request(app, call, path, kind),
        Ok(None) => {}
        Err(error) => bad_args(app, call, error),
    }
}

/// What bluetoothd asked for.
enum AgentCall {
    Release,
    Cancel,
    /// A code to show, or a request for the dialog.
    Pairing(ObjectPath, RequestKind),
}

/// The call's member and arguments; `None` for a member not exported.
fn decode(call: &IncomingCall) -> Result<Option<AgentCall>, DecodeError> {
    use RequestKind::*;
    let device = || call.args::<(ObjectPath,)>().map(|(path,)| path);
    let pairing = |path, kind| Some(AgentCall::Pairing(path, kind));
    Ok(match call.member().as_str() {
        "Release" => Some(AgentCall::Release),
        "Cancel" => Some(AgentCall::Cancel),
        "RequestPinCode" => pairing(device()?, PinCode),
        "RequestPasskey" => pairing(device()?, Passkey),
        "RequestAuthorization" => pairing(device()?, Authorization),
        "RequestConfirmation" => {
            let (path, passkey) = call.args()?;
            pairing(path, Confirmation(passkey))
        }
        "AuthorizeService" => {
            let (path, uuid) = call.args()?;
            pairing(path, Service(uuid))
        }
        "DisplayPinCode" => {
            let (path, pin) = call.args()?;
            pairing(path, DisplayPinCode(pin))
        }
        "DisplayPasskey" => {
            let (path, passkey, entered) = call.args()?;
            pairing(path, DisplayPasskey { passkey, entered })
        }
        _ => None,
    })
}

fn cancel(app: &mut App, call: &IncomingCall) {
    app.resource_mut::<DBus>().reply(call, &());
    tracing::info!("bluetooth: bluetoothd cancelled");
    drop_all(app);
}

fn bad_args(app: &mut App, call: &IncomingCall, error: DecodeError) {
    tracing::error!("bluetooth: {}: bad arguments: {error:?}", call.member());
    app.resource_mut::<DBus>()
        .reply_error(call, REJECTED, "Bad arguments");
}

/// A call waiting for the dialog: take the slot, or refuse while it is
/// taken. It takes the place of a code shown, and of a trust pending.
fn request(app: &mut App, call: &IncomingCall, path: ObjectPath, kind: RequestKind) {
    if app.resource::<Agent>().active.is_some() {
        tracing::warn!(
            "bluetooth: refused {} for {path}: another request is open",
            call.member()
        );
        app.resource_mut::<DBus>()
            .reply_error(call, CANCELED, "Another request is open");
        return;
    }
    tracing::info!("bluetooth: {} for {path}", call.member());
    replace_shown(app);
    // Whatever was checked before did not pair: trust nothing of it.
    unwatch_paired(app);
    let id = next_id(app);
    let request = Request {
        id,
        device: device::from_path(path.clone()),
        kind,
    };
    app.resource_mut::<Agent>().active = Some(PendingCall {
        request: request.clone(),
        call: call.clone(),
    });
    app.signal(PairingRequested(request));
    device::read(app, id, &path);
}

/// A code to show, already answered. The passkey shown again, for the
/// same device, only moves `entered`; anything else takes its place.
fn show(app: &mut App, path: ObjectPath, kind: RequestKind) {
    let update = {
        let mut agent = app.resource_mut::<Agent>();
        match (&mut agent.shown, &kind) {
            (
                Some(Request {
                    id,
                    device,
                    kind:
                        RequestKind::DisplayPasskey {
                            passkey: was,
                            entered,
                        },
                }),
                RequestKind::DisplayPasskey {
                    passkey,
                    entered: now,
                },
            ) if device.path == path && was == passkey => {
                *entered = *now;
                Some(PairingUpdated {
                    id: *id,
                    entered: *now,
                })
            }
            _ => None,
        }
    };
    if let Some(update) = update {
        app.signal(update);
        return;
    }
    tracing::info!("bluetooth: showing a code for {path}");
    replace_shown(app);
    let id = next_id(app);
    let request = Request {
        id,
        device: device::from_path(path.clone()),
        kind,
    };
    app.resource_mut::<Agent>().shown = Some(request.clone());
    // Typed on the device: a checked code.
    watch_paired(app, &path, true);
    app.signal(PairingRequested(request));
    device::read(app, id, &path);
}

/// The code shown, if any, gives way to what comes now: the dialog
/// closes it. Its pairing goes on, unseen; bluetoothd asked for the next.
fn replace_shown(app: &mut App) {
    let Some(old) = app.resource_mut::<Agent>().shown.take() else {
        return;
    };
    tracing::info!("bluetooth: the code for {} gives way", old.device.path);
    unwatch_paired(app);
    app.signal(PairingCancelled { id: old.id });
}

/// Follow `path`'s `Paired`, in place of the device watched before;
/// `trust` it once paired.
fn watch_paired(app: &mut App, path: &ObjectPath, trust: bool) {
    unwatch_paired(app);
    let bus = app.resource::<Agent>().bus;
    let watch = {
        let mut dbus = app.resource_mut::<DBus>();
        dbus.bus(bus)
            .map(|mut b| b.watch_properties(BLUEZ, path.clone(), DEVICE_IFACE))
    };
    app.resource_mut::<Agent>().paired = watch.map(|watch| PairedWatch {
        watch,
        path: path.clone(),
        trust,
    });
}

fn unwatch_paired(app: &mut App) {
    let (bus, watched) = {
        let mut agent = app.resource_mut::<Agent>();
        (agent.bus, agent.paired.take())
    };
    if let Some(watched) = watched
        && let Some(mut b) = app.resource_mut::<DBus>().bus(bus)
    {
        b.remove_match(watched.watch);
    }
}

/// The watched device is paired: a code shown for it did its job, close
/// it; and trust the device, if its code was checked.
fn on_paired(app: &mut App, e: &SignalReceived) {
    let (path, trust) = match &app.resource::<Agent>().paired {
        Some(w) if e.matches.contains(&w.watch) => (w.path.clone(), w.trust),
        _ => return,
    };
    let paired = match PropertiesChanged::parse(&e.msg) {
        Ok((_, changed, _)) => matches!(changed.get_as::<bool>("Paired"), Some(Ok(true))),
        Err(error) => {
            tracing::warn!("bluetooth: the device's PropertiesChanged: {error:?}");
            return;
        }
    };
    if !paired {
        return;
    }
    tracing::info!("bluetooth: {path} is paired");
    let shown = {
        let mut agent = app.resource_mut::<Agent>();
        agent.shown.take_if(|s| s.device.path == path)
    };
    unwatch_paired(app);
    if let Some(shown) = shown {
        app.signal(PairingCancelled { id: shown.id });
    }
    if trust {
        set_trusted(app, path);
    }
}

/// Set `path`'s `Trusted`: bluetoothd lets it reconnect, and connect its
/// services, without asking.
fn set_trusted(app: &mut App, path: ObjectPath) {
    let bus = app.resource::<Agent>().bus;
    let call = {
        let mut dbus = app.resource_mut::<DBus>();
        let Some(b) = dbus.bus(bus) else {
            tracing::error!("bluetooth: trusting {path}: the bus is not connected");
            return;
        };
        b.object(BLUEZ, path.clone())
            .set_property(DEVICE_IFACE, "Trusted", Value::Bool(true))
    };
    tracing::info!("bluetooth: trusting {path}");
    app.resource_mut::<Agent>().trusting = Some((call, path));
}

fn on_trust_reply(app: &mut App, e: &MethodReply) {
    let path = {
        let mut agent = app.resource_mut::<Agent>();
        match &agent.trusting {
            Some((call, _)) if *call == e.call => agent.trusting.take().map(|(_, p)| p),
            _ => return,
        }
    };
    if let (Err(error), Some(path)) = (&e.result, path) {
        // Paired still; its services will ask, as for an untrusted one.
        tracing::warn!("bluetooth: trusting {path} failed: {error:?}");
    }
}

fn next_id(app: &mut App) -> u64 {
    let mut agent = app.resource_mut::<Agent>();
    let id = agent.next_id;
    agent.next_id += 1;
    id
}

/// Request `id`'s device was read: keep what `change` makes of it, and
/// tell the dialog. Nothing if the request has ended.
pub(crate) fn update_device(app: &mut App, id: u64, change: impl FnOnce(DeviceInfo) -> DeviceInfo) {
    let device = {
        let mut agent = app.resource_mut::<Agent>();
        let agent = &mut *agent;
        let request = agent
            .active
            .as_mut()
            .map(|k| &mut k.request)
            .filter(|r| r.id == id)
            .or_else(|| agent.shown.as_mut().filter(|r| r.id == id));
        let Some(request) = request else {
            return;
        };
        request.device = change(request.device.clone());
        request.device.clone()
    };
    app.signal(PairingDevice { id, device });
}

fn on_answer(app: &mut App, e: &PairingAnswer) {
    let kept = {
        let mut agent = app.resource_mut::<Agent>();
        if agent.active.as_ref().is_none_or(|k| k.request.id != e.id) {
            return;
        }
        agent.active.take()
    };
    let Some(kept) = kept else { return };
    // A checked code accepted (entered or confirmed): trust the device
    // once paired. Watched before the reply goes, so `Paired` turning
    // true is not missed. Authorization or a service checks nothing.
    let path = &kept.request.device.path;
    let call = &kept.call;
    match (&kept.request.kind, &e.reply) {
        (RequestKind::PinCode, Reply::PinCode(pin)) if valid_pin(pin) => {
            watch_paired(app, path, true);
            app.resource_mut::<DBus>().reply(call, &(pin.as_str(),));
        }
        (RequestKind::Passkey, Reply::Passkey(passkey)) if valid_passkey(*passkey) => {
            watch_paired(app, path, true);
            app.resource_mut::<DBus>().reply(call, &(*passkey,));
        }
        (RequestKind::Confirmation(_), Reply::Accept) => {
            watch_paired(app, path, true);
            app.resource_mut::<DBus>().reply(call, &());
        }
        (RequestKind::Authorization | RequestKind::Service(_), Reply::Accept) => {
            app.resource_mut::<DBus>().reply(call, &());
        }
        (_, Reply::Reject) => {
            tracing::info!("bluetooth: {} rejected", call.member());
            app.resource_mut::<DBus>()
                .reply_error(call, REJECTED, "Rejected by the user");
            return;
        }
        (_, reply) => {
            tracing::error!(
                "bluetooth: {}: answer {} does not fit, rejected",
                call.member(),
                reply_name(reply)
            );
            app.resource_mut::<DBus>()
                .reply_error(call, REJECTED, "Invalid answer");
            return;
        }
    }
    tracing::info!("bluetooth: {} accepted", call.member());
}

/// The user stopped the pairing whose code is shown: forget the code,
/// and ask bluetoothd to stop pairing with the device.
fn on_abort(app: &mut App, e: &PairingAbort) {
    let (bus, path) = {
        let mut agent = app.resource_mut::<Agent>();
        let Some(shown) = agent.shown.take_if(|s| s.id == e.id) else {
            return;
        };
        (agent.bus, shown.device.path)
    };
    unwatch_paired(app);
    tracing::info!("bluetooth: stopping the pairing with {path}");
    let call = {
        let mut dbus = app.resource_mut::<DBus>();
        let Some(mut b) = dbus.bus(bus) else {
            tracing::error!("bluetooth: CancelPairing: the bus is not connected");
            return;
        };
        b.send(MethodCall::new(
            BLUEZ,
            path.clone(),
            DEVICE_IFACE,
            "CancelPairing",
        ))
    };
    app.resource_mut::<Agent>().aborting = Some((call, path));
}

fn on_abort_reply(app: &mut App, e: &MethodReply) {
    let path = {
        let mut agent = app.resource_mut::<Agent>();
        match &agent.aborting {
            Some((call, _)) if *call == e.call => agent.aborting.take().map(|(_, p)| p),
            _ => return,
        }
    };
    if let (Err(error), Some(path)) = (&e.result, path) {
        // Often only that the pairing had ended already.
        tracing::warn!("bluetooth: CancelPairing for {path} failed: {error:?}");
    }
}

/// Names an answer without its PIN or passkey.
fn reply_name(reply: &Reply) -> &'static str {
    match reply {
        Reply::PinCode(_) => "PinCode",
        Reply::Passkey(_) => "Passkey",
        Reply::Accept => "Accept",
        Reply::Reject => "Reject",
    }
}

/// bluetoothd unregistered the agent: drop everything, and take no more
/// calls until registered again.
fn release(app: &mut App, call: &IncomingCall) {
    app.resource_mut::<DBus>().reply(call, &());
    tracing::info!("bluetooth: the agent was released");
    app.resource_mut::<Agent>().bluetoothd = None;
    drop_all(app);
}

/// End the open request, answering `Canceled`, and what is shown.
fn drop_all(app: &mut App) {
    let (active, shown) = {
        let mut agent = app.resource_mut::<Agent>();
        (agent.active.take(), agent.shown.take())
    };
    unwatch_paired(app);
    if let Some(kept) = active {
        app.resource_mut::<DBus>()
            .reply_error(&kept.call, CANCELED, "Canceled");
        app.signal(PairingCancelled {
            id: kept.request.id,
        });
    }
    if let Some(shown) = shown {
        app.signal(PairingCancelled { id: shown.id });
    }
}

/// Take calls only from `bluetoothd`. A new bluetoothd drops every
/// request.
pub(crate) fn set_bluetoothd(app: &mut App, bluetoothd: Option<BusName>) {
    {
        let mut agent = app.resource_mut::<Agent>();
        if agent.bluetoothd == bluetoothd {
            return;
        }
        agent.bluetoothd = bluetoothd;
    }
    drop_all(app);
}

/// The bluetoothd registered with, if any.
pub(crate) fn bluetoothd(app: &App) -> Option<BusName> {
    app.resource::<Agent>().bluetoothd.clone()
}

/// The bus is gone, and bluetoothd's calls with it: drop them, and
/// close the dialog.
fn on_disconnect(app: &mut App, e: &BusDisconnected) {
    let (active, shown) = {
        let mut agent = app.resource_mut::<Agent>();
        if e.bus != agent.bus {
            return;
        }
        agent.bluetoothd = None;
        agent.aborting = None;
        agent.trusting = None;
        // Its matches went with the bus.
        agent.paired = None;
        (agent.active.take(), agent.shown.take())
    };
    for id in [active.map(|k| k.request.id), shown.map(|s| s.id)]
        .into_iter()
        .flatten()
    {
        app.signal(PairingCancelled { id });
    }
}

#[cfg(test)]
mod tests;
