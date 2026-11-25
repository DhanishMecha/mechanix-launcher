//! The dialog's widget: the sheet, its header, the device, the code or
//! the field, drawn from the [`Dialog`]'s state.

use mecha_wayland::mechanix_widgets::prelude::{
    BottomSheet, BottomSheetContextExt, FontContextExt, FontWeight, SheetClosed, Text,
    TextContextExt, TextOverflow, TextVariant, VerticalTrim, bottom_sheet, text,
};
use mecha_wayland::prelude::*;

use super::prompt::{self, Entry};
use super::{Dialog, HEADER_ICON, Icons, Refresh};
use crate::signals::{PairingAbort, PairingAnswer, Reply, RequestKind};

const TAP: f32 = 48.0;
const FIELD_HEIGHT: f32 = 40.0;
const OK_CIRCLE: f32 = 32.0;
/// The header's side padding: ✕ (48), the title, ✓ (48), no gaps.
const HEADER_SIDE: f32 = 8.0;
const DISABLED_OPACITY: f32 = 0.10;
const AVATAR: f32 = 40.0;
const STROKE: f32 = 1.0;

pub(super) struct PairingDialog {
    pub(super) parts: Parts,
    /// The active field is empty.
    pub(super) empty: bool,
    /// The active field's length, to tell an edit from anything else.
    pub(super) len: usize,
}

#[derive(Clone, Copy)]
pub(super) struct Parts {
    pub(super) sheet: Handle<BottomSheet>,
    pub(super) title: Handle<Text>,
    pub(super) cancel: Handle<Div>,
    pub(super) ok: Handle<Div>,
    pub(super) ok_icon: Handle<Icon>,
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

fn tap_area() -> LayoutStyle {
    LayoutStyle::default()
        .size(px(TAP), px(TAP))
        .shrink(0.0)
        .center()
}

/// Replace `node`'s layout with `change` of it.
fn restyle(s: Spawn, node: impl Into<NodeId>, change: impl FnOnce(LayoutStyle) -> LayoutStyle) {
    let mut style = s.component_mut::<LayoutStyle>(node).unwrap();
    let changed = change(style.clone());
    *style = changed;
}

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
        let (name, address) = build_device(s, body, icons);
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
        }
    }
}

/// Fit the sheet's header and body to the dialog; returns them.
fn style_sheet(s: Spawn, sheet: Handle<BottomSheet>) -> (NodeId, NodeId) {
    let (header, body, title) = {
        let w = s.widget::<BottomSheet>(sheet).expect("the sheet is built");
        (w.header(), w.body(), w.title())
    };
    // Our title replaces the sheet's, which would sit before the ✕.
    restyle(s, title, LayoutStyle::hidden);
    restyle(s, header, |style| {
        style
            .justify(Justify::Start)
            .column_gap(px(0.0))
            .padding(Insets::new(
                px(Spacing::Space100.dp()),
                px(HEADER_SIDE),
                px(Spacing::Space100.dp()),
                px(HEADER_SIDE),
            ))
    });
    restyle(s, body, |style| {
        style
            .row_gap(px(Spacing::Space200.dp()))
            .padding(Insets::new(
                px(0.0),
                px(Spacing::Space300.dp()),
                px(Spacing::Space300.dp()),
                px(Spacing::Space300.dp()),
            ))
    });
    (header.into(), body.into())
}

