//! A prompt's window: full screen on the overlay layer, above every other
//! shell window, shown only while the prompt has the turn.

use mecha_wayland::prelude::*;

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

/// The window a [`Prompt`](crate::Prompt) shows in, its view, and whether
/// the prompt has the turn. The crate shows and hides it with the turn;
/// the prompt only draws into the view, with [`emit`](crate::emit).
#[derive(Debug)]
pub struct PromptWindow<V> {
    window: NodeId,
    view: Handle<V>,
    turn: bool,
}

impl<V> PromptWindow<V> {
    /// Spawn the window, hidden, with `view` in it. `namespace` names the
    /// layer surface, and the window.
    ///
    /// While shown it takes the keyboard, and while it has the focus the
    /// compositor stacks the on-screen keyboard over it. Hidden, it takes
    /// nothing.
    pub fn spawn<B: Build<Widget = V>>(app: &mut App, namespace: &str, view: B) -> Self {
        let root = app.root();
        let role = Role::Layer(LayerRole {
            layer: Layer::Overlay,
            anchor: ALL_SIDES,
            // Clear of the zones others reserve: below the status bar.
            exclusive_zone: 0,
            namespace: namespace.into(),
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            shell_role: Some(ShellRole::Prompt),
        });
        let window = app
            .spawn_with(
                root,
                window()
                    .title(namespace)
                    .layout(LayoutStyle::default())
                    .clear(Color::TRANSPARENT),
                (role,),
            )
            .id();
        let view = app.spawn(window, view);
        let me = PromptWindow {
            window,
            view,
            turn: false,
        };
        set_hidden(app, window, true);
        me
    }

    /// The view drawn into.
    pub fn view(&self) -> Handle<V> {
        self.view
    }

    /// Whether the prompt has the turn: the window shows. Without it, a
    /// request kept waits, hidden, and the view should take nothing, the
    /// keyboard focus least of all.
    pub fn has_turn(&self) -> bool {
        self.turn
    }

    pub(crate) fn set_turn(&mut self, turn: bool) {
        self.turn = turn;
    }

    pub(crate) fn window(&self) -> NodeId {
        self.window
    }

    #[cfg(test)]
    pub(crate) fn from_parts(window: NodeId, view: Handle<V>) -> Self {
        PromptWindow {
            window,
            view,
            turn: false,
        }
    }
}

/// Hide or show `window`, keeping its surface.
pub(crate) fn set_hidden(app: &mut App, window: NodeId, hidden: bool) {
    if !app.has_resource::<Surfaces>() || !app.has_resource::<Wayland>() {
        return;
    }
    let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    surfaces.set_hidden(window, hidden, &mut wl);
}

/// Drop the keyboard focus, if it is in `window`: hidden, the window
/// takes nothing.
pub(crate) fn let_keyboard_go(app: &mut App, window: NodeId) {
    if !app.has_resource::<KeyboardFocus>() {
        return;
    }
    let Some(focused) = app.resource::<KeyboardFocus>().focused() else {
        return;
    };
    let inside = focused == window || app.ancestors(focused).any(|a| a == window);
    if inside {
        app.resource_mut::<KeyboardFocus>().blur();
    }
}
