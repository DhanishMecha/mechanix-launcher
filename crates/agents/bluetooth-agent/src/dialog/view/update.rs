//! What the dialog does: answering on ✓, Return, ✕ or a drag down,
//! following what is typed, and redrawing from the [`Dialog`].

use mecha_wayland::mechanix_widgets::prelude::{
    BottomSheetContextExt, SheetClosed, TextContextExt,
};
use mecha_wayland::prelude::*;

use super::super::prompt::{self, Entry};
use super::super::{Dialog, Icons, Refresh};
use super::{Ctx, PairingDialog, Parts, Spawn};
use crate::signals::{PairingAbort, PairingAnswer, Reply, RequestKind};

const DISABLED_OPACITY: f32 = 0.10;

pub(super) fn handle_events(s: Spawn, me: Handle<PairingDialog>, p: &Parts) {
    for field in [p.pin, p.passkey] {
        s.on::<InputSubmit>(field.input, |ctx, _| submit(ctx));
        // After the input's own handlers: what it holds now.
        s.on::<InputEdit>(field.input, |ctx, _| typed(ctx));
    }
    s.on::<Clicked>(p.ok, |ctx, _| submit(ctx));
    s.on::<Clicked>(p.cancel, |ctx, _| dismiss(ctx));
    s.on::<SheetClosed>(p.sheet, |ctx, _| dismiss(ctx));
    s.on::<Refresh>(me, show);
}

/// The request shown: its id and kind.
fn shown(ctx: Ctx) -> Option<(u64, RequestKind)> {
    ctx.resource::<Dialog>()
        .shown
        .as_ref()
        .map(|r| (r.id, r.kind.clone()))
}

fn typed_text(ctx: Ctx, entry: Entry) -> String {
    let input = ctx.me().parts.field(entry).input;
    ctx.widget::<Input>(input.id())
        .map(|i| i.text().to_owned())
        .unwrap_or_default()
}

/// ✓ or Return: send the answer, or say why it can't be sent.
fn submit(ctx: Ctx) {
    let Some((id, kind)) = shown(ctx) else {
        return;
    };
    let reply = match prompt::entry(&kind) {
        Some(_) if ctx.me().empty => return,
        Some(Entry::Pin) => prompt::read_pin(&typed_text(ctx, Entry::Pin)).map(Reply::PinCode),
        Some(Entry::Passkey) => {
            prompt::read_passkey(&typed_text(ctx, Entry::Passkey)).map(Reply::Passkey)
        }
        None if kind.is_display() => return,
        None => Ok(Reply::Accept),
    };
    match reply {
        Ok(reply) => ctx.signal(PairingAnswer { id, reply }),
        Err(why) => {
            ctx.resource_mut::<Dialog>().error = Some(why);
            sync_answer(ctx);
        }
    }
}

/// ✕ or a drag down: reject the request, or stop showing the code.
fn dismiss(ctx: Ctx) {
    let Some((id, kind)) = shown(ctx) else {
        return;
    };
    if kind.is_display() {
        ctx.signal(PairingAbort { id });
    } else {
        ctx.signal(PairingAnswer {
            id,
            reply: Reply::Reject,
        });
    }
}

/// Follow whether the field is empty; typing after an error clears it.
fn typed(ctx: Ctx) {
    let Some(entry) = shown(ctx).and_then(|(_, k)| prompt::entry(&k)) else {
        return;
    };
    let len = typed_text(ctx, entry).len();
    let edited = len != std::mem::replace(&mut ctx.me().len, len);
    ctx.me().empty = len == 0;
    if edited {
        ctx.resource_mut::<Dialog>().error = None;
    }
    sync_answer(ctx);
}

fn display(shown: bool) -> Display {
    if shown {
        Display::Flex
    } else {
        Display::Hidden
    }
}

/// Redraw all of it.
fn sync(ctx: Ctx) {
    sync_request(ctx);
    sync_answer(ctx);
}

