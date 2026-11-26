//! Holds on a local model: a recording that will be transcribed when it
//! stops keeps the model loaded from its start, so the transcription doesn't
//! wait for a load the idle unload made necessary. Each hold ends when it is
//! released, or by itself after its time, so a lost release never pins the
//! model.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::SoniqoModel;

/// The longest a hold lasts: an annotation's 20-minute cap, and preparing it.
pub const MAX_HOLD: Duration = Duration::from_secs(30 * 60);

/// What a hold keeps loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Held {
    /// The internal server's transcribe.cpp model.
    #[cfg_attr(not(feature = "gguf"), allow(dead_code))]
    Internal,
    Soniqo(SoniqoModel),
}

/// What the engine of `held` is told when its holds change: keep it loaded
/// until `until`, or let it go idle (`None`). Updates are delivered to the
/// engines separately and may arrive out of order, so they are numbered in
/// the order the holds changed, and an engine ignores one older than the last
/// it applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoldUpdate {
    pub held: Held,
    pub generation: u64,
    pub until: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct Holds {
    next: u64,
    generation: u64,
    leases: HashMap<String, (Held, Instant)>,
}

impl Holds {
    /// A new hold on `held` for `hold`, at most [`MAX_HOLD`]: its id, and the
    /// update for `held`'s engine.
    pub fn add(&mut self, held: Held, hold: Duration, now: Instant) -> (String, HoldUpdate) {
        self.prune(now);
        self.next += 1;
        let id = format!("hold-{}", self.next);
        self.leases
            .insert(id.clone(), (held, now + hold.min(MAX_HOLD)));
        (id, self.update(held))
    }

    /// Ends hold `id`: the update for the engine of what it held, kept loaded
    /// until the other holds on it end.
    pub fn remove(&mut self, id: &str, now: Instant) -> Option<HoldUpdate> {
        let (held, _) = self.leases.remove(id)?;
        self.prune(now);
        Some(self.update(held))
    }

    fn update(&mut self, held: Held) -> HoldUpdate {
        self.generation += 1;
        HoldUpdate {
            held,
            generation: self.generation,
            until: self
                .leases
                .values()
                .filter(|(other, _)| *other == held)
                .map(|(_, until)| *until)
                .max(),
        }
    }

    fn prune(&mut self, now: Instant) {
        self.leases.retain(|_, (_, until)| *until > now);
    }
}

/// Applies each engine's hold updates one at a time, newest only: one that
/// arrives after a newer one was applied is dropped.
#[derive(Default)]
pub(crate) struct Applied(Mutex<HashMap<Held, u64>>);

impl Applied {
    /// Runs `apply` with how long `update.held` is to stay loaded from now,
    /// unless a newer update for it was applied; whether it ran.
    pub fn apply(&self, update: HoldUpdate, apply: impl FnOnce(Option<Duration>)) -> bool {
        let mut applied = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let last = applied.entry(update.held).or_default();
        if *last >= update.generation {
            return false;
        }
        *last = update.generation;
        let now = Instant::now();
        apply(
            update
                .until
                .filter(|until| *until > now)
                .map(|until| until - now),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releasing_one_hold_keeps_the_model_for_another_until_it_ends() {
        let now = Instant::now();
        let mut holds = Holds::default();
        let minute = Duration::from_secs(60);
        let (dictation, kept) = holds.add(Held::Internal, minute, now);
        assert_eq!(kept.until, Some(now + minute));
        let (annotation, kept) = holds.add(Held::Internal, Duration::from_secs(3600), now);
        assert_eq!(kept.until, Some(now + MAX_HOLD));

        let released = holds.remove(&dictation, now).unwrap();
        assert_eq!(
            (released.held, released.until),
            (Held::Internal, Some(now + MAX_HOLD))
        );
        assert_eq!(holds.remove(&dictation, now), None);
        assert_eq!(holds.remove(&annotation, now).unwrap().until, None);
    }

    #[test]
    fn a_hold_delivered_after_its_release_does_not_pin_the_model() {
        let now = Instant::now();
        let mut holds = Holds::default();
        let (id, hold) = holds.add(Held::Internal, MAX_HOLD, now);
        let release = holds.remove(&id, now).unwrap();

        let applied = Applied::default();
        let mut kept = Vec::new();
        assert!(applied.apply(release, |hold| kept.push(hold)));
        assert!(!applied.apply(hold, |hold| kept.push(hold)));
        assert_eq!(kept, [None]);
    }
}
