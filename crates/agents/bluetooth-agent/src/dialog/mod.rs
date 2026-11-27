//! The pairing dialog: a bottom sheet for bluetoothd's requests.
//!
//! Shown on [`PairingRequested`], hidden on [`PairingCancelled`] or once
//! answered. The device shows by its address and the Bluetooth icon
//! until [`PairingDevice`] gives its name and kind. What it shows follows the request:
//!
//! - a PIN or passkey to enter: a field, sent with ✓ or Return;
//! - a code to confirm, a pairing or a service to allow: ✓ accepts;
//! - a code to type on the device: shown, with the keys typed so far.
//!
//! ✕ or dragging the sheet down rejects a request, or stops the pairing
//! whose code is shown ([`PairingAbort`]).
//!
//! The window is a [`PromptWindow`]: an overlay-layer `prompt`
//! (mx_shell_surface_v1), above every other shell window, that takes the
//! keyboard while shown.
//!
//! One system prompt shows at a time ([`prompt_queue`]), first come,
//! first served: asked for while another shows, the request waits with
//! the window hidden until its turn; what it is told meanwhile is kept,
//! and shown then.
//!
//! Not yet: closing for the lock screen.

use mecha_wayland::prelude::*;
use prompt_queue::{Owner, Prompt, PromptWindow};

use crate::signals::{
    PairingAbort, PairingAnswer, PairingCancelled, PairingDevice, PairingRequested, PairingUpdated,
    Request, RequestKind,
};

mod prompt;
mod view;

use prompt::DeviceKind;
use view::{PairingDialog, PairingDialogBuilder};

const NAMESPACE: &str = "bluetooth-agent";

const ICON: u32 = 24;
/// The header's icons (✕, ✓), and their tap area.
const HEADER_ICON: f32 = 20.0;

#[derive(Clone, Copy)]
struct Icons {
    close: SpriteId,
    confirm: SpriteId,
    warning: SpriteId,
    bluetooth: SpriteId,
    /// By [`DeviceKind`], in [`DeviceKind::ALL`]'s order.
    devices: [SpriteId; DeviceKind::ALL.len()],
}
impl Resource for Icons {}

impl Icons {
    /// The icon for bluetoothd's `Icon`; Bluetooth's for none, or one
    /// without its own.
    fn device(&self, icon: Option<&str>) -> SpriteId {
        icon.and_then(prompt::device_kind)
            .map_or(self.bluetooth, |kind| self.devices[kind as usize])
    }
}

fn device_svg(kind: DeviceKind) -> &'static [u8] {
    use assets::icons::devices::*;
    match kind {
        DeviceKind::Phone => PHONE,
        DeviceKind::Laptop => LAPTOP,
        DeviceKind::Headphones => HEADPHONES,
        DeviceKind::Speaker => SPEAKER,
        DeviceKind::Keyboard => KEYBOARD,
        DeviceKind::Mouse => MOUSE,
        DeviceKind::GameController => GAME_CONTROLLER,
        DeviceKind::Pen => PEN,
        DeviceKind::Display => DISPLAY,
        DeviceKind::Camera => CAMERA,
        DeviceKind::Printer => PRINTER,
    }
}

struct Dialog {
    window: PromptWindow<PairingDialog>,
    /// The request, shown or waiting its turn. Holds no answer: what is
    /// typed stays in the field until sent.
    shown: Option<Request>,
    /// Why the typed answer was not sent.
    error: Option<&'static str>,
}
impl Resource for Dialog {}

impl Prompt for Dialog {
    const OWNER: Owner = Owner::Bluetooth;
    type View = PairingDialog;

    fn window(&self) -> &PromptWindow<PairingDialog> {
        &self.window
    }

    fn window_mut(&mut self) -> &mut PromptWindow<PairingDialog> {
        &mut self.window
    }

    fn has_request(&self) -> bool {
        self.shown.is_some()
    }

    fn showing(app: &mut App) {
        refresh(app, true);
    }

    /// The view, seeing no turn, also shuts the fields.
    fn hiding(app: &mut App) {
        refresh(app, true);
    }
}

impl Dialog {
    fn shown(&mut self, id: u64) -> Option<&mut Request> {
        self.shown.as_mut().filter(|r| r.id == id)
    }
}

/// Spawn the dialog's window, hidden. Install before the keyboard.
pub(crate) fn install(app: &mut App) {
    if app.has_resource::<Dialog>() {
        return;
    }
    let icons = {
        let mut atlas = app.resource_mut::<Atlas>();
        let mut load = |svg, size, what| {
            atlas
                .insert_svg(svg, size)
                .unwrap_or_else(|e| panic!("the {what} icon loads: {e:?}"))
        };
        Icons {
            close: load(assets::icons::CLEAR, HEADER_ICON as u32, "x"),
            confirm: load(assets::icons::CHECK, HEADER_ICON as u32, "check"),
            warning: load(assets::icons::WARNING, ICON, "warning"),
            bluetooth: load(assets::icons::bluetooth::DEFAULT, ICON, "bluetooth"),
            devices: DeviceKind::ALL.map(|kind| load(device_svg(kind), ICON, "device")),
        }
    };
    app.insert_resource(icons);
    let window = PromptWindow::spawn(app, NAMESPACE, PairingDialogBuilder);
    app.insert_resource(Dialog {
        window,
        shown: None,
        error: None,
    });
    prompt_queue::install::<Dialog>(app);
    app.system(|app, e: &PairingRequested| {
        {
            let mut dialog = app.resource_mut::<Dialog>();
            dialog.shown = Some(e.0.clone());
            dialog.error = None;
        }
        prompt_queue::request::<Dialog>(app);
    })
    .system(|app, e: &PairingUpdated| {
        let moved = {
            let mut dialog = app.resource_mut::<Dialog>();
            match dialog.shown(e.id).map(|r| &mut r.kind) {
                Some(RequestKind::DisplayPasskey { entered, .. }) => {
                    *entered = e.entered;
                    true
                }
                _ => false,
            }
        };
        if moved {
            refresh(app, false);
        }
    })
    .system(|app, e: &PairingDevice| {
        let named = {
            let mut dialog = app.resource_mut::<Dialog>();
            dialog
                .shown(e.id)
                .map(|r| r.device = e.device.clone())
                .is_some()
        };
        if named {
            refresh(app, false);
        }
    })
    .system(|app, e: &PairingCancelled| close(app, e.id))
    .system(|app, e: &PairingAnswer| close(app, e.id))
    .system(|app, e: &PairingAbort| close(app, e.id));
}

fn close(app: &mut App, id: u64) {
    {
        let mut dialog = app.resource_mut::<Dialog>();
        if dialog.shown(id).is_none() {
            return;
        }
        dialog.shown = None;
        dialog.error = None;
    }
    // Shown, hidden and the next prompt's turn; waiting, out of line.
    prompt_queue::release::<Dialog>(app);
}

/// Redraw from the [`Dialog`]; `clear` empties the fields. Only with the
/// turn: a waiting request takes nothing.
fn refresh(app: &mut App, clear: bool) {
    prompt_queue::emit::<Dialog, _>(app, Refresh { clear });
}

struct Refresh {
    clear: bool,
}
impl Event for Refresh {}

#[cfg(test)]
mod tests;
