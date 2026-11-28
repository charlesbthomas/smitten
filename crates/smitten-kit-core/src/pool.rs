//! [`Pool`]: a store of things that come and go (bullets, enemies,
//! particles, pickups) named by [`Handle`]s that know when what they named
//! is gone.
//!
//! A removed slot is reused, so a storm of bullets stops allocating once
//! the pool has grown to fit it. Each slot counts how often it has been
//! reused (its generation), and a handle remembers the generation it was
//! made for, so a homing missile holding the handle of an enemy that died
//! gets `None` instead of whichever enemy took the slot next.
//!
//! ```
//! use smitten_kit_core::pool::Pool;
//!
//! let mut enemies = Pool::new();
//! let grunt = enemies.insert("grunt");
//! let boss = enemies.insert("boss");
//!
//! enemies.remove(grunt);
//! let imp = enemies.insert("imp"); // reuses the grunt's slot
//! assert_eq!(enemies.get(grunt), None); // the old handle stays dead
//! assert_eq!(enemies.get(imp), Some(&"imp"));
//! assert_eq!(enemies.len(), 2);
//!
//! enemies.retain(|_, name| *name != "boss");
//! assert!(!enemies.contains(boss));
//! ```
//!
//! For a game with many kinds of things that share parts (position,
//! health, sprite) an ECS such as [hecs] fits better: its `World` is a
//! pool whose entries are bundles of parts, queried by the parts they
//! have. A `Pool` per kind of thing is simpler while each kind is its own
//! struct.
//!
//! [hecs]: https://docs.rs/hecs

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// Names one value in a [`Pool<T>`]. Copy it freely; it stays small and
/// never keeps the value alive.
pub struct Handle<T> {
    index: u32,
    generation: u32,
    kind: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    /// The slot the value sits in, for a side table indexed by slot.
    pub fn index(self) -> usize {
        self.index as usize
    }

    /// How many values used the slot before this one.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

// Written out because deriving them would ask `T` for each one.
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Handle<T> {}
impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}
impl<T> Eq for Handle<T> {}
impl<T> PartialOrd for Handle<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Handle<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.index, self.generation).cmp(&(other.index, other.generation))
    }
}
impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}
impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle({}v{})", self.index, self.generation)
    }
}

#[derive(Clone, Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// Values named by [`Handle`]s, with removed slots reused. See the
/// [module docs](self).
#[derive(Clone, Debug)]
pub struct Pool<T> {
    slots: Vec<Slot<T>>,
    /// Empty slots, the most recently emptied last.
    free: Vec<u32>,
    len: usize,
    limit: usize,
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Pool {
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
            limit: usize::MAX,
        }
    }
}

impl<T> Pool<T> {
    /// An empty pool with no limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty pool that holds at most `limit` values, with room for all
    /// of them allocated now. [`try_insert`](Pool::try_insert) turns away
    /// values past the limit, as a bullet-hell game drops shots past its
    /// bullet budget.
    pub fn with_limit(limit: usize) -> Self {
        Pool {
            slots: Vec::with_capacity(limit),
            free: Vec::new(),
            len: 0,
            limit,
        }
    }

    /// The most values it holds, `usize::MAX` without a limit.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Whether it holds as many values as its limit allows.
    pub fn is_full(&self) -> bool {
        self.len >= self.limit
    }

