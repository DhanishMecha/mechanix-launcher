//! Registers the agent with bluetoothd, and asks to be its default.
//!
//! `RegisterAgent` makes the agent bluetoothd's for pairings the launcher
//! starts; `RequestDefaultAgent` makes it the one for every other pairing
//! and connection too. A failed registration is logged and the launcher
//! runs without an agent; a refused default leaves it registered.
//! bluetoothd drops the registration when the launcher leaves the bus.
//! The agent takes calls only from the bluetoothd that answered, and
//! registers again whenever bluetoothd (re)appears.

use mecha_wayland::prelude::*;

use super::agent;
use super::interface::AGENT_PATH;

const BLUEZ: &str = "org.bluez";
const MANAGER_PATH: &str = "/org/bluez";
const AGENT_MANAGER: &str = "org.bluez.AgentManager1";

/// What the agent can do: show a code and take one typed on the
/// on-screen keyboard.
pub(crate) const CAPABILITY: &str = "KeyboardDisplay";

const DAEMON: &str = "org.freedesktop.DBus";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// `AgentManager1.RegisterAgent`.
    Register,
    /// `AgentManager1.RequestDefaultAgent`.
    RequestDefault,
}

#[derive(Debug)]
struct Registration {
    bus: BusId,
    call: Option<(CallId, Step)>,
    /// bluetoothd's unique name, while registered.
    bluetoothd: Option<BusName>,
    owner_changed: Option<MatchId>,
}
impl Resource for Registration {}

/// Register the agent exported on `bus`.
pub(crate) fn install(app: &mut App, bus: BusId) {
    if app.has_resource::<Registration>() {
        return;
    }
    let owner_changed = app.resource_mut::<DBus>().bus(bus).map(|mut b| {
        b.add_match(
            MatchRule::signal()
                .sender(DAEMON)
                .interface(DAEMON)
                .member("NameOwnerChanged")
                .arg(0, BLUEZ),
        )
    });
    app.insert_resource(Registration {
        bus,
        call: None,
        bluetoothd: None,
        owner_changed,
    });
    app.system(on_reply)
        .system(on_owner_changed)
        .system(on_disconnect);
    register(app);
}

fn send(app: &mut App, call: MethodCall, step: Step) {
    let bus = app.resource::<Registration>().bus;
    let id = {
        let mut dbus = app.resource_mut::<DBus>();
        let Some(mut b) = dbus.bus(bus) else {
            tracing::error!("bluetooth: {step:?}: the bus is not connected");
            return;
        };
        b.send(call)
    };
    app.resource_mut::<Registration>().call = Some((id, step));
}

fn register(app: &mut App) {
    tracing::info!("bluetooth: registering the agent as {CAPABILITY}");
    send(
        app,
        MethodCall::new(BLUEZ, MANAGER_PATH, AGENT_MANAGER, "RegisterAgent")
            .body(&(ObjectPath::from(AGENT_PATH), CAPABILITY)),
        Step::Register,
    );
}

fn on_reply(app: &mut App, e: &MethodReply) {
    let step = {
        let mut reg = app.resource_mut::<Registration>();
        match reg.call {
            Some((call, step)) if call == e.call => {
                reg.call = None;
                step
            }
            _ => return,
        }
    };
    let msg = match (&e.result, step) {
        (Ok(msg), _) => msg,
        (Err(error), Step::Register) => {
            tracing::error!("bluetooth: RegisterAgent failed, no pairing agent: {error:?}");
            return;
        }
        (Err(error), Step::RequestDefault) => {
            tracing::warn!(
                "bluetooth: RequestDefaultAgent failed, the agent is registered but not the default: {error:?}"
            );
            return;
        }
    };
    match step {
        Step::Register => {
            let Some(bluetoothd) = msg.sender().cloned() else {
                tracing::error!("bluetooth: the registration's reply has no sender");
                return;
            };
            tracing::info!("bluetooth: agent registered with {bluetoothd}");
            app.resource_mut::<Registration>().bluetoothd = Some(bluetoothd.clone());
            agent::set_bluetoothd(app, Some(bluetoothd));
            send(
                app,
                MethodCall::new(BLUEZ, MANAGER_PATH, AGENT_MANAGER, "RequestDefaultAgent")
                    .body(&(ObjectPath::from(AGENT_PATH),)),
                Step::RequestDefault,
            );
        }
        Step::RequestDefault => tracing::info!("bluetooth: agent is the default"),
    }
}

/// bluetoothd went away or came back: forget it, or register again.
fn on_owner_changed(app: &mut App, e: &SignalReceived) {
    let (bus, owner_changed) = {
        let reg = app.resource::<Registration>();
        (reg.bus, reg.owner_changed)
    };
    if e.bus != bus || owner_changed.is_none_or(|m| !e.matches.contains(&m)) {
        return;
    }
    let Ok((_, _, new_owner)) = e.msg.body::<(String, String, String)>() else {
        return;
    };
    let gone = {
        let mut reg = app.resource_mut::<Registration>();
        let gone = reg
            .bluetoothd
            .as_ref()
            .is_some_and(|b| b.as_str() != new_owner);
        if gone {
            reg.bluetoothd = None;
        }
        gone
    };
    if gone {
        tracing::warn!("bluetooth: bluetoothd went away, its requests are dropped");
        agent::set_bluetoothd(app, None);
    }
    // A registration in flight already reaches the new bluetoothd.
    let idle = {
        let reg = app.resource::<Registration>();
        reg.bluetoothd.is_none() && reg.call.is_none()
    };
    if idle && !new_owner.is_empty() {
        register(app);
    }
}

fn on_disconnect(app: &mut App, e: &BusDisconnected) {
    let mut reg = app.resource_mut::<Registration>();
    if e.bus == reg.bus {
        reg.call = None;
        reg.bluetoothd = None;
    }
}

#[cfg(test)]
mod tests;
