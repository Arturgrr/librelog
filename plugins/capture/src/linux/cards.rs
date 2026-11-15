//! The cards in the screen's corner on Linux: one small window shows the
//! stack (newest nearest the corner, at most four), each card leaves after
//! eight seconds unless the pointer is over it, and its buttons are queued
//! as the same `card_action` and `card_closed` events the macOS layer
//! posts, which the plugin's event poll drains.

use std::time::{Duration, Instant};

use crate::types::{CaptureCard, CaptureCards, CardLabels};

use super::Events;

pub const MAX_CARDS: usize = 4;
pub const LIFETIME: Duration = Duration::from_secs(8);
const ACTIONS: [&str; 5] = ["copy", "save", "edit", "open", "close"];

/// Shows the stack; an empty one hides the window.
pub trait CardView: Send + Sync {
    fn show(&self, cards: &CaptureCards);
    /// Returns once no card is on the screen any more (bounded), for a
    /// screenshot that must not show one.
    fn settle(&self) {}
}

struct Up {
    card: CaptureCard,
    /// When it was shown, or when the pointer last left it.
    since: Instant,
    hovered: bool,
}

#[derive(Default)]
struct Stack {
    /// Oldest first.
    up: Vec<Up>,
    labels: CardLabels,
}

pub struct Cards {
    stack: std::sync::Mutex<Stack>,
    view: Box<dyn CardView>,
    events: std::sync::Arc<Events>,
}

fn closed(id: &str) -> serde_json::Value {
    serde_json::json!({ "type": "card_closed", "id": id })
}

impl Cards {
    pub fn new(view: Box<dyn CardView>, events: std::sync::Arc<Events>) -> Self {
        Self {
            stack: Default::default(),
            view,
            events,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Stack> {
        self.stack.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn current(&self) -> CaptureCards {
        let stack = self.lock();
        CaptureCards {
            cards: stack.up.iter().rev().map(|up| up.card.clone()).collect(),
            labels: stack.labels.clone(),
        }
    }

    fn render(&self) {
        self.view.show(&self.current());
    }

    /// Shows `card` nearest the corner; the oldest past four leaves.
    pub fn show(&self, card: CaptureCard, labels: CardLabels, now: Instant) {
        {
            let mut stack = self.lock();
            stack.labels = labels;
            let mut left = Vec::new();
            if let Some(index) = stack.up.iter().position(|up| up.card.id == card.id) {
                stack.up.remove(index);
                // The same card again: no `card_closed` for it.
            }
            stack.up.push(Up {
                card,
                since: now,
                hovered: false,
            });
            while stack.up.len() > MAX_CARDS {
                left.push(stack.up.remove(0).card.id);
            }
            for id in left {
                self.events.push(closed(&id));
            }
        }
        self.render();
    }

    /// Closes one card, or all of them when `id` is `None`; whether any was
    /// up.
    pub fn close(&self, id: Option<&str>) -> bool {
        let gone: Vec<String> = {
            let mut stack = self.lock();
            let (gone, kept) = std::mem::take(&mut stack.up)
                .into_iter()
                .partition::<Vec<_>, _>(|up| id.is_none_or(|id| up.card.id == id));
            stack.up = kept;
            gone.into_iter().map(|up| up.card.id).collect()
        };
        if gone.is_empty() {
            return false;
        }
        for id in &gone {
            self.events.push(closed(id));
        }
        self.render();
        true
    }

    /// Closes every card and waits (briefly) until none is on the screen.
    pub fn clear_screen(&self) {
        if self.close(None) {
            self.view.settle();
        }
    }

    /// Lets the cards whose time is up leave.
    pub fn expire(&self, now: Instant) {
        let gone: Vec<String> = {
            let stack = self.lock();
            stack
                .up
                .iter()
                .filter(|up| !up.hovered && now.saturating_duration_since(up.since) >= LIFETIME)
                .map(|up| up.card.id.clone())
                .collect()
        };
        for id in gone {
            let _ = self.close(Some(&id));
        }
    }

    pub fn hover(&self, id: &str, hovered: bool, now: Instant) {
        let mut stack = self.lock();
        if let Some(up) = stack.up.iter_mut().find(|up| up.card.id == id) {
            up.hovered = hovered;
            up.since = now;
        }
    }

    /// A button on a card that is up: queued for the plugin.
    pub fn press(&self, id: &str, action: &str) -> bool {
        if !ACTIONS.contains(&action) || !self.lock().up.iter().any(|up| up.card.id == id) {
            return false;
        }
        self.events.push(serde_json::json!({
            "type": "card_action", "id": id, "action": action,
        }));
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct View(Arc<Mutex<Vec<Vec<String>>>>);

    impl CardView for View {
        fn show(&self, cards: &CaptureCards) {
            self.0
                .lock()
                .unwrap()
                .push(cards.cards.iter().map(|card| card.id.clone()).collect());
        }
    }

    fn card(id: &str) -> CaptureCard {
        CaptureCard {
            id: id.into(),
            kind: "screenshot".into(),
            thumbnail_path: String::new(),
            saved: false,
            duration_ms: 0,
        }
    }

    fn drain(events: &Events) -> Vec<String> {
        events
            .drain()
            .into_iter()
            .map(|event| {
                format!(
                    "{} {} {}",
                    event["type"].as_str().unwrap(),
                    event["id"].as_str().unwrap(),
                    event["action"].as_str().unwrap_or("")
                )
                .trim()
                .to_owned()
            })
            .collect()
    }

    #[test]
    fn card_buttons_and_departures_are_queued_as_the_macos_layers_events() {
        let events = Arc::new(Events::default());
        let shown = Arc::new(Mutex::new(Vec::new()));
        let cards = Cards::new(Box::new(View(shown.clone())), events.clone());
        let start = Instant::now();
        for (n, id) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            cards.show(
                card(id),
                CardLabels::default(),
                start + Duration::from_secs(n as u64),
            );
        }
        // Newest first; the fifth pushed the oldest out.
        assert_eq!(shown.lock().unwrap().last().unwrap(), &["e", "d", "c", "b"]);
        assert_eq!(drain(&events), ["card_closed a"]);

        assert!(cards.press("c", "copy"));
        assert!(!cards.press("a", "edit"), "a card that left");
        assert!(!cards.press("c", "drop-table"));
        assert_eq!(drain(&events), ["card_action c copy"]);

        // Eight seconds after each was shown, unless the pointer is on it.
        cards.hover("b", true, start + Duration::from_secs(2));
        cards.expire(start + Duration::from_secs(10));
        assert_eq!(drain(&events), ["card_closed c"]);
        cards.hover("b", false, start + Duration::from_secs(20));
        cards.expire(start + Duration::from_secs(21));
        assert_eq!(drain(&events), ["card_closed d", "card_closed e"]);
        cards.expire(start + Duration::from_secs(28));
        assert_eq!(drain(&events), ["card_closed b"]);
        assert_eq!(shown.lock().unwrap().last().unwrap(), &Vec::<String>::new());

        cards.show(card("f"), CardLabels::default(), start);
        assert!(cards.close(None));
        assert_eq!(drain(&events), ["card_closed f"]);
    }
}