/// ✕, the title, and ✓.
fn build_header(
    s: Spawn,
    header: NodeId,
    icons: Icons,
) -> (Handle<Div>, Handle<Text>, Handle<Div>, Handle<Icon>) {
    let header_icon = Size::new(HEADER_ICON, HEADER_ICON);
    let cancel = s.spawn(header, div().style(tap_area()));
    s.spawn(
        cancel,
        icon(icons.close)
            .size(header_icon)
            .color(s.color(ColorRole::OnSurface)),
    );
    let title_large = s.typography(TextVariant::TitleLarge);
    let title = s.spawn(
        header,
        text("")
            .variant(TextVariant::TitleLarge)
            .weight(FontWeight::Regular)
            .line_height(title_large.line_height)
            .letter_spacing(title_large.letter_spacing)
            .vertical_trim(VerticalTrim::CapHeight)
            .color(ColorRole::OnSurface)
            .max_lines(1)
            .overflow(TextOverflow::Ellipsis)
            // At the start, after ✕: the auto margin pushes ✓ to the end.
            .style(LayoutStyle::default().margin(Insets::new(px(0.0), auto(), px(0.0), px(0.0)))),
    );
    let ok = s.spawn(header, div().style(tap_area()));
    let ok_circle = s.spawn(
        ok,
        div()
            .style(
                LayoutStyle::default()
                    .size(px(OK_CIRCLE), px(OK_CIRCLE))
                    .center(),
            )
            .background(s.color(ColorRole::SurfaceContainerHighest))
            .radius(OK_CIRCLE / 2.0),
    );
    let ok_icon = s.spawn(
        ok_circle,
        icon(icons.confirm)
            .size(header_icon)
            .color(s.color(ColorRole::OnSurface)),
    );
    (cancel, title, ok, ok_icon)
}

/// The Bluetooth avatar beside the device's name and address.
fn build_device(s: Spawn, body: NodeId, icons: Icons) -> (Handle<Text>, Handle<Text>) {
    let row = s.spawn(
        body,
        div().style(
            LayoutStyle::default()
                .row()
                .align_items(Align::Center)
                .gap(px(Spacing::Space200.dp())),
        ),
    );
    let avatar = s.spawn(
        row,
        div()
            .style(
                LayoutStyle::default()
                    .size(px(AVATAR), px(AVATAR))
                    .shrink(0.0)
                    .center(),
            )
            .background(s.color(ColorRole::SecondaryFixedDim))
            .radius(AVATAR / 2.0),
    );
    s.spawn(
        avatar,
        icon(icons.bluetooth).color(s.color(ColorRole::Primary)),
    );
    let words = s.spawn(
        row,
        div().style(
            LayoutStyle::default()
                .column()
                .grow(1.0)
                .shrink(1.0)
                .min_width(px(0.0)),
        ),
    );
    let name = s.spawn(
        words,
        text("")
            .variant(TextVariant::TitleMedium)
            .color(ColorRole::OnSurface)
            .max_lines(1)
            .overflow(TextOverflow::Ellipsis),
    );
    let address = s.spawn(
        words,
        text("")
            .variant(TextVariant::BodySmall)
            .color(ColorRole::OnSurfaceVariant)
            .max_lines(1),
    );
    (name, address)
}

/// The code, centred and large, and the keys typed of it.
fn build_code(s: Spawn, body: NodeId) -> (Handle<Div>, Handle<Text>, Handle<Text>) {
    let code_box = s.spawn(
        body,
        div().style(
            LayoutStyle::default()
                .column()
                .width(percent(100.0))
                .align_items(Align::Center)
                .gap(px(Spacing::Space100.dp()))
                .hidden(),
        ),
    );
    let code = s.spawn(
        code_box,
        text("")
            .variant(TextVariant::DisplaySmall)
            .color(ColorRole::OnSurface)
            .max_lines(1),
    );
    let progress = s.spawn(
        code_box,
        text("")
            .variant(TextVariant::BodySmall)
            .color(ColorRole::OnSurfaceVariant)
            .max_lines(1),
    );
    (code_box, code, progress)
}

