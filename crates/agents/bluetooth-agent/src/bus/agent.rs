//! The `org.bluez.Agent1` object that bluetoothd calls to pair devices.
//!
//! - Only the registered bluetoothd may call. PINs and passkeys are never
//!   logged.
//! - One request at a time waits for the dialog's answer; another one
//!   arriving meanwhile gets `Canceled`.
//! - Display calls are answered at once and bluetoothd never cancels
//!   them, so the newest request or code replaces the code shown. A code
//!   shown closes when its device's `Paired` turns true.
//! - A device paired with a code the user checked (shown, entered or
//!   confirmed) is set `Trusted`. Anything that comes next, or
//!   [`TRUST_WINDOW`] passing, cancels that. Known gap: within the window,
//!   a "just works" pairing of the same device is trusted too.

use std::time::Duration;

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

/// How long a checked device may take to pair and still be trusted
/// (SMP gives up after 30 s).
const TRUST_WINDOW: Duration = Duration::from_secs(60);

/// A request whose call is not answered yet.
struct PendingCall {
    request: Request,
    call: IncomingCall,
}

pub(crate) struct Agent {
    bus: BusId,
    export: Option<ExportId>,
    /// The only caller accepted; set by `register`.
    bluetoothd: Option<BusName>,
    next_id: u64,
    active: Option<PendingCall>,
    /// The code on screen (already answered).
    shown: Option<Request>,
    /// `CancelPairing` in flight.
    aborting: Option<(CallId, ObjectPath)>,
    paired: Option<PairedWatch>,
    /// `Trusted` being set.
    trusting: Option<(CallId, ObjectPath)>,
    /// [`TRUST_WINDOW`] timeouts not yet finished, current or replaced.
    timeouts: Vec<Token>,
}
impl Resource for Agent {}

/// Watches a device's `Paired`, for a code shown or accepted.
struct PairedWatch {
    watch: MatchId,
    path: ObjectPath,
    /// Set `Trusted` once paired; cleared when the window ends.
    trust: bool,
    expires: Token,
}

/// Export the agent on `bus`; false if that failed.
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
        timeouts: Vec::new(),
    });
    app.system(on_call)
        .system(on_answer)
        .system(on_abort)
        .system(on_abort_reply)
        .system(on_paired)
        .system(on_trust_reply)
        .system(on_timeout)
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

/// A call for the dialog: refused while another waits, otherwise it
/// replaces the code shown and any pending trust.
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
    // A code checked before did not pair: don't trust it.
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

/// Show a code. The same passkey and device again only updates `entered`.
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

/// Close the code shown, if any; its pairing goes on unseen.
fn replace_shown(app: &mut App) {
    let Some(old) = app.resource_mut::<Agent>().shown.take() else {
        return;
    };
    tracing::info!("bluetooth: the code for {} gives way", old.device.path);
    unwatch_paired(app);
    app.signal(PairingCancelled { id: old.id });
}

/// Watch `path`'s `Paired` instead of the previous device.
fn watch_paired(app: &mut App, path: &ObjectPath, trust: bool) {
    unwatch_paired(app);
    let bus = app.resource::<Agent>().bus;
    let watch = {
        let mut dbus = app.resource_mut::<DBus>();
        dbus.bus(bus)
            .map(|mut b| b.watch_properties(BLUEZ, path.clone(), DEVICE_IFACE))
    };
    let Some(watch) = watch else {
        return;
    };
    let (mut agent, mut ring) = app.query::<(ResMut<Agent>, ResMut<Ring>)>();
    let expires = ring.timeout(TRUST_WINDOW);
    agent.timeouts.push(expires);
    agent.paired = Some(PairedWatch {
        watch,
        path: path.clone(),
        trust,
        expires,
    });
}

/// The current watch's window ended unpaired: no trust. A code still
/// shown keeps its watch so it closes once paired.
fn on_timeout(app: &mut App, e: &IoEvent) {
    let shown = {
        let (mut agent, mut ring) = app.query::<(ResMut<Agent>, ResMut<Ring>)>();
        let Some(i) = agent.timeouts.iter().position(|t| *t == e.token) else {
            return;
        };
        agent.timeouts.swap_remove(i);
        ring.finish(e.token);
        let agent = &mut *agent;
        let Some(watched) = agent.paired.as_mut().filter(|w| w.expires == e.token) else {
            return;
        };
        tracing::info!(
            "bluetooth: {} did not pair in time, it will not be trusted",
            watched.path
        );
        watched.trust = false;
        agent
            .shown
            .as_ref()
            .is_some_and(|s| s.device.path == watched.path)
    };
    if !shown {
        unwatch_paired(app);
    }
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

/// The watched device paired: close its code and trust it if allowed.
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

/// Set `Trusted`, so the device reconnects and uses services unasked.
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
        // Still paired, just untrusted.
        tracing::warn!("bluetooth: trusting {path} failed: {error:?}");
    }
}

fn next_id(app: &mut App) -> u64 {
    let mut agent = app.resource_mut::<Agent>();
    let id = agent.next_id;
    agent.next_id += 1;
    id
}

/// Apply `change` to request `id`'s device and tell the dialog, if the
/// request is still open.
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
    // A code entered or confirmed: trust once paired. Watch before
    // replying so `Paired` can't be missed.
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

/// The user stopped a shown code: forget it and call `CancelPairing`.
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
        // Usually the pairing had already ended.
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

/// bluetoothd unregistered the agent: drop everything until re-registered.
fn release(app: &mut App, call: &IncomingCall) {
    app.resource_mut::<DBus>().reply(call, &());
    tracing::info!("bluetooth: the agent was released");
    app.resource_mut::<Agent>().bluetoothd = None;
    drop_all(app);
}

/// Cancel the open request and close the code shown.
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

/// Accept calls only from `bluetoothd`; a change drops everything.
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

/// The bus is gone: drop everything and close the dialog.
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
