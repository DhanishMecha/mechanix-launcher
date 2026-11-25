//! The agent on the bus: exports `org.bluez.Agent1`.
//!
//! bluetoothd keeps one request per agent in flight, so there is one
//! active slot: a request call kept unanswered until the dialog answers.
//! One arriving while the slot is taken gets `Canceled`. The display
//! calls are answered at once and kept apart, as the code shown; they
//! never take the slot. Only the registered bluetoothd may call; others
//! get `AccessDenied`. PINs and passkeys are never logged.

use mecha_wayland::dbus::DecodeError;
use mecha_wayland::prelude::*;

use super::interface::{ACCESS_DENIED, AGENT_PATH, CANCELED, REJECTED, agent_spec};
use crate::signals::{
    DeviceInfo, PairingAnswer, PairingCancelled, PairingRequested, PairingUpdated, Reply, Request,
    RequestKind, valid_passkey, valid_pin,
};

/// A request, and its call, kept unanswered.
struct PendingCall {
    request: Request,
    call: IncomingCall,
}

pub(crate) struct Agent {
    bus: BusId,
    export: Option<ExportId>,
    /// The only caller taken.
    bluetoothd: Option<BusName>,
    next_id: u64,
    active: Option<PendingCall>,
    /// The PIN or passkey shown, already answered.
    shown: Option<Request>,
}
impl Resource for Agent {}

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
    });
    app.system(on_call).system(on_answer).system(on_disconnect);
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

/// A call waiting for the dialog: take the slot, or refuse.
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
    let request = Request {
        id: next_id(app),
        device: device_info(path),
        kind,
    };
    app.resource_mut::<Agent>().active = Some(PendingCall {
        request: request.clone(),
        call: call.clone(),
    });
    app.signal(PairingRequested(request));
}

/// A code to show, already answered. A passkey while one is shown only
/// moves `entered`; anything else is shown.
fn show(app: &mut App, path: ObjectPath, kind: RequestKind) {
    let update = {
        let mut agent = app.resource_mut::<Agent>();
        match (&mut agent.shown, &kind) {
            (
                Some(Request {
                    id,
                    kind: RequestKind::DisplayPasskey { entered, .. },
                    ..
                }),
                RequestKind::DisplayPasskey { entered: now, .. },
            ) => {
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
    let request = Request {
        id: next_id(app),
        device: device_info(path),
        kind,
    };
    app.resource_mut::<Agent>().shown = Some(request.clone());
    app.signal(PairingRequested(request));
}

fn next_id(app: &mut App) -> u64 {
    let mut agent = app.resource_mut::<Agent>();
    let id = agent.next_id;
    agent.next_id += 1;
    id
}

/// What the path says until the device's properties are read:
/// `…/dev_AA_BB_CC_DD_EE_FF` is `AA:BB:CC:DD:EE:FF`.
pub(crate) fn device_info(path: ObjectPath) -> DeviceInfo {
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

fn on_answer(app: &mut App, e: &PairingAnswer) {
    let kept = {
        let mut agent = app.resource_mut::<Agent>();
        if agent.active.as_ref().is_none_or(|k| k.request.id != e.id) {
            return;
        }
        agent.active.take()
    };
    let Some(kept) = kept else { return };
    let mut dbus = app.resource_mut::<DBus>();
    let call = &kept.call;
    match (&kept.request.kind, &e.reply) {
        (RequestKind::PinCode, Reply::PinCode(pin)) if valid_pin(pin) => {
            dbus.reply(call, &(pin.as_str(),));
        }
        (RequestKind::Passkey, Reply::Passkey(passkey)) if valid_passkey(*passkey) => {
            dbus.reply(call, &(*passkey,));
        }
        (
            RequestKind::Confirmation(_) | RequestKind::Authorization | RequestKind::Service(_),
            Reply::Accept,
        ) => dbus.reply(call, &()),
        (_, Reply::Reject) => {
            tracing::info!("bluetooth: {} rejected", call.member());
            dbus.reply_error(call, REJECTED, "Rejected by the user");
            return;
        }
        (_, reply) => {
            tracing::error!(
                "bluetooth: {}: answer {} does not fit, rejected",
                call.member(),
                reply_name(reply)
            );
            dbus.reply_error(call, REJECTED, "Invalid answer");
            return;
        }
    }
    tracing::info!("bluetooth: {} accepted", call.member());
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

#[cfg(test)]
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
