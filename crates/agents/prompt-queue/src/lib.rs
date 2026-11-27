//! One system prompt at a time.
//!
//! The polkit and Bluetooth agents each show a full-screen prompt on the
//! overlay layer. Two at once would stack in creation order, not request
//! order, and the hidden one could take the keyboard focus from the one
//! on top. So a prompt asks for the turn before it shows ([`request`]),
//! and gives it back when it closes ([`release`]); a prompt asked for
//! while another shows waits, first come first served, and is told with
//! [`PromptTurn`] when it is its turn. Until then it stays hidden and
//! leaves the keyboard focus alone.
//!
//! No prompt goes ahead of another: the one shown is never interrupted.
//! A request that ends while waiting (a Bluetooth pairing timed out
//! behind a polkit prompt, say) is logged as a warning.
//!
//! Each owner has at most one prompt, shown or waiting. Releasing a
//! waiting one (its request was cancelled) takes it out of the line.
//!
//! A prompt keeps a [`PromptWindow`], implements [`Prompt`] and calls
//! [`install`]. The turn's comings and goings, and showing and hiding the
//! window with them, are then handled here, the keyboard focus let go
//! with the window; the prompt only draws, with [`emit`], and wipes what
//! was typed when the turn goes.

mod window;

use std::collections::VecDeque;
use std::marker::PhantomData;

use mecha_wayland::prelude::*;

pub use window::PromptWindow;

/// Who shows a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Polkit,
    Bluetooth,
}

/// It is `Owner`'s turn: show the prompt that waited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptTurn(pub Owner);
impl Signal for PromptTurn {}

/// A prompt that takes turns: its dialog's resource.
pub trait Prompt: Resource {
    const OWNER: Owner;
    /// The widget drawn in the window.
    type View: 'static;

    fn window(&self) -> &PromptWindow<Self::View>;
    fn window_mut(&mut self) -> &mut PromptWindow<Self::View>;

    /// Whether a request is kept to show.
    fn has_request(&self) -> bool;

    /// The turn came, and the window shows: draw the request kept, as it
    /// stands now, fields empty.
    fn showing(app: &mut App);

    /// The request closed, and the window hides: wipe what was typed. The
    /// keyboard focus, if in the window, is let go here; the view should
    /// not take it back without the turn.
    ///
    /// Called with the turn still held, so [`emit`] passes; what it emits
    /// runs after, the turn gone.
    fn hiding(app: &mut App);
}

/// Send `event` to `P`'s view, if `P` has the turn: a prompt waiting
/// draws nothing, and takes nothing.
pub fn emit<P: Prompt, E: Event>(app: &mut App, event: E) {
    let (turn, view) = {
        let w = app.resource::<P>().window();
        (w.has_turn(), w.view())
    };
    if turn {
        app.emit(event, view);
    }
}

/// Whether `P` has the turn: its window shows.
pub fn has_turn<P: Prompt>(app: &App) -> bool {
    app.resource::<P>().window().has_turn()
}

fn take_turn<P: Prompt>(app: &mut App) {
    let window = {
        let mut me = app.resource_mut::<P>();
        me.window_mut().set_turn(true);
        me.window().window()
    };
    window::set_hidden(app, window, false);
    P::showing(app);
}

fn give_up_turn<P: Prompt>(app: &mut App) {
    if !has_turn::<P>(app) {
        return;
    }
    P::hiding(app);
    let window = {
        let mut me = app.resource_mut::<P>();
        me.window_mut().set_turn(false);
        me.window().window()
    };
    window::set_hidden(app, window, true);
    window::let_keyboard_go(app, window);
}

/// The prompt shown, and those waiting, in order.
#[derive(Debug, Default)]
pub struct PromptQueue {
    shown: Option<Owner>,
    waiting: VecDeque<Owner>,
}
impl Resource for PromptQueue {}

impl PromptQueue {
    /// The prompt shown, if any.
    pub fn shown(&self) -> Option<Owner> {
        self.shown
    }

