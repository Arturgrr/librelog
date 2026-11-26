//! Holds on a local model: a recording that will be transcribed when it
//! stops keeps the model loaded from its start, so the transcription doesn't
//! wait for a load the idle unload made necessary. Each hold ends when it is
//! released, or by itself after its time, so a lost release never pins the
//! model.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::SoniqoModel;

/// The longest a hold lasts: an annotation's 20-minute cap, and preparing it.
pub const MAX_HOLD: Duration = Duration::from_secs(30 * 60);

/// What a hold keeps loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Held {
    /// The internal server's transcribe.cpp model.
    #[cfg_attr(not(feature = "gguf"), allow(dead_code))]
    Internal,
    Soniqo(SoniqoModel),
}

#[derive(Default)]
pub(crate) struct Holds {
    next: u64,
    leases: HashMap<String, (Held, Instant)>,
}

impl Holds {
    /// A new hold on `held` for `hold`, at most [`MAX_HOLD`]: its id, and how
    /// long `held` is to stay loaded.
    pub fn add(&mut self, held: Held, hold: Duration, now: Instant) -> (String, Duration) {
        self.prune(now);
        self.next += 1;
        let id = format!("hold-{}", self.next);
        self.leases
            .insert(id.clone(), (held, now + hold.min(MAX_HOLD)));
        (id, self.remaining(held, now).unwrap_or_default())
    }

    /// Ends hold `id`: what it held, and how long the other holds keep that
    /// loaded (`None`: it may go idle).
    pub fn remove(&mut self, id: &str, now: Instant) -> Option<(Held, Option<Duration>)> {
        let (held, _) = self.leases.remove(id)?;
        self.prune(now);
        Some((held, self.remaining(held, now)))
    }

    fn remaining(&self, held: Held, now: Instant) -> Option<Duration> {
        self.leases
            .values()
            .filter(|(other, _)| *other == held)
            .map(|(_, until)| until.saturating_duration_since(now))
            .max()
    }

    fn prune(&mut self, now: Instant) {
        self.leases.retain(|_, (_, until)| *until > now);
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
        assert_eq!(kept, minute);
        let (annotation, kept) = holds.add(Held::Internal, Duration::from_secs(3600), now);
        assert_eq!(kept, MAX_HOLD);

        assert_eq!(
            holds.remove(&dictation, now),
            Some((Held::Internal, Some(MAX_HOLD)))
        );
        assert_eq!(holds.remove(&dictation, now), None);
        assert_eq!(holds.remove(&annotation, now), Some((Held::Internal, None)));
    }
}
