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
//! Each owner has at most one prompt, shown or waiting. Releasing a
//! waiting one (its request was cancelled) takes it out of the line.

use std::collections::VecDeque;

use mecha_wayland::prelude::*;

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
    /// own prompt already is), else it waits in line.
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

/// Install the queue; safe to call from each prompt's install.
pub fn install(app: &mut App) {
    if !app.has_resource::<PromptQueue>() {
        app.init_resource::<PromptQueue>();
    }
}

/// Ask for `owner`'s turn: true to show now, false to wait for
/// [`PromptTurn`].
pub fn request(app: &mut App, owner: Owner) -> bool {
    let now = app.resource_mut::<PromptQueue>().request(owner);
    if !now {
        tracing::info!("prompt: {owner:?} waits for the prompt shown");
    }
    now
}

/// `owner`'s prompt closed, or its request ended while waiting: the next
/// in line gets its turn.
pub fn release(app: &mut App, owner: Owner) {
    if let Some(next) = app.resource_mut::<PromptQueue>().release(owner) {
        app.signal(PromptTurn(next));
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
        q.request(Polkit);
        q.request(Bluetooth);
        q.request(Bluetooth);
        assert_eq!(q.release(Polkit), Some(Bluetooth));
        assert_eq!(q.release(Bluetooth), None);
    }

    /// Cancelled while waiting: out of line, and nothing else moves.
    #[test]
    fn a_waiting_one_released_leaves_the_line() {
        let mut q = PromptQueue::default();
        q.request(Polkit);
        q.request(Bluetooth);
        assert_eq!(q.release(Bluetooth), None);
        assert_eq!(q.shown(), Some(Polkit));
        assert_eq!(q.release(Polkit), None);
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

    #[test]
    fn the_turn_is_signalled() {
        let mut app = App::new();
        install(&mut app);
        #[derive(Default)]
        struct Turns(Vec<Owner>);
        impl Resource for Turns {}
        app.init_resource::<Turns>();
        app.system(|app: &mut App, e: &PromptTurn| {
            app.resource_mut::<Turns>().0.push(e.0);
        });
        assert!(request(&mut app, Bluetooth));
        assert!(!request(&mut app, Polkit));
        release(&mut app, Bluetooth);
        app.flush();
        assert_eq!(app.resource::<Turns>().0, vec![Polkit]);
    }
}
