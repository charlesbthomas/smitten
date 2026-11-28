//! Weighted picks: loot tables, enemy waves, which line a shopkeeper says.
//!
//! The kit does not hold a random generator of its own. Each pick takes a
//! `roll` in `0.0..1.0` from whatever the game uses (smitten's
//! `RandomGenerator`, a seeded generator for replays, a fixed number in a
//! test), so the same rolls always give the same picks.
//!
//! ```
//! use smitten_kit_core::random::{Weighted, weighted_choice};
//!
//! let drops = [("coin", 8.0), ("potion", 1.5), ("sword", 0.5)];
//! assert_eq!(weighted_choice(&drops, |d| d.1, 0.5).unwrap().0, "coin");
//! assert_eq!(weighted_choice(&drops, |d| d.1, 0.97).unwrap().0, "sword");
//!
//! // A table built once and picked from many times.
//! let table = Weighted::new(drops.iter().map(|&(name, weight)| (name, weight)));
//! assert_eq!(table.pick(0.85), Some(&"potion"));
//! ```

/// The index `roll` lands on when each index takes up its share of
/// `0.0..1.0` by weight. Weights at or below zero (and NaN) are never
/// picked. `None` when no weight is above zero.
pub fn weighted_index(weights: impl IntoIterator<Item = f32> + Clone, roll: f32) -> Option<usize> {
    let usable = |w: f32| if w > 0.0 { w } else { 0.0 };
    let total: f32 = weights.clone().into_iter().map(usable).sum();
    if total <= 0.0 || !total.is_finite() {
        return None;
    }
    let mut target = roll.clamp(0.0, 1.0) * total;
    let mut last = None;
    for (index, weight) in weights.into_iter().enumerate() {
        let weight = usable(weight);
        if weight <= 0.0 {
            continue;
        }
        if target < weight {
            return Some(index);
        }
        target -= weight;
        last = Some(index);
    }
    // A roll of 1.0, or rounding at the very end, lands on the last item.
    last
}

/// The item `roll` lands on, each taking up its share of `0.0..1.0` by
/// `weight`. See [`weighted_index`].
pub fn weighted_choice<T>(items: &[T], weight: impl Fn(&T) -> f32, roll: f32) -> Option<&T> {
    weighted_index(items.iter().map(&weight), roll).map(|index| &items[index])
}

/// A table of weighted items, summed once so each pick is a binary
/// search.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Weighted<T> {
    items: Vec<T>,
    /// The running total of the weights up to and including each item.
    ends: Vec<f32>,
}

impl<T> Weighted<T> {
    /// A table of `(item, weight)` pairs. Items weighing nothing are
    /// dropped.
    pub fn new(entries: impl IntoIterator<Item = (T, f32)>) -> Self {
        let mut table = Weighted {
            items: Vec::new(),
            ends: Vec::new(),
        };
        for (item, weight) in entries {
            table.push(item, weight);
        }
        table
    }

    /// Adds an item. One weighing nothing is dropped.
    pub fn push(&mut self, item: T, weight: f32) {
        if weight > 0.0 && weight.is_finite() {
            let total = self.total();
            self.items.push(item);
            self.ends.push(total + weight);
        }
    }

    /// The sum of the weights.
    pub fn total(&self) -> f32 {
        self.ends.last().copied().unwrap_or(0.0)
    }

    /// How many items it holds.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether it holds nothing to pick.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The item `roll` (in `0.0..1.0`) lands on, or `None` when empty.
    pub fn pick(&self, roll: f32) -> Option<&T> {
        let target = roll.clamp(0.0, 1.0) * self.total();
        let index = self.ends.partition_point(|&end| end <= target);
        self.items.get(index.min(self.items.len().checked_sub(1)?))
    }

    /// Each item with the chance (`0.0..=1.0`) of picking it.
    pub fn chances(&self) -> impl Iterator<Item = (&T, f32)> {
        let total = self.total();
        let mut start = 0.0;
        self.items.iter().zip(&self.ends).map(move |(item, &end)| {
            let chance = (end - start) / total;
            start = end;
            (item, chance)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolls_split_by_weight() {
        let weights = [1.0, 0.0, 3.0];
        assert_eq!(weighted_index(weights, 0.0), Some(0));
        assert_eq!(weighted_index(weights, 0.24), Some(0));
        assert_eq!(weighted_index(weights, 0.25), Some(2));
        assert_eq!(weighted_index(weights, 1.0), Some(2));
        assert_eq!(weighted_index([0.0, -1.0], 0.5), None);
        assert_eq!(weighted_index([f32::NAN, 1.0], 0.0), Some(1));
    }

    #[test]
    fn table_matches_the_function() {
        let weights = [2.0, 5.0, 0.5, 2.5];
        let table = Weighted::new(weights.iter().copied().enumerate());
        for step in 0..=100 {
            let roll = step as f32 / 100.0;
            assert_eq!(
                table.pick(roll).copied(),
                weighted_index(weights, roll),
                "roll {roll}"
            );
        }
        let chances: Vec<f32> = table.chances().map(|(_, c)| c).collect();
        assert_eq!(chances, [0.2, 0.5, 0.05, 0.25]);
        assert_eq!(Weighted::<u8>::default().pick(0.5), None);
    }
}
