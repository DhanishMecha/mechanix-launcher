//! The polkit dialog: a bottom sheet asking for the password of the
//! request's user.
//!
//! Shown on [`AuthRequested`], hidden on [`AuthFinished`] or
//! [`AuthCancelled`]. Return or ✓ sends [`AuthAnswer`]; ✕ or dragging the
//! sheet down sends [`AuthCancel`](crate::signals::AuthCancel).
//!
//! The window is an overlay-layer `prompt` (mx_shell_surface_v1), above
//! every other shell window. It takes the keyboard while shown, and while
//! it has the focus the compositor stacks the on-screen keyboard over it.
//!
//! One system prompt shows at a time ([`prompt_queue`]): asked for while
//! another shows, the request waits with the window hidden, and leaves the
//! keyboard focus alone, until [`PromptTurn`]. What it is told meanwhile
//! is kept, and shown then.
//!
//! Not yet: closing for the lock screen, a user picker, the requesting
//! program.

use mecha_wayland::prelude::*;
use prompt_queue::{Owner, PromptTurn};
use std::time::Instant;

use crate::signals::{
    AuthAnswer, AuthCancelled, AuthFinished, AuthLocked, AuthMessage, AuthPrompt, AuthRequested,
    AuthRetry, AuthUser,
};

mod view;

use view::{PolkitDialog, PolkitDialogBuilder};

const NAMESPACE: &str = "polkit-agent";

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

const ICON: u32 = 24;
/// The header's icons (✕, ✓), and their tap area.
const HEADER_ICON: f32 = 20.0;

const WRONG: &str = "Incorrect password. Try again.";
const CAPS_LOCK: &str = "Caps Lock is on";
#[derive(Clone, Copy)]
struct Icons {
    close: SpriteId,
    confirm: SpriteId,
    show: SpriteId,
    hide: SpriteId,
    warning: SpriteId,
}
impl Resource for Icons {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Prompt {
    text: String,
    echo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    text: String,
    error: bool,
}

/// Holds the cookie: no `Debug`, so it can't be logged.
struct Shown {
    cookie: String,
    message: String,
    user: Option<String>,
    prompt: Option<Prompt>,
    /// The last prompt's text, kept as the placeholder between prompts.
    last_prompt: Option<String>,
    note: Option<Note>,
    /// An answer was sent: it stays in the field, disabled, until the
    /// helper says what it made of it.
    checking: bool,
    /// Locked out until then, after too many wrong answers.
    locked: Option<Instant>,
}

struct Dialog {
    window: NodeId,
    view: Handle<PolkitDialog>,
    shown: Option<Shown>,
    /// A hardware keyboard's Caps Lock, as the compositor last said.
    caps_lock: bool,
    /// The request shown has the prompt's turn: the window shows. Without
    /// it, the request waits, hidden.
    turn: bool,
}
impl Resource for Dialog {}

impl Dialog {
    fn shown(&mut self, cookie: &str) -> Option<&mut Shown> {
        self.shown.as_mut().filter(|s| s.cookie == cookie)
    }

