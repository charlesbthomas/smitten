//! Channels for passing values between threads, the equivalent of
//! `love.thread`'s `Channel`.
//!
//! Start threads with [`std::thread::spawn`], which does the job of
//! `love.thread.newThread`. Rust's own [`std::sync::mpsc`] channels are
//! often all a game needs. [`Channel`] is here for code ported from LÖVE:
//! it has LÖVE's methods, any thread can both send and receive on it,
//! senders can wait for their message to be read, and [`channel`] finds a
//! channel by name, like `love.thread.getChannel`.
//!
//! Remember that the framework modules, such as [`graphics`](crate::graphics),
//! only work on the main thread.
//!
//! ```
//! use smitten::thread::Channel;
//!
//! let jobs = Channel::new();
//! let results = Channel::new();
//! let worker = {
//!     let (jobs, results) = (jobs.clone(), results.clone());
//!     std::thread::spawn(move || {
//!         while let Some(n) = jobs.demand(None) {
//!             if n < 0 {
//!                 break;
//!             }
//!             results.push(n * n);
//!         }
//!     })
//! };
//! jobs.push(3);
//! jobs.push(-1);
//! worker.join().unwrap();
//! assert_eq!(results.pop(), Some(9));
//! ```

use std::any::{Any, TypeId};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// A queue of values that any number of threads can push to and pop
/// from, the equivalent of LÖVE's `Channel`.
///
/// Cloning a channel gives another handle to the same queue, to move into
/// another thread.
pub struct Channel<T> {
    shared: Arc<Shared<T>>,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    /// Signalled whenever a value is pushed or popped, or the queue is
    /// cleared.
    changed: Condvar,
}

struct State<T> {
    queue: VecDeque<T>,
    /// How many values have ever been pushed, which is the id of the last.
    pushed: u64,
    /// How many values have been taken off, by popping or clearing. Values
    /// leave in order, so every id up to this one has been read.
    read: u64,
}

impl<T> Clone for Channel<T> {
    fn clone(&self) -> Self {
        Channel {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Default for Channel<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> std::fmt::Debug for Channel<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Channel")
            .field("count", &self.count())
            .finish()
    }
}

/// Waits on `changed` until `done` holds, or until `timeout` has passed.
/// Gives back the lock and whether `done` holds.
fn wait_until<'a, T>(
    changed: &Condvar,
    mut state: MutexGuard<'a, State<T>>,
    timeout: Option<Duration>,
    done: impl Fn(&State<T>) -> bool,
) -> (MutexGuard<'a, State<T>>, bool) {
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        if done(&state) {
            return (state, true);
        }
        state = match deadline {
            None => changed
                .wait(state)
                .unwrap_or_else(|poison| poison.into_inner()),
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return (state, false);
                }
                changed
                    .wait_timeout(state, left)
                    .unwrap_or_else(|poison| poison.into_inner())
                    .0
            }
        };
    }
}

impl<T> Channel<T> {
    /// An empty channel, like `love.thread.newChannel`.
    pub fn new() -> Channel<T> {
        Channel {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    queue: VecDeque::new(),
                    pushed: 0,
                    read: 0,
                }),
                changed: Condvar::new(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<T>> {
        // A thread that panicked while holding the lock cannot have left
        // the queue half changed, so carry on.
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Adds `value` to the end of the queue and returns at once. The
    /// returned id can be given to [`has_read`](Self::has_read).
    pub fn push(&self, value: T) -> u64 {
        let id = self.lock().push(value);
        self.shared.changed.notify_all();
        id
    }

    /// Adds `value` to the end of the queue, then waits until it has been
    /// popped, like `Channel:supply`. With a `timeout`, gives up after it
    /// and returns `false`; the value then stays in the queue.
    pub fn supply(&self, value: T, timeout: Option<Duration>) -> bool {
        let mut state = self.lock();
        let id = state.push(value);
        self.shared.changed.notify_all();
        wait_until(&self.shared.changed, state, timeout, |state| {
            state.read >= id
        })
        .1
    }

    /// Takes the value at the front of the queue, or `None` if it is empty.
    /// Does not wait.
    pub fn pop(&self) -> Option<T> {
        let value = self.lock().pop();
        if value.is_some() {
            self.shared.changed.notify_all();
        }
        value
    }

    /// Takes the value at the front of the queue, waiting for one if the
    /// queue is empty, like `Channel:demand`. With a `timeout`, gives up
    /// after it and returns `None`.
    pub fn demand(&self, timeout: Option<Duration>) -> Option<T> {
        let state = self.lock();
        let (mut state, ready) = wait_until(&self.shared.changed, state, timeout, |state| {
            !state.queue.is_empty()
        });
        let value = if ready { state.pop() } else { None };
        drop(state);
        if value.is_some() {
            self.shared.changed.notify_all();
        }
        value
    }

    /// How many values are waiting in the queue.
    pub fn count(&self) -> usize {
        self.lock().queue.len()
    }

    /// Whether the value with this id, from [`push`](Self::push), has been
    /// popped or cleared away.
    pub fn has_read(&self, id: u64) -> bool {
        self.lock().read >= id
    }

    /// Empties the queue. Threads waiting in [`supply`](Self::supply) stop
    /// waiting, as if their values had been read.
    pub fn clear(&self) {
        self.lock().clear();
        self.shared.changed.notify_all();
    }

    /// Runs `work` with the channel to itself, like
    /// `Channel:performAtomic`: no other thread pushes or pops until it
    /// returns. Use it to look at a value and take it only if it suits.
    pub fn perform_atomic<R>(&self, work: impl FnOnce(&mut Locked<'_, T>) -> R) -> R {
        let mut locked = Locked { state: self.lock() };
        let result = work(&mut locked);
        drop(locked);
        self.shared.changed.notify_all();
        result
    }
}

impl<T: Clone> Channel<T> {
    /// A copy of the value at the front of the queue, leaving it there, or
    /// `None` if the queue is empty.
    pub fn peek(&self) -> Option<T> {
        self.lock().queue.front().cloned()
    }
}

impl<T> State<T> {
    fn push(&mut self, value: T) -> u64 {
        self.queue.push_back(value);
        self.pushed += 1;
        self.pushed
    }

    fn pop(&mut self) -> Option<T> {
        let value = self.queue.pop_front()?;
        self.read += 1;
        Some(value)
    }

    fn clear(&mut self) {
        self.queue.clear();
        self.read = self.pushed;
    }
}

/// A [`Channel`] held by one thread inside
/// [`Channel::perform_atomic`]. It has the methods that do not wait.
pub struct Locked<'a, T> {
    state: MutexGuard<'a, State<T>>,
}

impl<T> std::fmt::Debug for Locked<'_, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Locked").finish_non_exhaustive()
    }
}

