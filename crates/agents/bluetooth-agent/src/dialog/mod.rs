//! The pairing dialog: a bottom sheet for bluetoothd's requests.
//!
//! Shown on [`PairingRequested`], hidden on [`PairingCancelled`] or once
//! answered. The device shows by its address and the Bluetooth icon
//! until [`PairingDevice`] gives its name and kind. What it shows follows
//! the request:
//!
//! - a PIN or passkey to enter: a field, sent with ✓ or Return;
//! - a code to confirm, a pairing or a service to allow: ✓ accepts;
//! - a code to type on the device: shown, with the keys typed so far.
//!
//! ✕ or dragging the sheet down rejects a request, or stops the pairing
//! whose code is shown ([`PairingAbort`]).
//!
//! The window is an overlay-layer `prompt` (mx_shell_surface_v1), above
//! every other shell window. It takes the keyboard while shown, and while
//! it has the focus the compositor stacks the on-screen keyboard over it.
//!
//! One system prompt shows at a time ([`prompt_queue`]): asked for while
//! another shows, the request waits with the window hidden until
//! [`PromptTurn`]; what it is told meanwhile is kept, and shown then.

use mecha_wayland::prelude::*;
use prompt_queue::{Owner, PromptTurn};

use crate::signals::{
    PairingAbort, PairingAnswer, PairingCancelled, PairingDevice, PairingRequested, PairingUpdated,
    Request, RequestKind,
};

mod prompt;
mod view;

use prompt::DeviceKind;
use view::{PairingDialog, PairingDialogBuilder};

const NAMESPACE: &str = "bluetooth-agent";

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

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
    window: NodeId,
    view: Handle<PairingDialog>,
    /// The request shown. Holds no answer: what is typed stays in the
    /// field until sent.
    shown: Option<Request>,
    /// Why the typed answer was not sent.
    error: Option<&'static str>,
    /// The request shown has the prompt's turn: the window shows. Without
    /// it, the request waits, hidden.
    turn: bool,
}
impl Resource for Dialog {}

impl Dialog {
    /// The request shown, if it is request `id`.
    fn request_mut(&mut self, id: u64) -> Option<&mut Request> {
        self.shown.as_mut().filter(|r| r.id == id)
    }
}

/// An icon that draws nothing: in place of one that did not load.
const NO_ICON: Bitmap = Bitmap {
    width: 0,
    height: 0,
    format: Format::R8,
    pixels: Vec::new(),
};

/// `svg` as an icon `size` pixels high. One that does not load is left
/// blank, and logged: the dialog still asks without it.
fn load_icon(atlas: &mut Atlas, svg: &'static [u8], size: u32, what: &str) -> SpriteId {
    atlas.insert_svg(svg, size).unwrap_or_else(|e| {
        tracing::error!("bluetooth: the {what} icon did not load: {e:?}");
        atlas
            .insert(Class::Icon, &NO_ICON)
            .expect("an empty icon always inserts")
    })
}

/// Spawn the dialog's window, hidden. Install before the keyboard.
pub(crate) fn install(app: &mut App) {
    if app.has_resource::<Dialog>() {
        return;
    }
    let icons = {
        let mut atlas = app.resource_mut::<Atlas>();
        let mut load = |svg, size, what| load_icon(&mut atlas, svg, size, what);
        Icons {
            close: load(assets::icons::CLEAR, HEADER_ICON as u32, "x"),
            confirm: load(assets::icons::CHECK, HEADER_ICON as u32, "check"),
            warning: load(assets::icons::WARNING, ICON, "warning"),
            bluetooth: load(assets::icons::bluetooth::DEFAULT, ICON, "bluetooth"),
            devices: DeviceKind::ALL.map(|kind| load(device_svg(kind), ICON, "device")),
        }
    };
    app.insert_resource(icons);
    let root = app.root();
    let role = Role::Layer(LayerRole {
        layer: Layer::Overlay,
        anchor: ALL_SIDES,
        // Clear of the zones others reserve: below the status bar.
        exclusive_zone: 0,
        namespace: NAMESPACE.into(),
        // The keyboard is its while shown; hidden, it takes nothing.
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        shell_role: Some(ShellRole::Prompt),
    });
    let window = app
        .spawn_with(
            root,
            window()
                .title(NAMESPACE)
                .layout(LayoutStyle::default())
                .clear(Color::TRANSPARENT),
            (role,),
        )
        .id();
    let view = app.spawn(window, PairingDialogBuilder);
    app.insert_resource(Dialog {
        window,
        view,
        shown: None,
        error: None,
        turn: false,
    });
    set_hidden(app, true);
    prompt_queue::install(app);
    app.system(|app, e: &PairingRequested| {
        {
            let mut dialog = app.resource_mut::<Dialog>();
            dialog.shown = Some(e.0.clone());
            dialog.error = None;
        }
        if prompt_queue::request(app, Owner::Bluetooth) {
            take_turn(app);
        }
    })
    .system(|app, e: &PromptTurn| {
        if e.0 != Owner::Bluetooth {
            return;
        }
        if app.resource::<Dialog>().shown.is_some() {
            take_turn(app);
        } else {
            prompt_queue::release(app, Owner::Bluetooth);
        }
    })
    .system(|app, e: &PairingUpdated| {
        let moved = {
            let mut dialog = app.resource_mut::<Dialog>();
            match dialog.request_mut(e.id).map(|r| &mut r.kind) {
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
                .request_mut(e.id)
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

/// The request waiting has its turn: show it, as it stands now.
fn take_turn(app: &mut App) {
    app.resource_mut::<Dialog>().turn = true;
    set_hidden(app, false);
    refresh(app, true);
}

fn close(app: &mut App, id: u64) {
    let turn = {
        let mut dialog = app.resource_mut::<Dialog>();
        if dialog.request_mut(id).is_none() {
            return;
        }
        dialog.shown = None;
        dialog.error = None;
        dialog.turn
    };
    if turn {
        refresh(app, true);
        set_hidden(app, true);
        app.resource_mut::<Dialog>().turn = false;
    }
    // Shown, the next prompt's turn; waiting, out of line.
    prompt_queue::release(app, Owner::Bluetooth);
}

/// Redraw from the [`Dialog`]; `clear` empties the fields. Only with the
/// turn: a waiting request takes nothing.
fn refresh(app: &mut App, clear: bool) {
    let (turn, view) = {
        let dialog = app.resource::<Dialog>();
        (dialog.turn, dialog.view)
    };
    if turn {
        app.emit(Refresh { clear }, view);
    }
}

/// Hide or show the dialog's window, keeping its surface.
fn set_hidden(app: &mut App, hidden: bool) {
    if !app.has_resource::<Surfaces>() || !app.has_resource::<Wayland>() {
        return;
    }
    let window = app.resource::<Dialog>().window;
    let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    surfaces.set_hidden(window, hidden, &mut wl);
}

struct Refresh {
    clear: bool,
}
impl Event for Refresh {}

#[cfg(test)]
mod tests;