    fn cookie(&self) -> Option<String> {
        self.shown.as_ref().map(|s| s.cookie.clone())
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
            show: load(assets::icons::EYE, ICON, "eye"),
            hide: load(assets::icons::EYE_SLASH, ICON, "eye-slash"),
            warning: load(assets::icons::WARNING, ICON, "warning"),
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
    let view = app.spawn(window, PolkitDialogBuilder);
    app.insert_resource(Dialog {
        window,
        view,
        shown: None,
        caps_lock: false,
        turn: false,
    });
    set_hidden(app, true);
    prompt_queue::install(app);
    app.system(|app, e: &AuthRequested| {
        app.resource_mut::<Dialog>().shown = Some(Shown {
            cookie: e.0.cookie.clone(),
            message: e.0.message.clone(),
            user: None,
            prompt: None,
            last_prompt: None,
            note: None,
            checking: false,
            locked: None,
        });
        if prompt_queue::request(app, Owner::Polkit) {
            take_turn(app);
        }
    })
    .system(|app, e: &PromptTurn| {
        if e.0 != Owner::Polkit {
            return;
        }
        if app.resource::<Dialog>().shown.is_some() {
            take_turn(app);
        } else {
            prompt_queue::release(app, Owner::Polkit);
        }
    })
    .system(|app, e: &AuthUser| {
        update(app, &e.cookie, |s| {
            s.user = Some(e.user.clone());
            false
        });
    })
    .system(|app, e: &AuthPrompt| {
        update(app, &e.cookie, |s| {
            s.last_prompt = Some(e.text.clone());
            s.prompt = Some(Prompt {
                text: e.text.clone(),
                echo: e.echo,
            });
            s.locked = None;
            std::mem::take(&mut s.checking)
        });
    })
    // Too many wrong answers: count down to the next attempt.
    .system(|app, e: &AuthLocked| {
        update(app, &e.cookie, |s| {
            s.prompt = None;
            s.checking = false;
            s.note = None;
            s.locked = Some(Instant::now() + e.wait);
            true
        });
        countdown(app);
    })
    .system(|app, e: &AuthMessage| {
        update(app, &e.cookie, |s| {
            s.note = Some(Note {
                text: e.text.clone(),
                error: e.error,
            });
            false
        });
    })
    .system(|app, e: &AuthRetry| {
        update(app, &e.cookie, |s| {
            s.prompt = None;
            s.checking = false;
            s.note = Some(Note {
                text: WRONG.into(),
                error: true,
            });
            true
        });
    })
    .system(|app, e: &AuthAnswer| {
        update(app, &e.cookie, |s| {
            s.prompt = None;
            s.checking = true;
            s.note = None;
            false
        });
    })
    // Sent each time the dialog takes the keyboard, and on each change.
    .system(|app, e: &ModifiersChanged| {
        let shown = {
            let mut dialog = app.resource_mut::<Dialog>();
            let was = std::mem::replace(&mut dialog.caps_lock, e.0.caps_lock);
            was != e.0.caps_lock && dialog.shown.is_some()
        };
        if shown {
            refresh(app, Clear::Nothing);
        }
    })
    .system(|app, e: &AuthFinished| close(app, &e.cookie))
    .system(|app, e: &AuthCancelled| close(app, &e.cookie));
}

/// `change` says whether to empty the field.
fn update(app: &mut App, cookie: &str, change: impl FnOnce(&mut Shown) -> bool) {
    let empty_field = {
        let mut dialog = app.resource_mut::<Dialog>();
        let Some(shown) = dialog.shown(cookie) else {
            return;
        };
        change(shown)
    };
    refresh(
        app,
        if empty_field {
            Clear::Field
        } else {
            Clear::Nothing
        },
    );
}

/// The request waiting has its turn: show it, as it stands now.
fn take_turn(app: &mut App) {
    app.resource_mut::<Dialog>().turn = true;
    set_hidden(app, false);
    refresh(app, Clear::All);
    countdown(app);
}

/// Count a lockout down, if the dialog shows one.
fn countdown(app: &mut App) {
    let (turn, locked, view) = {
        let dialog = app.resource::<Dialog>();
        let locked = dialog.shown.as_ref().is_some_and(|s| s.locked.is_some());
        (dialog.turn, locked, dialog.view)
    };
    if turn && locked {
        app.emit(Countdown, view);
    }
}

fn close(app: &mut App, cookie: &str) {
    let turn = {
        let mut dialog = app.resource_mut::<Dialog>();
        if dialog.shown(cookie).is_none() {
            return;
        }
        dialog.shown = None;
        dialog.turn
    };
    if turn {
        refresh(app, Clear::All);
        set_hidden(app, true);
        app.resource_mut::<Dialog>().turn = false;
    }
    // Shown, the next prompt's turn; waiting, out of line.
    prompt_queue::release(app, Owner::Polkit);
}

/// Redraw from the [`Dialog`]: only with the turn, so a waiting request
/// takes nothing, the keyboard focus least of all.
fn refresh(app: &mut App, clear: Clear) {
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

/// Start the lockout's countdown.
struct Countdown;
impl Event for Countdown {}

struct Refresh {
    clear: Clear,
}
impl Event for Refresh {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Clear {
    Nothing,
    Field,
    /// Empty the field and hide ✓: a new request, or none.
    All,
}

#[cfg(test)]
mod tests;
