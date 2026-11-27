use mecha_wayland::prelude::*;
use prompt_queue::{Owner, PromptWindow};
use std::time::Instant;

use crate::signals::{
    AuthAnswer, AuthCancelled, AuthFinished, AuthLocked, AuthMessage, AuthPrompt, AuthRequested,
    AuthRetry, AuthUser,
};

mod view;

use view::{PolkitDialog, PolkitDialogBuilder};

const NAMESPACE: &str = "polkit-agent";

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
    window: PromptWindow<PolkitDialog>,
    /// The request, shown or waiting its turn.
    shown: Option<Shown>,
    /// A hardware keyboard's Caps Lock, as the compositor last said.
    caps_lock: bool,
}
impl Resource for Dialog {}

impl prompt_queue::Prompt for Dialog {
    const OWNER: Owner = Owner::Polkit;
    type View = PolkitDialog;

    fn window(&self) -> &PromptWindow<PolkitDialog> {
        &self.window
    }

    fn window_mut(&mut self) -> &mut PromptWindow<PolkitDialog> {
        &mut self.window
    }

    fn has_request(&self) -> bool {
        self.shown.is_some()
    }

    fn showing(app: &mut App) {
        refresh(app, Clear::All);
        countdown(app);
    }

    /// The keyboard is let go by the queue; the view, seeing no turn,
    /// shuts the field so it does not take it back.
    fn hiding(app: &mut App) {
        refresh(app, Clear::All);
    }
}

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
    let window = PromptWindow::spawn(app, NAMESPACE, PolkitDialogBuilder);
    app.insert_resource(Dialog {
        window,
        shown: None,
        caps_lock: false,
    });
    prompt_queue::install::<Dialog>(app);
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
        prompt_queue::request::<Dialog>(app);
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

/// Count a lockout down, if the dialog shows one.
fn countdown(app: &mut App) {
    let locked = app
        .resource::<Dialog>()
        .shown
        .as_ref()
        .is_some_and(|s| s.locked.is_some());
    if locked {
        prompt_queue::emit::<Dialog, _>(app, Countdown);
    }
}

fn close(app: &mut App, cookie: &str) {
    {
        let mut dialog = app.resource_mut::<Dialog>();
        if dialog.shown(cookie).is_none() {
            return;
        }
        dialog.shown = None;
    }
    // Shown, hidden and the next prompt's turn; waiting, out of line.
    prompt_queue::release::<Dialog>(app);
}

/// Redraw from the [`Dialog`]: only with the turn, so a waiting request
/// takes nothing, the keyboard focus least of all.
fn refresh(app: &mut App, clear: Clear) {
    prompt_queue::emit::<Dialog, _>(app, Refresh { clear });
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