    /// Whether `owner` waits for its turn.
    pub fn is_waiting(&self, owner: Owner) -> bool {
        self.waiting.contains(&owner)
    }

    /// `owner` wants to show: true if it may now (nothing shown, or its
    /// own prompt already is), else it waits at the back of the line.
    fn request(&mut self, owner: Owner) -> bool {
        match self.shown {
            None => {
                self.shown = Some(owner);
                true
            }
            Some(shown) if shown == owner => true,
            Some(_) => {
                if !self.is_waiting(owner) {
                    self.waiting.push_back(owner);
                }
                false
            }
        }
    }

    /// `owner`'s prompt closed, or stopped waiting. The next in line, if
    /// it now shows.
    fn release(&mut self, owner: Owner) -> Option<Owner> {
        self.waiting.retain(|w| *w != owner);
        if self.shown != Some(owner) {
            return None;
        }
        self.shown = self.waiting.pop_front();
        self.shown
    }
}

/// `P`'s part in the queue is installed.
struct Installed<P>(PhantomData<fn() -> P>);
impl<P: 'static> Resource for Installed<P> {}

/// Install the queue, and `P`'s part in it. Again for the same `P`, a
/// no-op: its turns are handled once.
pub fn install<P: Prompt>(app: &mut App) {
    if !app.has_resource::<PromptQueue>() {
        app.init_resource::<PromptQueue>();
    }
    if app.has_resource::<Installed<P>>() {
        return;
    }
    app.insert_resource(Installed::<P>(PhantomData));
    app.system(on_turn::<P>);
}

/// Ask for `P`'s turn: shown now if it may be, else when it comes.
pub fn request<P: Prompt>(app: &mut App) {
    if request_owner(app, P::OWNER) {
        take_turn::<P>(app);
    }
}

/// `P`'s prompt closed, or its request ended while waiting: it hides,
/// and the next in line gets its turn.
pub fn release<P: Prompt>(app: &mut App) {
    give_up_turn::<P>(app);
    release_owner(app, P::OWNER);
}

/// [`request`] by owner alone, for what has no [`Prompt`] (a test's
/// stand-in): true to show now, false to wait for [`PromptTurn`].
pub fn request_owner(app: &mut App, owner: Owner) -> bool {
    let now = app.resource_mut::<PromptQueue>().request(owner);
    if !now {
        tracing::info!("prompt: {owner:?} waits for the prompt shown");
    }
    now
}

/// [`release`] by owner alone; hides nothing.
pub fn release_owner(app: &mut App, owner: Owner) {
    let mut queue = app.resource_mut::<PromptQueue>();
    if queue.is_waiting(owner) {
        // Most likely timed out behind the prompt shown.
        tracing::warn!("prompt: {owner:?}'s request ended unseen, while it waited");
    }
    let next = queue.release(owner);
    if let Some(next) = next {
        tracing::info!("prompt: {owner:?} → {next:?}");
        app.signal(PromptTurn(next));
    }
}