/// The request: header, device, message, the code shown, and whether
/// ✓ shows at all.
fn sync_request(ctx: Ctx) {
    let (kind, name, address, sprite) = {
        let dialog = ctx.resource::<Dialog>();
        let icons = ctx.resource::<Icons>();
        match &dialog.shown {
            Some(r) => (
                Some(r.kind.clone()),
                r.device.name.clone(),
                r.device.address.clone(),
                icons.device(r.device.icon.as_deref()),
            ),
            None => (None, String::new(), String::new(), icons.bluetooth),
        }
    };
    let p = ctx.me().parts;
    let code = kind.as_ref().and_then(prompt::code);
    let progress = kind.as_ref().and_then(prompt::progress);

    if let Some(mut t) = ctx.at(p.title) {
        t.set_text(kind.as_ref().map_or("", prompt::title));
    }
    if let Some(mut t) = ctx.at(p.message) {
        t.set_text(kind.as_ref().map(prompt::message).unwrap_or_default());
    }
    if ctx.me().avatar != sprite {
        ctx.me().avatar = sprite;
        if let Some(mut i) = ctx.at(p.avatar) {
            i.set_sprite(sprite);
        }
    }
    // The address is the name until the device's is read.
    let same = name == address;
    if let Some(mut t) = ctx.at(p.name) {
        t.set_text(name);
    }
    if let Some(mut t) = ctx.at(p.address) {
        t.set_display(display(!same));
        t.set_text(address);
    }
    if let Some(mut b) = ctx.at(p.code_box) {
        b.set_display(display(code.is_some()));
    }
    if let Some(mut t) = ctx.at(p.code) {
        t.set_text(code.unwrap_or_default());
    }
    if let Some(mut t) = ctx.at(p.progress) {
        t.set_display(display(progress.is_some()));
        t.set_text(progress.unwrap_or_default());
    }
    // Nothing to answer while a code is only shown.
    let answerable = kind.as_ref().is_some_and(|k| !k.is_display());
    if let Some(mut b) = ctx.at(p.ok) {
        b.set_display(display(answerable));
    }
}

/// The answer: which field shows, its error, and whether ✓ is dimmed.
/// All that typing changes.
fn sync_answer(ctx: Ctx) {
    let (entry, error) = {
        let dialog = ctx.resource::<Dialog>();
        (
            dialog.shown.as_ref().and_then(|r| prompt::entry(&r.kind)),
            dialog.error,
        )
    };
    let p = ctx.me().parts;
    let empty = ctx.me().empty;
    let accent = ctx.color(if error.is_some() {
        ColorRole::Error
    } else {
        ColorRole::Primary
    });
    for which in [Entry::Pin, Entry::Passkey] {
        let f = p.field(which);
        let active = entry == Some(which);
        if let Some(mut r) = ctx.at(f.root) {
            r.set_display(display(active));
        }
        if let Some(mut i) = ctx.at(f.input) {
            i.set_disabled(!active);
            i.set_caret_color(accent);
        }
        if let Some(mut t) = ctx.at(f.placeholder) {
            t.set_display(display(empty));
        }
        if let Some(mut l) = ctx.at(f.line) {
            l.set_background(accent);
        }
        if let Some(mut a) = ctx.at(f.alert) {
            a.set_display(display(active && error.is_some()));
        }
        if let Some(mut t) = ctx.at(f.note) {
            t.set_display(display(active && error.is_some()));
            t.set_text(error.unwrap_or_default());
        }
    }
    let on_surface = ctx.color(ColorRole::OnSurface);
    if let Some(mut i) = ctx.at(p.ok_icon) {
        i.set_color(if entry.is_some() && empty {
            on_surface.with_alpha(DISABLED_OPACITY)
        } else {
            on_surface
        });
    }
    // Enabled above first: a disabled field takes no focus.
    if std::mem::replace(&mut ctx.me().focused, entry) != entry {
        match entry {
            Some(which) => focus_field(ctx, which),
            None => blur_fields(ctx),
        }
    }
}

/// `entry`'s field takes the keyboard: typing goes to it, and the
/// on-screen keyboard shows for it. No tap needed.
fn focus_field(ctx: Ctx, entry: Entry) {
    let input = ctx.me().parts.field(entry).input.id();
    ctx.emit(InputFocus { focused: true }, input);
    ctx.resource_mut::<KeyboardFocus>().focus(input);
}

/// Nothing to type: the fields give the keyboard up, if one has it.
fn blur_fields(ctx: Ctx) {
    let p = ctx.me().parts;
    let ours = [p.pin.input.id(), p.passkey.input.id()];
    let mut focus = ctx.resource_mut::<KeyboardFocus>();
    if focus.focused().is_some_and(|f| ours.contains(&f)) {
        focus.blur();
    }
}

fn show(ctx: Ctx, e: &Refresh) {
    let p = ctx.me().parts;
    if e.clear {
        for f in [p.pin, p.passkey] {
            if let Some(mut i) = ctx.at(f.input) {
                drop(i.take_text());
            }
        }
        ctx.me().empty = true;
        ctx.me().len = 0;
    }
    // Back up, if the user dragged it away last time.
    if ctx.resource::<Dialog>().shown.is_some()
        && let Some(mut sheet) = ctx.at(p.sheet)
    {
        sheet.open();
    }
    sync(ctx);
}