fn build_field(s: Spawn, body: NodeId, icons: Icons, purpose: ContentPurpose, hint: &str) -> Field {
    let root = s.spawn(
        body,
        div().style(
            LayoutStyle::default()
                .column()
                .width(percent(100.0))
                .hidden(),
        ),
    );
    let field = s.spawn(
        root,
        div().style(LayoutStyle::default().relative().width(percent(100.0))),
    );
    let inset = px(Spacing::Space200.dp());
    let end_room = px(TAP);
    let font = s.font(FontWeight::Regular);
    // As the placeholder over it.
    let text_size = s.typography(TextVariant::BodyLarge).font_size.round() as u16;
    let input = s.spawn(
        field,
        input(font)
            .size(text_size)
            .color(s.color(ColorRole::OnSurface))
            .caret(s.color(ColorRole::Primary))
            .background(Color::TRANSPARENT)
            .border(0.0, Color::TRANSPARENT)
            .content_purpose(purpose)
            .style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .width(percent(100.0))
                    .min_height(px(FIELD_HEIGHT))
                    .padding(Insets::new(px(0.0), end_room, px(0.0), inset)),
            ),
    );
    *s.component_mut::<LayoutStyle>(input).unwrap() =
        LayoutStyle::default().relative().width(percent(100.0));
    // Inside the input, so a press on it focuses the field.
    let hint_box = s.spawn(
        input,
        div().style(
            LayoutStyle::default()
                .absolute()
                .inset(Insets::new(px(0.0), end_room, px(0.0), inset))
                .row()
                .align_items(Align::Center),
        ),
    );
    let placeholder = s.spawn(
        hint_box,
        text(hint)
            .variant(TextVariant::BodyLarge)
            .color(ColorRole::OnSurfaceVariant)
            .max_lines(1)
            .overflow(TextOverflow::Ellipsis),
    );
    let alert = s.spawn(
        field,
        div().style(
            LayoutStyle::default()
                .absolute()
                .inset(Insets::new(px(0.0), px(0.0), px(0.0), auto()))
                .width(end_room)
                .center()
                .hidden(),
        ),
    );
    s.spawn(alert, icon(icons.warning).color(s.color(ColorRole::Error)));
    let line = s.spawn(
        root,
        div()
            .style(LayoutStyle::default().size(percent(100.0), px(STROKE)))
            .background(s.color(ColorRole::Primary)),
    );
    let note = s.spawn(
        root,
        text("")
            .variant(TextVariant::BodySmall)
            .color(ColorRole::Error)
            .style(LayoutStyle::default().padding(Insets::new(
                px(Spacing::Space100.dp()),
                inset,
                px(0.0),
                inset,
            ))),
    );
    Field {
        root,
        input,
        placeholder,
        alert,
        line,
        note,
    }
}

fn handle_events(s: Spawn, me: Handle<PairingDialog>, p: &Parts) {
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
            sync(ctx);
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
    sync(ctx);
}

fn display(shown: bool) -> Display {
    if shown {
        Display::Flex
    } else {
        Display::Hidden
    }
}

fn sync(ctx: Ctx) {
    let (kind, name, address, error) = {
        let dialog = ctx.resource::<Dialog>();
        match &dialog.shown {
            Some(r) => (
                Some(r.kind.clone()),
                r.device.name.clone(),
                r.device.address.clone(),
                dialog.error,
            ),
            None => (None, String::new(), String::new(), None),
        }
    };
    let p = ctx.me().parts;
    let empty = ctx.me().empty;
    let entry = kind.as_ref().and_then(prompt::entry);
    let code = kind.as_ref().and_then(prompt::code);
    let progress = kind.as_ref().and_then(prompt::progress);

    if let Some(mut t) = ctx.at(p.title) {
        t.set_text(kind.as_ref().map_or("", prompt::title));
    }
    if let Some(mut t) = ctx.at(p.message) {
        t.set_text(kind.as_ref().map(prompt::message).unwrap_or_default());
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

    // Nothing to answer while a code is only shown.
    let answerable = kind.as_ref().is_some_and(|k| !k.is_display());
    if let Some(mut b) = ctx.at(p.ok) {
        b.set_display(display(answerable));
    }
    let on_surface = ctx.color(ColorRole::OnSurface);
    if let Some(mut i) = ctx.at(p.ok_icon) {
        i.set_color(if entry.is_some() && empty {
            on_surface.with_alpha(DISABLED_OPACITY)
        } else {
            on_surface
        });
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