    /// Adds `value` and returns its handle, or gives `value` back when the
    /// pool is full.
    pub fn try_insert(&mut self, value: T) -> Result<Handle<T>, T> {
        if self.is_full() {
            return Err(value);
        }
        self.len += 1;
        let index = match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.value = Some(value);
                index
            }
            None => {
                let index = u32::try_from(self.slots.len()).expect("a pool holds under 2^32 slots");
                self.slots.push(Slot {
                    generation: 0,
                    value: Some(value),
                });
                index
            }
        };
        Ok(Handle {
            index,
            generation: self.slots[index as usize].generation,
            kind: PhantomData,
        })
    }

    /// Adds `value` and returns its handle.
    ///
    /// # Panics
    ///
    /// When the pool is full. Use [`try_insert`](Pool::try_insert) on a
    /// pool made [`with_limit`](Pool::with_limit).
    pub fn insert(&mut self, value: T) -> Handle<T> {
        match self.try_insert(value) {
            Ok(handle) => handle,
            Err(_) => panic!("the pool is full ({} values)", self.limit),
        }
    }

    fn slot(&self, handle: Handle<T>) -> Option<&Slot<T>> {
        self.slots
            .get(handle.index as usize)
            .filter(|slot| slot.generation == handle.generation)
    }

    /// The value, or `None` once it has been removed.
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        self.slot(handle)?.value.as_ref()
    }

    /// The value to change, or `None` once it has been removed.
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.slots
            .get_mut(handle.index as usize)
            .filter(|slot| slot.generation == handle.generation)?
            .value
            .as_mut()
    }

    /// Two different values to change at once, as a collision between
    /// two enemies needs. `None` if either is gone or they are the same.
    pub fn get_pair_mut(&mut self, a: Handle<T>, b: Handle<T>) -> Option<(&mut T, &mut T)> {
        if a.index == b.index {
            return None;
        }
        self.get(a)?;
        self.get(b)?;
        let (low, high) = if a.index < b.index { (a, b) } else { (b, a) };
        let (left, right) = self.slots.split_at_mut(high.index as usize);
        let low_value = left[low.index as usize].value.as_mut()?;
        let high_value = right[0].value.as_mut()?;
        Some(if a.index < b.index {
            (low_value, high_value)
        } else {
            (high_value, low_value)
        })
    }

    /// Whether the value is still in the pool.
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.get(handle).is_some()
    }

    /// Takes the value out. `None` if it was already gone.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let slot = self
            .slots
            .get_mut(handle.index as usize)
            .filter(|slot| slot.generation == handle.generation)?;
        let value = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(handle.index);
        self.len -= 1;
        Some(value)
    }

    /// How many values it holds.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Removes everything. Handles from before stay dead.
    pub fn clear(&mut self) {
        self.retain(|_, _| false);
    }

    /// Keeps only the values `keep` returns true for, visiting each once.
    /// The usual way to update everything and drop what died:
    ///
    /// ```
    /// # use smitten_kit_core::pool::Pool;
    /// let mut lives = Pool::new();
    /// lives.insert(0.5_f32);
    /// lives.insert(2.0);
    /// lives.retain(|_, life| {
    ///     *life -= 1.0;
    ///     *life > 0.0
    /// });
    /// assert_eq!(lives.len(), 1);
    /// ```
    pub fn retain(&mut self, mut keep: impl FnMut(Handle<T>, &mut T) -> bool) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some(value) = &mut slot.value else {
                continue;
            };
            let handle = Handle {
                index: index as u32,
                generation: slot.generation,
                kind: PhantomData,
            };
            if !keep(handle, value) {
                slot.value = None;
                slot.generation = slot.generation.wrapping_add(1);
                self.free.push(index as u32);
                self.len -= 1;
            }
        }
    }

    /// Every value with its handle, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            let value = slot.value.as_ref()?;
            Some((
                Handle {
                    index: index as u32,
                    generation: slot.generation,
                    kind: PhantomData,
                },
                value,
            ))
        })
    }

    /// Every value to change, with its handle, in slot order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle<T>, &mut T)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| {
                let generation = slot.generation;
                let value = slot.value.as_mut()?;
                Some((
                    Handle {
                        index: index as u32,
                        generation,
                        kind: PhantomData,
                    },
                    value,
                ))
            })
    }

    /// Every value, in slot order.
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|slot| slot.value.as_ref())
    }

    /// Every value to change, in slot order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().filter_map(|slot| slot.value.as_mut())
    }

    /// Every handle, in slot order. Collect them to loop while changing
    /// the pool.
    pub fn handles(&self) -> impl Iterator<Item = Handle<T>> + '_ {
        self.iter().map(|(handle, _)| handle)
    }
}

impl<T> std::ops::Index<Handle<T>> for Pool<T> {
    type Output = T;

    /// # Panics
    ///
    /// When the value is gone. Use [`get`](Pool::get) where it can be.
    fn index(&self, handle: Handle<T>) -> &T {
        match self.get(handle) {
            Some(value) => value,
            None => panic!("{handle:?} is no longer in the pool"),
        }
    }
}

impl<T> std::ops::IndexMut<Handle<T>> for Pool<T> {
    fn index_mut(&mut self, handle: Handle<T>) -> &mut T {
        match self.get_mut(handle) {
            Some(value) => value,
            None => panic!("{handle:?} is no longer in the pool"),
        }
    }
}

impl<T> FromIterator<T> for Pool<T> {
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        let mut pool = Pool::new();
        for value in values {
            pool.insert(value);
        }
        pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_handles_stay_dead() {
        let mut pool = Pool::new();
        let a = pool.insert(1);
        assert_eq!(pool.remove(a), Some(1));
        assert_eq!(pool.remove(a), None);
        let b = pool.insert(2);
        assert_eq!(a.index(), b.index());
        assert_ne!(a, b);
        assert_eq!(pool.get(a), None);
        assert_eq!(pool[b], 2);
    }

    #[test]
    fn limit_turns_values_away() {
        let mut bullets = Pool::with_limit(2);
        assert!(bullets.try_insert('a').is_ok());
        let b = bullets.try_insert('b').unwrap();
        assert_eq!(bullets.try_insert('c'), Err('c'));
        assert!(bullets.is_full());
        bullets.remove(b);
        assert!(bullets.try_insert('d').is_ok());
        assert_eq!(bullets.len(), 2);
    }

    #[test]
    fn retain_and_clear() {
        let mut pool: Pool<i32> = (0..10).collect();
        pool.retain(|_, n| *n % 2 == 0);
        assert_eq!(pool.len(), 5);
        assert_eq!(pool.values().copied().collect::<Vec<_>>(), [0, 2, 4, 6, 8]);
        let first = pool.handles().next().unwrap();
        pool.clear();
        assert!(pool.is_empty());
        assert!(!pool.contains(first));
        // The cleared slots are reused.
        pool.insert(1);
        assert_eq!(pool.iter().next().unwrap().0.generation(), 1);
    }

    #[test]
    fn pair_mut() {
        let mut pool = Pool::new();
        let a = pool.insert(1);
        let b = pool.insert(2);
        let (x, y) = pool.get_pair_mut(b, a).unwrap();
        std::mem::swap(x, y);
        assert_eq!((pool[a], pool[b]), (2, 1));
        assert!(pool.get_pair_mut(a, a).is_none());
    }
}
