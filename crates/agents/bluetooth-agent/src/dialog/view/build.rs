//! Spawning the dialog's parts: the sheet's header and body, the
//! device, the code, and the answer fields. Each starts hidden or empty;
//! [`super::update`] fills them in.

use mecha_wayland::mechanix_widgets::prelude::{
    BottomSheet, FontContextExt, FontWeight, Text, TextOverflow, TextVariant, VerticalTrim, text,
};
use mecha_wayland::prelude::*;

use super::super::{HEADER_ICON, Icons};
use super::{Field, Spawn};

const TAP: f32 = 48.0;
const FIELD_HEIGHT: f32 = 40.0;
const OK_CIRCLE: f32 = 32.0;
/// The header's side padding: ✕ (48), the title, ✓ (48), no gaps.
const HEADER_SIDE: f32 = 8.0;
const AVATAR: f32 = 40.0;
const STROKE: f32 = 1.0;

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

/// Fit the sheet's header and body to the dialog; returns them.
pub(super) fn style_sheet(s: Spawn, sheet: Handle<BottomSheet>) -> (NodeId, NodeId) {
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
pub(super) fn build_header(
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

/// The device's avatar beside its name and address.
pub(super) fn build_device(
    s: Spawn,
    body: NodeId,
    icons: Icons,
) -> (Handle<Icon>, Handle<Text>, Handle<Text>) {
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
    let avatar = s.spawn(
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
    (avatar, name, address)
}

/// The code, centred and large, and the keys typed of it.
pub(super) fn build_code(s: Spawn, body: NodeId) -> (Handle<Div>, Handle<Text>, Handle<Text>) {
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

pub(super) fn build_field(
    s: Spawn,
    body: NodeId,
    icons: Icons,
    purpose: ContentPurpose,
    hint: &str,
) -> Field {
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
