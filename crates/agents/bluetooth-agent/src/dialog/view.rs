//! The dialog's widget: the sheet, its header, the device, the code or
//! the field, drawn from the [`Dialog`](super::Dialog)'s state.
//!
//! - `build`: spawning the parts;
//! - `update`: answering, following what is typed, and redrawing.

use mecha_wayland::mechanix_widgets::prelude::{BottomSheet, Text, bottom_sheet, text};
use mecha_wayland::prelude::*;

use super::Icons;
use super::prompt::Entry;

mod build;
mod update;

use build::{build_code, build_device, build_field, build_header, style_sheet};
use update::handle_events;

pub(super) struct PairingDialog {
    pub(super) parts: Parts,
    /// The active field is empty.
    pub(super) empty: bool,
    /// The active field's length, to tell an edit from anything else.
    pub(super) len: usize,
    /// The avatar's sprite: the device's kind.
    pub(super) avatar: SpriteId,
    /// The field given the keyboard, if any.
    pub(super) focused: Option<Entry>,
}

#[derive(Clone, Copy)]
pub(super) struct Parts {
    pub(super) sheet: Handle<BottomSheet>,
    pub(super) title: Handle<Text>,
    pub(super) cancel: Handle<Div>,
    pub(super) ok: Handle<Div>,
    pub(super) ok_icon: Handle<Icon>,
    pub(super) avatar: Handle<Icon>,
    pub(super) name: Handle<Text>,
    pub(super) address: Handle<Text>,
    pub(super) message: Handle<Text>,
    pub(super) code_box: Handle<Div>,
    pub(super) code: Handle<Text>,
    pub(super) progress: Handle<Text>,
    pub(super) pin: Field,
    pub(super) passkey: Field,
}

/// An answer field: the input with its placeholder, the alert at its
/// end, the underline, and the note below.
#[derive(Clone, Copy)]
pub(super) struct Field {
    pub(super) root: Handle<Div>,
    pub(super) input: Handle<Input>,
    pub(super) placeholder: Handle<Text>,
    pub(super) alert: Handle<Div>,
    pub(super) line: Handle<Div>,
    pub(super) note: Handle<Text>,
}

impl Parts {
    fn field(&self, entry: Entry) -> Field {
        match entry {
            Entry::Pin => self.pin,
            Entry::Passkey => self.passkey,
        }
    }
}

pub(super) struct PairingDialogBuilder;

impl Build for PairingDialogBuilder {
    type Widget = PairingDialog;
}

type Spawn<'a, 'b> = &'a mut Spawner<'b, PairingDialog>;
type Ctx<'a, 'b> = &'a mut Context<'b, PairingDialog>;

impl Widget for PairingDialog {
    type Builder = PairingDialogBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .absolute()
            .inset(Insets::all(px(0.0)))
            .size(percent(100.0), percent(100.0));
        let icons = *s.resource::<Icons>();

        let sheet = s.spawn(me, bottom_sheet().height(percent(100.0)));
        let (header, body) = style_sheet(s, sheet);
        let (cancel, title, ok, ok_icon) = build_header(s, header, icons);
        let (avatar, name, address) = build_device(s, body, icons);
        let message = s.spawn(
            body,
            text("")
                .wrap(TextWrap::Word)
                .variant(TextVariant::BodyMedium)
                .color(ColorRole::OnSurfaceVariant),
        );
        let (code_box, code, progress) = build_code(s, body);
        let pin = build_field(s, body, icons, ContentPurpose::Normal, "PIN");
        let passkey = build_field(s, body, icons, ContentPurpose::Digits, "Passkey");
        let parts = Parts {
            sheet,
            title,
            cancel,
            ok,
            ok_icon,
            avatar,
            name,
            address,
            message,
            code_box,
            code,
            progress,
            pin,
            passkey,
        };
        handle_events(s, me, &parts);

        PairingDialog {
            parts,
            empty: true,
            len: 0,
            avatar: icons.bluetooth,
            focused: None,
        }
    }
}