impl<T> Locked<'_, T> {
    /// See [`Channel::push`].
    pub fn push(&mut self, value: T) -> u64 {
        self.state.push(value)
    }

    /// See [`Channel::pop`].
    pub fn pop(&mut self) -> Option<T> {
        self.state.pop()
    }

    /// The value at the front of the queue, leaving it there.
    pub fn peek(&self) -> Option<&T> {
        self.state.queue.front()
    }

    /// See [`Channel::count`].
    pub fn count(&self) -> usize {
        self.state.queue.len()
    }

    /// See [`Channel::has_read`].
    pub fn has_read(&self, id: u64) -> bool {
        self.state.read >= id
    }

    /// See [`Channel::clear`].
    pub fn clear(&mut self) {
        self.state.clear();
    }
}

type Registry = Mutex<HashMap<(String, TypeId), Box<dyn Any + Send>>>;

/// The channel named `name`, made the first time it is asked for, like
/// `love.thread.getChannel`. Every thread that asks for the same name
/// and value type gets the same channel, so threads can find each other
/// without passing handles around.
///
/// Channels of different value types are kept apart even when their
/// names match: `channel::<i32>("log")` and `channel::<String>("log")`
/// are two channels.
pub fn channel<T: Send + 'static>(name: &str) -> Channel<T> {
    static NAMED: OnceLock<Registry> = OnceLock::new();
    let mut named = NAMED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    named
        .entry((name.to_string(), TypeId::of::<T>()))
        .or_insert_with(|| Box::new(Channel::<T>::new()))
        .downcast_ref::<Channel<T>>()
        .expect("each entry holds a channel of the type in its key")
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_come_out_in_the_order_they_went_in() {
        let channel = Channel::new();
        assert_eq!(channel.pop(), None);
        channel.push(1);
        channel.push(2);
        assert_eq!(channel.count(), 2);
        assert_eq!(channel.peek(), Some(1));
        assert_eq!(channel.pop(), Some(1));
        assert_eq!(channel.pop(), Some(2));
        assert_eq!(channel.pop(), None);
    }

    #[test]
    fn has_read_follows_pops_and_clears() {
        let channel = Channel::new();
        let first = channel.push("a");
        let second = channel.push("b");
        assert!(!channel.has_read(first));
        channel.pop();
        assert!(channel.has_read(first));
        assert!(!channel.has_read(second));
        let third = channel.push("c");
        channel.clear();
        assert_eq!(channel.count(), 0);
        assert!(channel.has_read(second) && channel.has_read(third));
    }

    #[test]
    fn demand_waits_for_another_thread() {
        let channel = Channel::new();
        let sender = channel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            sender.push(42);
        });
        assert_eq!(channel.demand(None), Some(42));
        thread.join().unwrap();
    }

    #[test]
    fn demand_and_supply_give_up_after_their_timeout() {
        let channel = Channel::<u8>::new();
        let start = Instant::now();
        assert_eq!(channel.demand(Some(Duration::from_millis(20))), None);
        assert!(start.elapsed() >= Duration::from_millis(20));
        assert!(!channel.supply(1, Some(Duration::from_millis(10))));
        assert_eq!(channel.count(), 1);
    }

    #[test]
    fn supply_returns_once_its_value_is_read() {
        let channel = Channel::new();
        let reader = channel.clone();
        let thread = std::thread::spawn(move || reader.demand(None));
        assert!(channel.supply(7, None));
        assert_eq!(thread.join().unwrap(), Some(7));
        assert_eq!(channel.count(), 0);
    }

    #[test]
    fn atomic_work_sees_a_steady_queue() {
        let channel = Channel::new();
        channel.push(5);
        channel.push(6);
        let taken = channel.perform_atomic(|locked| {
            if locked.peek() == Some(&5) {
                locked.pop()
            } else {
                None
            }
        });
        assert_eq!(taken, Some(5));
        assert_eq!(channel.count(), 1);
    }

    #[test]
    fn named_channels_are_shared_by_name_and_type() {
        channel::<i32>("thread-tests").push(1);
        assert_eq!(channel::<i32>("thread-tests").pop(), Some(1));
        channel::<i32>("thread-tests").push(2);
        assert_eq!(channel::<String>("thread-tests").pop(), None);
        assert_eq!(channel::<i32>("other-thread-tests").pop(), None);
        assert_eq!(channel::<i32>("thread-tests").pop(), Some(2));
    }
}