fn on_turn<P: Prompt>(app: &mut App, e: &PromptTurn) {
    // Stale, if the turn moved again since it was sent.
    if e.0 != P::OWNER || app.resource::<PromptQueue>().shown() != Some(P::OWNER) {
        return;
    }
    if app.resource::<P>().has_request() {
        take_turn::<P>(app);
    } else {
        // Closed while the turn was on its way: pass it on.
        release_owner(app, P::OWNER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Owner::*;

    #[test]
    fn the_first_shows_and_the_next_waits() {
        let mut q = PromptQueue::default();
        assert!(q.request(Polkit));
        assert!(!q.request(Bluetooth));
        assert_eq!(q.shown(), Some(Polkit));
        assert!(q.is_waiting(Bluetooth));
    }

    #[test]
    fn closing_hands_the_turn_on() {
        let mut q = PromptQueue::default();
        q.request(Bluetooth);
        q.request(Polkit);
        assert_eq!(q.release(Bluetooth), Some(Polkit));
        assert_eq!(q.shown(), Some(Polkit));
        assert!(!q.is_waiting(Polkit));
        assert_eq!(q.release(Polkit), None);
        assert_eq!(q.shown(), None);
    }

    #[test]
    fn the_one_shown_asking_again_still_shows() {
        let mut q = PromptQueue::default();
        q.request(Polkit);
        assert!(q.request(Polkit));
        assert!(!q.is_waiting(Polkit));
    }

    #[test]
    fn a_waiting_one_waits_once() {
        let mut q = PromptQueue::default();
        q.request(Bluetooth);
        q.request(Polkit);
        q.request(Polkit);
        assert_eq!(q.release(Bluetooth), Some(Polkit));
        assert_eq!(q.release(Polkit), None);
    }

    /// Cancelled while waiting: out of line, and nothing else moves.
    #[test]
    fn a_waiting_one_released_leaves_the_line() {
        let mut q = PromptQueue::default();
        q.request(Bluetooth);
        q.request(Polkit);
        assert_eq!(q.release(Polkit), None);
        assert_eq!(q.shown(), Some(Bluetooth));
        assert_eq!(q.release(Bluetooth), None);
        assert_eq!(q.shown(), None);
    }

    #[test]
    fn releasing_what_is_not_there_does_nothing() {
        let mut q = PromptQueue::default();
        assert_eq!(q.release(Polkit), None);
        q.request(Bluetooth);
        assert_eq!(q.release(Polkit), None);
        assert_eq!(q.shown(), Some(Bluetooth));
    }

    /// First come, first served, whoever asks: polkit shown, a Bluetooth
    /// pairing waits for it.
    #[test]
    fn no_one_goes_ahead() {
        let mut q = PromptQueue::default();
        q.request(Polkit);
        assert!(!q.request(Bluetooth));
        assert_eq!(q.shown(), Some(Polkit));
        assert_eq!(q.release(Polkit), Some(Bluetooth));
    }

    struct Leaf;
    impl Build for Leaf {
        type Widget = Leaf;
    }
    impl Widget for Leaf {
        type Builder = Leaf;
        fn build(b: Leaf, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
            s.on::<Drawn>(me, |ctx, e| {
                ctx.resource_mut::<Fake>().log.push(e.0);
            });
            b
        }
    }

    /// What the view was sent.
    struct Drawn(&'static str);
    impl Event for Drawn {}

    /// A prompt that records its turns, and what reached its view.
    struct Fake {
        window: PromptWindow<Leaf>,
        request: bool,
        log: Vec<&'static str>,
    }
    impl Resource for Fake {}
    impl Prompt for Fake {
        const OWNER: Owner = Polkit;
        type View = Leaf;
        fn window(&self) -> &PromptWindow<Leaf> {
            &self.window
        }
        fn window_mut(&mut self) -> &mut PromptWindow<Leaf> {
            &mut self.window
        }
        fn has_request(&self) -> bool {
            self.request
        }
        fn showing(app: &mut App) {
            emit::<Fake, _>(app, Drawn("take"));
        }
        fn hiding(app: &mut App) {
            emit::<Fake, _>(app, Drawn("give up"));
        }
    }

    fn fake_app() -> App {
        let mut app = App::new();
        let root = app.root();
        let window = app.spawn(root, Leaf).id();
        let view = app.spawn(window, Leaf);
        app.init_resource::<KeyboardFocus>();
        app.insert_resource(Fake {
            window: PromptWindow::from_parts(window, view),
            request: false,
            log: Vec::new(),
        });
        install::<Fake>(&mut app);
        app
    }

    fn view(app: &App) -> NodeId {
        app.resource::<Fake>().window.view().id()
    }

    fn focused(app: &App) -> Option<NodeId> {
        app.resource::<KeyboardFocus>().focused()
    }

    fn ask(app: &mut App) {
        app.resource_mut::<Fake>().request = true;
        request::<Fake>(app);
        app.flush();
    }

    fn log(app: &App) -> Vec<&'static str> {
        app.resource::<Fake>().log.clone()
    }

    #[test]
    fn a_prompt_waits_then_takes_its_turn() {
        let mut app = fake_app();
        assert!(request_owner(&mut app, Bluetooth));
        ask(&mut app);
        assert!(log(&app).is_empty());
        release_owner(&mut app, Bluetooth);
        app.flush();
        assert_eq!(log(&app), ["take"]);
    }

    /// Shown, it stays shown while another asks.
    #[test]
    fn a_prompt_shown_is_not_interrupted() {
        let mut app = fake_app();
        ask(&mut app);
        assert!(!request_owner(&mut app, Bluetooth));
        app.flush();
        assert_eq!(log(&app), ["take"]);
        assert!(has_turn::<Fake>(&app));
        release::<Fake>(&mut app);
        app.flush();
        assert_eq!(log(&app), ["take", "give up"]);
        assert_eq!(app.resource::<PromptQueue>().shown(), Some(Bluetooth));
    }

    /// Closed while its turn was on the way: the turn passes on.
    #[test]
    fn a_turn_for_no_request_is_passed_on() {
        let mut app = fake_app();
        assert!(request_owner(&mut app, Bluetooth));
        ask(&mut app);
        app.resource_mut::<Fake>().request = false;
        release_owner(&mut app, Bluetooth);
        // Polkit's turn, before Fake hears of it.
        assert_eq!(app.resource::<PromptQueue>().shown(), Some(Polkit));
        app.flush();
        assert!(log(&app).is_empty());
        assert_eq!(app.resource::<PromptQueue>().shown(), None);
    }

    /// Asked for again while a turn sent earlier is still on its way.
    #[test]
    fn a_stale_turn_does_nothing() {
        let mut app = fake_app();
        ask(&mut app);
        release::<Fake>(&mut app);
        app.signal(PromptTurn(Polkit));
        assert!(request_owner(&mut app, Bluetooth));
        app.flush();
        assert_eq!(log(&app), ["take", "give up"]);
    }

    /// Waiting, it draws nothing; shown, it does.
    #[test]
    fn only_the_turn_draws() {
        let mut app = fake_app();
        assert!(request_owner(&mut app, Bluetooth));
        ask(&mut app);
        emit::<Fake, _>(&mut app, Drawn("waiting"));
        app.flush();
        assert!(log(&app).is_empty());
        assert!(!has_turn::<Fake>(&app));
        release_owner(&mut app, Bluetooth);
        app.flush();
        emit::<Fake, _>(&mut app, Drawn("shown"));
        app.flush();
        assert!(has_turn::<Fake>(&app));
        assert_eq!(log(&app), ["take", "shown"]);
    }

    /// Installed twice, its turn is still taken once.
    #[test]
    fn installing_twice_handles_turns_once() {
        let mut app = fake_app();
        install::<Fake>(&mut app);
        assert!(request_owner(&mut app, Bluetooth));
        ask(&mut app);
        release_owner(&mut app, Bluetooth);
        app.flush();
        assert_eq!(log(&app), ["take"]);
    }

    /// Hidden, the window keeps no keyboard focus, whatever its view does.
    #[test]
    fn hiding_lets_the_keyboard_go() {
        let mut app = fake_app();
        ask(&mut app);
        let view = view(&app);
        app.resource_mut::<KeyboardFocus>().focus(view);
        release::<Fake>(&mut app);
        app.flush();
        assert_eq!(focused(&app), None);
    }

    /// Focus outside the window, another's, is left alone.
    #[test]
    fn hiding_leaves_others_focus_alone() {
        let mut app = fake_app();
        let root = app.root();
        let other = app.spawn(root, Leaf).id();
        ask(&mut app);
        app.resource_mut::<KeyboardFocus>().focus(other);
        release::<Fake>(&mut app);
        app.flush();
        assert_eq!(focused(&app), Some(other));
    }
}
