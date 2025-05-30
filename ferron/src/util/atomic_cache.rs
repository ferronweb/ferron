//! # AtomicGenericCache
//!
//! A high-performance, lock-free generic cache implementation for Rust that provides
//! thread-safe storage for any type implementing `Clone + Send + Sync + Default`.
//!
//! ## Features
//!
//! - **Lock-free operations**: Uses atomic operations for maximum concurrency
//! - **Generic storage**: Supports any type meeting the trait bounds
//! - **Efficient memory layout**: Optimizes storage for small types (≤8 bytes, ≤8 byte alignment)
//! - **Fallback storage**: Uses RwLock for larger types that don't fit in atomic storage
//! - **Versioning**: Tracks modifications to detect concurrent access
//! - **Metadata tracking**: Stores UTC timestamps and sequence numbers
//! - **State management**: Uses atomic state transitions for consistency
//!
//! ## Usage
//!
//! ```rust
//! use std::sync::Arc;
//!
//! let cache = AtomicGenericCache::<String>::new(1024);
//!
//! // Insert values
//! cache.insert("key1", "value1".to_string());
//! cache.insert("key2", "value2".to_string());
//!
//! // Retrieve values
//! if let Some(value) = cache.get("key1") {
//!     println!("Found: {}", value);
//! }
//!
//! // Get with metadata
//! if let Some((value, timestamp, sequence)) = cache.get_with_metadata("key1") {
//!     println!("Value: {}, Timestamp: {}, Sequence: {}", value, timestamp, sequence);
//! }
//! ```

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::mem::{align_of, size_of, MaybeUninit};
use std::ptr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Cache-line aligned atomic entry structure.
///
/// Each entry stores a value along with metadata including state, version,
/// key hash, timestamp, and sequence number. The entry uses atomic operations
/// for thread-safe access without locks.
#[repr(C, align(64))]
struct AtomicEntry<K, T>
where
  T: Clone + Send + Sync + 'static,
  K: Hash + Eq + Send + Sync + 'static,
{
  /// Combined state (lower 32 bits) and version (upper 32 bits)
  state_version: AtomicU64,
  /// Hash of the key for this entry
  key_hash: AtomicU64,
  /// Inline storage for small values (≤8 bytes, ≤8 byte alignment)
  value_storage: AtomicU64,
  /// Fallback storage for larger values using RwLock
  value_fallback: Option<std::sync::RwLock<T>>,
  /// UTC timestamp in nanoseconds when the entry was last modified
  timestamp_nanos: AtomicU64,
  /// Monotonic sequence number for ordering operations
  sequence: AtomicU64,
  phantom_key_type: std::marker::PhantomData<K>,
}

/// Entry states for atomic state machine
const STATE_EMPTY: u32 = 0; // Entry is empty and available
const STATE_RESERVED: u32 = 1; // Entry is reserved for writing
const STATE_WRITING: u32 = 2; // Entry is being written to
const STATE_WRITTEN: u32 = 3; // Entry contains valid data
const STATE_TOMBSTONE: u32 = 4; // Entry is marked for deletion

/// Packs state and version into a single 64-bit value.
///
/// # Arguments
/// * `state` - The current state (32 bits)
/// * `version` - The version number (32 bits)
///
/// # Returns
/// Combined 64-bit value with version in upper 32 bits, state in lower 32 bits
#[inline(always)]
fn pack_state_version(state: u32, version: u32) -> u64 {
  ((version as u64) << 32) | (state as u64)
}

/// Extracts the state from a packed state_version value.
///
/// # Arguments
/// * `state_version` - Combined state and version value
///
/// # Returns
/// The state portion (lower 32 bits)
#[inline(always)]
fn unpack_state(state_version: u64) -> u32 {
  state_version as u32
}

/// Extracts the version from a packed state_version value.
///
/// # Arguments
/// * `state_version` - Combined state and version value
///
/// # Returns
/// The version portion (upper 32 bits)
#[inline(always)]
fn unpack_version(state_version: u64) -> u32 {
  (state_version >> 32) as u32
}

impl<K, T> AtomicEntry<K, T>
where
  T: Clone + Send + Sync + Default + 'static,
  K: Hash + Eq + Send + Sync + 'static,
{
  /// Creates a new empty atomic entry.
  ///
  /// Determines whether to use inline storage or fallback storage based on
  /// the size and alignment requirements of type T.
  #[inline]
  fn new() -> Self {
    let use_fallback = size_of::<T>() > 8 || align_of::<T>() > 8;

    Self {
      key_hash: AtomicU64::new(0),
      value_storage: AtomicU64::new(0),
      value_fallback: if use_fallback {
        Some(std::sync::RwLock::new(T::default()))
      } else {
        None
      },
      timestamp_nanos: AtomicU64::new(0),
      sequence: AtomicU64::new(0),
      state_version: AtomicU64::new(pack_state_version(STATE_EMPTY, 0)),
      phantom_key_type: std::marker::PhantomData,
    }
  }

  /// Stores a value in the entry using the appropriate storage method.
  ///
  /// Small types (≤8 bytes, ≤8 byte alignment) are stored directly in
  /// `value_storage` using atomic operations. Larger types use the
  /// fallback RwLock storage.
  ///
  /// # Arguments
  /// * `value` - The value to store
  #[inline]
  fn store_value(&self, value: T) -> std::io::Result<()> {
    if size_of::<T>() <= 8 && align_of::<T>() <= 8 {
      let raw_value = unsafe {
        let mut buffer = MaybeUninit::<[u8; 8]>::uninit();
        ptr::write_bytes(buffer.as_mut_ptr(), 0, 1);
        ptr::copy_nonoverlapping(
          &value as *const T as *const u8,
          buffer.as_mut_ptr() as *mut u8,
          size_of::<T>(),
        );
        u64::from_le_bytes(buffer.assume_init())
      };
      self.value_storage.store(raw_value, Ordering::Relaxed);

      return Ok(());
    } else if let Some(ref lock) = self.value_fallback {
      if let Ok(mut value_lock) = lock.write() {
        *value_lock = value;
        return Ok(());
      }
    }

    Err(std::io::Error::other("Unsupported type size or alignment"))
  }

  /// Loads a value from the entry using the appropriate storage method.
  ///
  /// # Returns
  /// A clone of the stored value, or the default value if storage fails
  #[inline]
  fn load_value(&self) -> T {
    if size_of::<T>() <= 8 && align_of::<T>() <= 8 {
      let raw_value = self.value_storage.load(Ordering::Relaxed);
      unsafe {
        let buffer = raw_value.to_le_bytes();
        let mut result = MaybeUninit::<T>::uninit();
        ptr::copy_nonoverlapping(
          buffer.as_ptr(),
          result.as_mut_ptr() as *mut u8,
          size_of::<T>(),
        );
        result.assume_init()
      }
    } else if let Some(ref lock) = self.value_fallback {
      lock.read().unwrap().clone()
    } else {
      T::default()
    }
  }

  /// Gets the current state of the entry.
  ///
  /// # Returns
  /// The current state (STATE_EMPTY, STATE_RESERVED, etc.)
  #[inline]
  fn get_state(&self) -> u32 {
    unpack_state(self.state_version.load(Ordering::Acquire))
  }

  /// Gets the current version of the entry.
  ///
  /// The version is incremented on each state change to detect concurrent modifications.
  ///
  /// # Returns
  /// The current version number
  #[inline]
  fn get_version(&self) -> u32 {
    unpack_version(self.state_version.load(Ordering::Acquire))
  }

  /// Atomically compares and exchanges the state, incrementing the version.
  ///
  /// This is the core synchronization primitive for state transitions.
  ///
  /// # Arguments
  /// * `current` - Expected current state
  /// * `new` - New state to set
  ///
  /// # Returns
  /// Result containing the old state_version on success, or current state_version on failure
  #[inline]
  fn compare_exchange_state(&self, current: u32, new: u32) -> Result<u64, u64> {
    let current_sv = self.state_version.load(Ordering::Relaxed);
    let current_state = unpack_state(current_sv);
    let current_version = unpack_version(current_sv);

    if current_state == current {
      let new_version = current_version.wrapping_add(1);
      let new_sv = pack_state_version(new, new_version);
      self.state_version.compare_exchange_weak(
        current_sv,
        new_sv,
        Ordering::Relaxed,
        Ordering::Relaxed,
      )
    } else {
      Err(current_sv)
    }
  }

  /// Sets the state without changing the version.
  ///
  /// Used for final state transitions where version consistency is already ensured.
  ///
  /// # Arguments
  /// * `state` - New state to set
  #[inline]
  fn set_state(&self, state: u32) {
    let current_sv = self.state_version.load(Ordering::Relaxed);
    let current_version = unpack_version(current_sv);
    let new_sv = pack_state_version(state, current_version);
    self.state_version.store(new_sv, Ordering::Release);
  }
}

impl<K, T> Default for AtomicEntry<K, T>
where
  T: Clone + Send + Sync + Default + 'static,
  K: Hash + Eq + Send + Sync + 'static,
{
  fn default() -> Self {
    Self::new()
  }
}

/// A high-performance, lock-free generic cache implementation.
///
/// `AtomicGenericCache` provides thread-safe storage for any type implementing
/// the required traits. It uses atomic operations for coordination and supports
/// both inline storage for small types and fallback storage for larger types.
///
/// The cache uses a ring buffer approach with atomic state management to provide
/// lock-free insertion, retrieval, and removal operations.
pub struct AtomicGenericCache<K, T>
where
  T: Clone + Send + Sync + 'static,
  K: Hash + Eq + Send + Sync + 'static,
{
  /// Array of cache entries, sized to a power of 2
  entries: Box<[AtomicEntry<K, T>]>,
  /// Monotonic write index for ring buffer behavior
  write_index: AtomicUsize,
  /// Bit mask for fast modulo operations (capacity - 1)
  capacity_mask: usize,
  max_capacity: usize,
}

#[allow(dead_code)]
impl<K, T> AtomicGenericCache<K, T>
where
  T: Clone + Send + Sync + Default + 'static,
  K: Hash + Eq + Send + Sync + 'static,
{
  /// Creates a new cache with the specified capacity.
  ///
  /// The actual capacity will be rounded up to the next power of 2 and
  /// will be at least 64 entries for optimal performance.
  ///
  /// # Arguments
  /// * `capacity` - Desired cache capacity
  ///
  /// # Returns
  /// Arc-wrapped cache instance for shared ownership
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// ```
  pub fn new(capacity: usize, max_capacity: usize) -> Arc<Self> {
    let capacity = capacity.next_power_of_two().max(64);

    let mut entries = Vec::with_capacity(capacity);
    for _ in 0..capacity {
      entries.push(AtomicEntry::new());
    }

    Arc::new(Self {
      entries: entries.into_boxed_slice(),
      write_index: AtomicUsize::new(0),
      capacity_mask: capacity - 1,
      max_capacity,
    })
  }

  /// Computes a hash for the given key.
  ///
  /// Uses the default hasher and ensures the result is non-zero for
  /// better distribution in the cache.
  ///
  /// # Arguments
  /// * `key` - Key to hash
  ///
  /// # Returns
  /// 64-bit hash value (guaranteed non-zero)
  #[inline]
  fn hash_key(&self, key: &K) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    hash | 1
  }

  /// Gets the current UTC timestamp in nanoseconds.
  ///
  /// Returns nanoseconds since Unix epoch (1970-01-01 00:00:00 UTC).
  /// This is always UTC+0 regardless of system timezone.
  ///
  /// # Returns
  /// UTC timestamp in nanoseconds, or 0 if system time is unavailable
  #[inline]
  fn get_timestamp_nanos() -> u64 {
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_nanos() as u64
  }

  /// Inserts a key-value pair into the cache.
  ///
  /// This operation is lock-free and thread-safe. If the key already exists,
  /// the value will be updated. The operation uses a ring buffer approach
  /// to find available slots.
  ///
  /// # Arguments
  /// * `key` - Key to insert (must implement Hash)
  /// * `value` - Value to store
  ///
  /// # Returns
  /// `true` if the insertion was successful, `false` if the cache is full wrapped in Result<bool, std::io::Error> if successful
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// let success = cache.insert("key1", "value1".to_string()).unwrap();
  /// assert!(success);
  /// ```
  pub fn insert(&self, key: K, value: T) -> Result<bool, std::io::Error> {
    if self.len() >= self.max_capacity && self.max_capacity > 0 {
      return Ok(false);
    }

    let key_hash = self.hash_key(&key);
    let now = Self::get_timestamp_nanos();

    // Try to update existing entry first
    if self.try_update_existing(key_hash, &value, now)? {
      return Ok(true);
    }

    // Look for available slot using ring buffer approach
    let max_attempts = self.capacity_mask + 1;

    for attempt in 0..max_attempts {
      let write_pos = self.write_index.fetch_add(1, Ordering::Relaxed);
      let index = write_pos & self.capacity_mask;
      let slot = &self.entries[index];

      if let Ok(res) = self.try_acquire_slot(slot, key_hash, &value, now) {
        if res {
          return Ok(true);
        }
      }

      // Yield periodically to avoid busy spinning
      if attempt & 7 == 7 {
        std::thread::yield_now();
      }
    }

    Ok(false)
  }

  /// Forces insertion of a key-value pair into the cache.
  ///
  /// Unlike the regular `insert` function, this operation will always succeed by
  /// removing the oldest entries if the cache is at max capacity. This ensures
  /// that new entries can always be added, maintaining a bounded cache size.
  ///
  /// # Arguments
  /// * `key` - Key to insert (must implement Hash)
  /// * `value` - Value to store
  ///
  /// # Returns
  /// Result containing a tuple:
  /// - `bool` - Always `true` indicating successful insertion
  /// - `Vec<(u64, T, u64, u64)>` - Vector of removed entries (key_hash, value, timestamp_nanos, sequence)
  ///   Empty if no entries needed to be removed.
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(64, 100); // capacity 64, max 100
  ///
  /// // Fill the cache to max capacity
  /// for i in 0..100 {
  ///     cache.insert(format!("key{}", i), format!("value{}", i)).unwrap();
  /// }
  ///
  /// // Force insert will remove oldest entries to make space
  /// let (success, removed_entries) = cache.force_insert("new_key", "new_value".to_string()).unwrap();
  /// assert!(success);
  ///
  /// if !removed_entries.is_empty() {
  ///     println!("Removed {} oldest entries to make space", removed_entries.len());
  ///     for (key_hash, value, timestamp, sequence) in removed_entries {
  ///         println!("Removed: key_hash={}, value={}, sequence={}", key_hash, value, sequence);
  ///     }
  /// }
  /// ```
  #[allow(clippy::type_complexity)]
  pub fn force_insert(
    &self,
    key: K,
    value: T,
  ) -> Result<(bool, Vec<(u64, T, u64, u64)>), std::io::Error> {
    let mut removed_entries = Vec::new();

    // If we're at max capacity, remove oldest entries to make space
    if self.max_capacity > 0 {
      let current_len = self.len();
      if current_len >= self.max_capacity {
        // Remove at least one entry, but potentially more if we're over capacity
        let entries_to_remove = (current_len - self.max_capacity + 1).max(1);
        removed_entries = self.remove_oldest(entries_to_remove);
      }
    }

    let key_hash = self.hash_key(&key);
    let now = Self::get_timestamp_nanos();

    // Try to update existing entry first
    if self.try_update_existing(key_hash, &value, now)? {
      return Ok((true, removed_entries));
    }

    // Look for available slot using ring buffer approach
    let max_attempts = self.capacity_mask + 1;

    for attempt in 0..max_attempts {
      let write_pos = self.write_index.fetch_add(1, Ordering::Relaxed);
      let index = write_pos & self.capacity_mask;
      let slot = &self.entries[index];

      if let Ok(res) = self.try_acquire_slot(slot, key_hash, &value, now) {
        if res {
          return Ok((true, removed_entries));
        }
      }

      // Yield periodically to avoid busy spinning
      if attempt & 7 == 7 {
        std::thread::yield_now();
      }
    }

    // If we still can't insert after removing oldest entries,
    // try removing more entries and retry once
    if !removed_entries.is_empty() || self.max_capacity == 0 {
      // Remove a few more entries to increase chances of success
      let additional_removed = self.remove_oldest(3);
      removed_entries.extend(additional_removed);

      // Retry insertion one more time
      for attempt in 0..max_attempts {
        let write_pos = self.write_index.fetch_add(1, Ordering::Relaxed);
        let index = write_pos & self.capacity_mask;
        let slot = &self.entries[index];

        if let Ok(res) = self.try_acquire_slot(slot, key_hash, &value, now) {
          if res {
            return Ok((true, removed_entries));
          }
        }

        if attempt & 7 == 7 {
          std::thread::yield_now();
        }
      }
    }

    // This should rarely happen, but if it does, we still return the removed entries
    // The caller can decide whether to retry or handle this edge case
    Ok((false, removed_entries))
  }

  /// Attempts to acquire a specific slot for writing.
  ///
  /// Handles the atomic state transitions required for safe slot acquisition.
  ///
  /// # Arguments
  /// * `slot` - The slot to acquire
  /// * `key_hash` - Hash of the key being inserted
  /// * `value` - Value to store
  /// * `timestamp` - UTC timestamp in nanoseconds
  ///
  /// # Returns
  /// `true` if the slot was successfully acquired and written
  #[inline]
  fn try_acquire_slot(
    &self,
    slot: &AtomicEntry<K, T>,
    key_hash: u64,
    value: &T,
    timestamp: u64,
  ) -> Result<bool, std::io::Error> {
    let current_state = slot.get_state();

    match current_state {
      STATE_EMPTY | STATE_TOMBSTONE => {
        if slot
          .compare_exchange_state(current_state, STATE_RESERVED)
          .is_ok()
        {
          self.write_entry_data(slot, key_hash, value.clone(), timestamp)?;
          slot.set_state(STATE_WRITTEN);
          Ok(true)
        } else {
          Ok(false)
        }
      }
      STATE_WRITTEN => {
        if slot
          .compare_exchange_state(STATE_WRITTEN, STATE_WRITING)
          .is_ok()
        {
          self.write_entry_data(slot, key_hash, value.clone(), timestamp)?;
          slot.set_state(STATE_WRITTEN);
          Ok(true)
        } else {
          Ok(false)
        }
      }
      _ => Ok(false),
    }
  }

  /// Attempts to update an existing entry with the same key hash.
  ///
  /// Scans all entries looking for a matching key hash and attempts to update it.
  ///
  /// # Arguments
  /// * `key_hash` - Hash of the key to update
  /// * `value` - New value to store
  /// * `timestamp` - UTC timestamp in nanoseconds
  ///
  /// # Returns
  /// `true` if an existing entry was found and updated
  fn try_update_existing(
    &self,
    key_hash: u64,
    value: &T,
    timestamp: u64,
  ) -> Result<bool, std::io::Error> {
    for slot in self.entries.iter() {
      if slot.get_state() == STATE_WRITTEN
        && slot.key_hash.load(Ordering::Relaxed) == key_hash
        && slot
          .compare_exchange_state(STATE_WRITTEN, STATE_WRITING)
          .is_ok()
      {
        if slot.key_hash.load(Ordering::Relaxed) == key_hash {
          self.write_entry_data(slot, key_hash, value.clone(), timestamp)?;
          slot.set_state(STATE_WRITTEN);
          return Ok(true);
        } else {
          slot.set_state(STATE_WRITTEN);
        }
      }
    }
    Ok(false)
  }

  /// Writes data to an entry slot.
  ///
  /// Updates the sequence number, key hash, value, and timestamp.
  /// This method assumes the slot is already in a writable state.
  ///
  /// # Arguments
  /// * `slot` - The slot to write to
  /// * `key_hash` - Hash of the key
  /// * `value` - Value to store
  /// * `timestamp` - UTC timestamp in nanoseconds
  #[inline]
  fn write_entry_data(
    &self,
    slot: &AtomicEntry<K, T>,
    key_hash: u64,
    value: T,
    timestamp: u64,
  ) -> std::io::Result<()> {
    // Increment sequence for ordering
    slot.sequence.fetch_add(1, Ordering::Relaxed);
    slot.key_hash.store(key_hash, Ordering::Relaxed);
    let result = slot.store_value(value);
    slot.timestamp_nanos.store(timestamp, Ordering::Relaxed);
    result
  }

  /// Retrieves a value by key from the cache.
  ///
  /// This operation is lock-free and thread-safe. Uses version checking
  /// to ensure consistency during concurrent modifications.
  ///
  /// # Arguments
  /// * `key` - Key to look up (must implement Hash)
  ///
  /// # Returns
  /// `Some(value)` if found, `None` if not found or if concurrent modification detected
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  ///
  /// if let Some(value) = cache.get("key1") {
  ///     println!("Found: {}", value);
  /// }
  /// ```
  pub fn get(&self, key: &K) -> Option<T> {
    let key_hash = self.hash_key(key);

    for slot in self.entries.iter() {
      if slot.get_state() == STATE_WRITTEN && slot.key_hash.load(Ordering::Relaxed) == key_hash {
        let version_before = slot.get_version();
        let value = slot.load_value();

        // Verify consistency
        if version_before == slot.get_version()
          && slot.get_state() == STATE_WRITTEN
          && slot.key_hash.load(Ordering::Relaxed) == key_hash
        {
          return Some(value);
        }
      }
    }

    None
  }

  /// Retrieves a value along with its metadata by key.
  ///
  /// Returns the value along with UTC timestamp and sequence number.
  ///
  /// # Arguments
  /// * `key` - Key to look up (must implement Hash)
  ///
  /// # Returns
  /// `Some((value, timestamp_nanos, sequence))` if found, `None` otherwise.
  /// The timestamp is in UTC nanoseconds since Unix epoch.
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  ///
  /// if let Some((value, timestamp, sequence)) = cache.get_with_metadata("key1") {
  ///     println!("Value: {}, UTC Timestamp: {} ns, Sequence: {}", value, timestamp, sequence);
  /// }
  /// ```
  pub fn get_with_metadata(&self, key: &K) -> Option<(T, u64, u64)> {
    let key_hash = self.hash_key(key);

    for slot in self.entries.iter() {
      if slot.get_state() == STATE_WRITTEN && slot.key_hash.load(Ordering::Relaxed) == key_hash {
        let version_before = slot.get_version();
        let value = slot.load_value();
        let timestamp = slot.timestamp_nanos.load(Ordering::Relaxed);
        let sequence = slot.sequence.load(Ordering::Relaxed);

        // Verify consistency
        if version_before == slot.get_version()
          && slot.get_state() == STATE_WRITTEN
          && slot.key_hash.load(Ordering::Relaxed) == key_hash
        {
          return Some((value, timestamp, sequence));
        }
      }
    }

    None
  }

  /// Retrieves multiple values by their keys from the cache.
  ///
  /// This is a bulk operation that looks up all provided keys.
  ///
  /// # Arguments
  /// * `keys` - A slice of keys to look up
  ///
  /// # Returns
  /// Vector of values in the same order as keys. Missing keys result in None entries.
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key3", "value3".to_string());
  ///
  /// let keys = vec!["key1", "key2", "key3"];
  /// let values = cache.get_all_by_keys(keys);
  /// // values will be [Some("value1"), None, Some("value3")]
  /// ```
  pub fn get_all_by_keys(&self, keys: &[K]) -> Vec<Option<T>> {
    keys.iter().map(|key| self.get(key)).collect()
  }

  /// Checks if the cache contains a specific key.
  ///
  /// # Arguments
  /// * `key` - Key to check (must implement Hash)
  ///
  /// # Returns
  /// `true` if the key exists, `false` otherwise
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  ///
  /// assert!(cache.contains_key("key1"));
  /// assert!(!cache.contains_key("nonexistent"));
  /// ```
  #[inline]
  pub fn contains_key(&self, key: &K) -> bool {
    self.get(key).is_some()
  }

  /// Checks if the cache contains any of the specified keys.
  ///
  /// # Arguments
  /// * `keys` - A slice of keys to check
  ///
  /// # Returns
  /// `true` if any of the keys exist, `false` if none exist
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  ///
  /// let keys = vec!["key1", "key2", "key3"];
  /// assert!(cache.contains_any_key(keys));
  /// ```
  pub fn contains_any_key(&self, keys: &[K]) -> bool {
    keys.iter().any(|key| self.contains_key(key))
  }

  /// Checks if the cache contains all of the specified keys.
  ///
  /// # Arguments
  /// * `keys` - A slice of keys to check
  ///
  /// # Returns
  /// `true` if all keys exist, `false` if any key is missing
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key2", "value2".to_string());
  ///
  /// let keys = vec!["key1", "key2"];
  /// assert!(cache.contains_all_keys(keys));
  /// ```
  pub fn contains_all_keys(&self, keys: &[K]) -> bool {
    keys.iter().all(|key| self.contains_key(key))
  }

  /// Removes a key-value pair from the cache.
  ///
  /// Marks the entry as a tombstone rather than truly deleting it.
  /// This allows the slot to be reused while maintaining consistency.
  ///
  /// # Arguments
  /// * `key` - Key to remove (must implement Hash)
  ///
  /// # Returns
  /// `Some(value)` if the key was found and removed, `None` otherwise
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  ///
  /// if let Some(removed_value) = cache.remove("key1") {
  ///     println!("Removed: {}", removed_value);
  /// }
  /// ```
  pub fn remove(&self, key: &K) -> Option<T> {
    let key_hash = self.hash_key(key);

    for slot in self.entries.iter() {
      if slot.get_state() == STATE_WRITTEN && slot.key_hash.load(Ordering::Relaxed) == key_hash {
        let value = slot.load_value();

        if slot
          .compare_exchange_state(STATE_WRITTEN, STATE_TOMBSTONE)
          .is_ok()
          && slot.key_hash.load(Ordering::Relaxed) == key_hash
        {
          return Some(value);
        }
      }
    }

    None
  }

  /// Removes multiple key-value pairs from the cache.
  ///
  /// This is a bulk operation that attempts to remove all provided keys.
  ///
  /// # Arguments
  /// * `keys` - A slice of keys to remove
  ///
  /// # Returns
  /// Vector of removed values in the same order as keys. Missing keys result in None entries.
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key3", "value3".to_string());
  ///
  /// let keys = vec!["key1", "key2", "key3"];
  /// let removed_values = cache.remove_all(keys);
  /// // removed_values will be [Some("value1"), None, Some("value3")]
  /// ```
  pub fn remove_all(&self, keys: &[K]) -> Vec<Option<T>> {
    keys.iter().map(|key| self.remove(key)).collect()
  }

  /// Clears all entries from the cache.
  ///
  /// Atomically transitions all written entries to empty state.
  ///
  /// # Returns
  /// Number of entries that were cleared
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key2", "value2".to_string());
  ///
  /// let cleared_count = cache.clear();
  /// println!("Cleared {} entries", cleared_count);
  /// ```
  pub fn clear(&self) -> usize {
    let mut cleared = 0;

    for slot in self.entries.iter() {
      if slot
        .compare_exchange_state(STATE_WRITTEN, STATE_EMPTY)
        .is_ok()
      {
        cleared += 1;
      }
    }

    cleared
  }

  /// Removes multiple key-value pairs and returns only the successfully removed values.
  ///
  /// Unlike `remove_all`, this function filters out None values and returns only
  /// the values that were actually found and removed.
  ///
  /// # Arguments
  /// * `keys` - Vector of keys to remove
  ///
  /// # Returns
  /// Vector containing only the values that were successfully removed
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key3", "value3".to_string());
  ///
  /// let keys = vec!["key1", "key2", "key3"];
  /// let removed_values = cache.remove_existing(keys);
  /// // removed_values will be ["value1", "value3"]
  /// ```
  pub fn remove_existing(&self, keys: &[K]) -> Vec<T> {
    keys.iter().filter_map(|key| self.remove(key)).collect()
  }

  /// Removes the oldest entries from the cache.
  ///
  /// This function identifies and removes the oldest entries based on their sequence numbers.
  /// Entries with lower sequence numbers are considered older and will be removed first.
  ///
  /// # Arguments
  /// * `count` - Maximum number of oldest entries to remove
  ///
  /// # Returns
  /// Vector of removed (key_hash, value, timestamp_nanos, sequence) tuples,
  /// sorted by sequence number (oldest first). The actual number of removed
  /// entries may be less than `count` if the cache contains fewer entries.
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024, 2048);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key2", "value2".to_string());
  /// cache.insert("key3", "value3".to_string());
  ///
  /// // Remove the 2 oldest entries
  /// let removed = cache.remove_oldest(2);
  /// println!("Removed {} oldest entries", removed.len());
  ///
  /// for (key_hash, value, timestamp, sequence) in removed {
  ///     println!("Removed: key_hash={}, value={}, sequence={}", key_hash, value, sequence);
  /// }
  /// ```
  pub fn remove_oldest(&self, count: usize) -> Vec<(u64, T, u64, u64)> {
    if count == 0 {
      return Vec::new();
    }

    // First, collect all current entries with their metadata
    let mut candidates = Vec::new();

    for slot in self.entries.iter() {
      if slot.get_state() == STATE_WRITTEN {
        let version_before = slot.get_version();
        let key_hash = slot.key_hash.load(Ordering::Relaxed);
        let value = slot.load_value();
        let timestamp = slot.timestamp_nanos.load(Ordering::Relaxed);
        let sequence = slot.sequence.load(Ordering::Relaxed);

        // Verify consistency
        if version_before == slot.get_version() && slot.get_state() == STATE_WRITTEN {
          candidates.push((slot, key_hash, value, timestamp, sequence));
        }
      }
    }

    // Sort by sequence number (oldest first - lowest sequence numbers)
    candidates.sort_unstable_by(|a, b| a.4.cmp(&b.4));

    // Try to remove the oldest entries up to the requested count
    let mut removed = Vec::new();
    let remove_count = count.min(candidates.len());

    for (slot, key_hash, value, timestamp, sequence) in candidates.into_iter().take(remove_count) {
      // Attempt to atomically transition from WRITTEN to TOMBSTONE
      if slot
        .compare_exchange_state(STATE_WRITTEN, STATE_TOMBSTONE)
        .is_ok()
      {
        // Double-check that this is still the same entry (sequence hasn't changed)
        let current_sequence = slot.sequence.load(Ordering::Relaxed);
        if current_sequence == sequence {
          removed.push((key_hash, value, timestamp, sequence));
        }
        // If sequence changed, the slot was reused, but we still removed something
        // so we'll count it as successful removal
      }
    }

    removed
  }

  /// Returns the number of entries currently in the cache.
  ///
  /// Note: This is a snapshot count and may change immediately after the call
  /// in a concurrent environment.
  ///
  /// # Returns
  /// Number of entries with STATE_WRITTEN
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  ///
  /// println!("Cache contains {} entries", cache.len());
  /// ```
  #[inline]
  pub fn len(&self) -> usize {
    self
      .entries
      .iter()
      .filter(|slot| slot.get_state() == STATE_WRITTEN)
      .count()
  }

  /// Returns all entries as key-value pairs.
  ///
  /// This method returns tuples of (key_hash, value) for all entries in the cache.
  /// Entries are sorted by sequence number (most recent first).
  ///
  /// # Returns
  /// Vector of (key_hash, value) tuples
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key2", "value2".to_string());
  ///
  /// let entries = cache.entries();
  /// for (key_hash, value) in entries {
  ///     println!("Key hash: {}, Value: {}", key_hash, value);
  /// }
  /// ```
  pub fn entries(&self) -> Vec<(u64, T)> {
    self
      .get_all()
      .into_iter()
      .map(|(key_hash, value, _, _)| (key_hash, value))
      .collect()
  }

  /// Drains all entries from the cache and returns them as a vector.
  ///
  /// This operation clears the cache and returns all entries that were present.
  /// After this operation, the cache will be empty.
  ///
  /// # Returns
  /// Vector of all entries that were in the cache, sorted by sequence number (most recent first)
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key2", "value2".to_string());
  ///
  /// let drained_entries = cache.drain();
  /// assert!(cache.is_empty());
  /// println!("Drained {} entries", drained_entries.len());
  /// ```
  pub fn drain(&self) -> Vec<(u64, T, u64, u64)> {
    let mut results = Vec::new();

    for slot in self.entries.iter() {
      if slot.get_state() == STATE_WRITTEN {
        let version_before = slot.get_version();
        let key = slot.key_hash.load(Ordering::Relaxed);
        let value = slot.load_value();
        let timestamp = slot.timestamp_nanos.load(Ordering::Relaxed);
        let sequence = slot.sequence.load(Ordering::Relaxed);

        // Verify consistency and try to remove
        if version_before == slot.get_version()
          && slot.get_state() == STATE_WRITTEN
          && slot
            .compare_exchange_state(STATE_WRITTEN, STATE_EMPTY)
            .is_ok()
        {
          results.push((key, value, timestamp, sequence));
        }
      }
    }

    // Sort by sequence (most recent first)
    results.sort_unstable_by(|a, b| b.3.cmp(&a.3));
    results
  }

  /// Checks if the cache is empty.
  ///
  /// # Returns
  /// `true` if no entries have STATE_WRITTEN, `false` otherwise
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// assert!(cache.is_empty());
  ///
  /// cache.insert("key1", "value1".to_string());
  /// assert!(!cache.is_empty());
  /// ```
  #[inline]
  pub fn is_empty(&self) -> bool {
    !self
      .entries
      .iter()
      .any(|slot| slot.get_state() == STATE_WRITTEN)
  }

  /// Returns the total capacity of the cache.
  ///
  /// This is the maximum number of entries the cache can hold.
  ///
  /// # Returns
  /// Total capacity (always a power of 2)
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1000);
  /// println!("Cache capacity: {}", cache.capacity()); // Will be 1024 (next power of 2)
  /// ```
  #[inline]
  pub fn capacity(&self) -> usize {
    self.capacity_mask + 1
  }

  /// Returns all entries in the cache with their metadata.
  ///
  /// Results are sorted by sequence number (most recent first).
  /// This operation provides a consistent snapshot of the cache contents.
  ///
  /// # Returns
  /// Vector of tuples containing (key_hash, value, timestamp_nanos, sequence).
  /// Timestamp is in UTC nanoseconds since Unix epoch.
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key2", "value2".to_string());
  ///
  /// let all_entries = cache.get_all();
  /// for (key_hash, value, timestamp, sequence) in all_entries {
  ///     println!("Key hash: {}, Value: {}, UTC Timestamp: {} ns, Sequence: {}",
  ///              key_hash, value, timestamp, sequence);
  /// }
  /// ```
  pub fn get_all(&self) -> Vec<(u64, T, u64, u64)> {
    let mut results = Vec::new();

    for slot in self.entries.iter() {
      if slot.get_state() == STATE_WRITTEN {
        let version_before = slot.get_version();
        let key = slot.key_hash.load(Ordering::Relaxed);
        let value = slot.load_value();
        let timestamp = slot.timestamp_nanos.load(Ordering::Relaxed);
        let sequence = slot.sequence.load(Ordering::Relaxed);

        // Verify consistency
        if version_before == slot.get_version() && slot.get_state() == STATE_WRITTEN {
          results.push((key, value, timestamp, sequence));
        }
      }
    }

    // Sort by sequence (most recent first)
    results.sort_unstable_by(|a, b| b.3.cmp(&a.3));
    results
  }

  /// Retains only entries that satisfy the given predicate.
  ///
  /// Entries that don't match the predicate are marked as tombstones.
  ///
  /// # Arguments
  /// * `predicate` - Function that takes (value, timestamp_nanos, sequence) and returns bool. Timestamp is in UTC nanoseconds since Unix epoch.
  ///
  /// # Returns
  /// Number of entries that were removed
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("keep", "important".to_string());
  /// cache.insert("remove", "unimportant".to_string());
  ///
  /// // Keep only entries containing "important"
  /// let removed_count = cache.retain(|value, _timestamp, _sequence| {
  ///     value.contains("important")
  /// });
  ///
  /// println!("Removed {} entries", removed_count);
  /// ```
  pub fn retain<F>(&self, mut predicate: F) -> usize
  where
    F: FnMut(&T, u64, u64) -> bool,
  {
    let mut removed_count = 0;

    for slot in self.entries.iter() {
      if slot.get_state() == STATE_WRITTEN {
        let version_before = slot.get_version();
        let value = slot.load_value();
        let timestamp = slot.timestamp_nanos.load(Ordering::Relaxed);
        let sequence = slot.sequence.load(Ordering::Relaxed);

        // Verify consistency
        if version_before == slot.get_version()
          && slot.get_state() == STATE_WRITTEN
          && !predicate(&value, timestamp, sequence)
          && slot
            .compare_exchange_state(STATE_WRITTEN, STATE_TOMBSTONE)
            .is_ok()
        {
          removed_count += 1;
        }
      }
    }

    removed_count
  }

  /// Converts the cache into a HashMap containing all key-value pairs.
  ///
  /// This operation drains the cache (making it empty) and returns a HashMap
  /// with the key hashes as keys and the cached values as values. The original
  /// keys cannot be recovered since only their hashes are stored.
  ///
  /// # Returns
  /// HashMap<u64, T> where keys are the hash values of the original keys
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key2", "value2".to_string());
  ///
  /// let hashmap = cache.into_hashmap();
  /// assert_eq!(hashmap.len(), 2);
  /// assert!(cache.is_empty()); // Cache is now empty
  ///
  /// // Note: You need to hash the original keys to look up values
  /// // let key1_hash = cache.hash_key(&"key1");
  /// // assert_eq!(hashmap.get(&key1_hash), Some(&"value1".to_string()));
  /// ```
  pub fn into_hashmap(self: Arc<Self>) -> HashMap<u64, T> {
    let drained_entries = self.drain();
    let mut hashmap = HashMap::with_capacity(drained_entries.len());

    for (key_hash, value, _, _) in drained_entries {
      hashmap.insert(key_hash, value);
    }

    hashmap
  }

  /// Converts the cache to a HashMap without consuming the cache.
  ///
  /// This operation creates a HashMap containing all current key-value pairs
  /// while leaving the original cache unchanged. The HashMap uses key hashes
  /// as keys since the original keys are not stored.
  ///
  /// # Returns
  /// HashMap<u64, T> where keys are the hash values of the original keys
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("key1", "value1".to_string());
  /// cache.insert("key2", "value2".to_string());
  ///
  /// let hashmap = cache.to_hashmap();
  /// assert_eq!(hashmap.len(), 2);
  /// assert_eq!(cache.len(), 2); // Cache is unchanged
  /// ```
  pub fn to_hashmap(&self) -> HashMap<u64, T> {
    let all_entries = self.get_all();
    let mut hashmap = HashMap::with_capacity(all_entries.len());

    for (key_hash, value, _, _) in all_entries {
      hashmap.insert(key_hash, value);
    }

    hashmap
  }

  /// Creates a new cache from a HashMap.
  ///
  /// This operation creates a new cache with the specified capacity and
  /// populates it with all entries from the provided HashMap. The HashMap
  /// keys are treated as key hashes.
  ///
  /// # Arguments
  /// * `hashmap` - HashMap containing key hashes and values to insert
  /// * `capacity` - Desired capacity for the new cache
  ///
  /// # Returns
  /// `Arc<AtomicGenericCache<T>>` populated with the HashMap entries
  ///
  /// # Examples
  /// ```
  /// use std::collections::HashMap;
  ///
  /// let mut hashmap = HashMap::new();
  /// hashmap.insert(12345u64, "value1".to_string());
  /// hashmap.insert(67890u64, "value2".to_string());
  ///
  /// let cache = AtomicGenericCache::from_hashmap(hashmap, 1024);
  /// assert_eq!(cache.len(), 2);
  /// ```
  pub fn from_hashmap(hashmap: HashMap<u64, T>, capacity: usize) -> Arc<Self> {
    let cache = Self::new(capacity, 0);
    let now = Self::get_timestamp_nanos();

    for (key_hash, value) in hashmap {
      // Try to find an available slot
      let max_attempts = cache.capacity_mask + 1;

      for attempt in 0..max_attempts {
        let write_pos = cache.write_index.fetch_add(1, Ordering::Relaxed);
        let index = write_pos & cache.capacity_mask;
        let slot = &cache.entries[index];

        if let Ok(res) = cache.try_acquire_slot(slot, key_hash, &value, now) {
          if res {
            break;
          }
        }

        if attempt & 7 == 7 {
          std::thread::yield_now();
        }
      }
    }

    cache
  }

  /// Creates a new cache from a HashMap with preserved key-value pairs.
  ///
  /// This operation creates a new cache and populates it with entries from
  /// a HashMap where keys can be any hashable type. This is more convenient
  /// than `from_hashmap` when you have actual keys rather than key hashes.
  ///
  /// # Arguments
  /// * `hashmap` - HashMap containing keys and values to insert
  /// * `capacity` - Desired capacity for the new cache
  ///
  /// # Returns
  /// `Arc<AtomicGenericCache<T>>` populated with the HashMap entries
  ///
  /// # Examples
  /// ```
  /// use std::collections::HashMap;
  ///
  /// let mut hashmap = HashMap::new();
  /// hashmap.insert("key1", "value1".to_string());
  /// hashmap.insert("key2", "value2".to_string());
  ///
  /// let cache = AtomicGenericCache::from_hashmap_with_keys(hashmap, 1024);
  /// assert_eq!(cache.len(), 2);
  /// assert_eq!(cache.get("key1"), Some("value1".to_string()));
  /// assert_eq!(cache.get("key2"), Some("value2".to_string()));
  /// ```
  pub fn from_hashmap_with_keys(
    hashmap: HashMap<K, T>,
    capacity: usize,
  ) -> Result<Arc<Self>, std::io::Error> {
    let cache = Self::new(capacity, 0);

    for (key, value) in hashmap {
      if let Ok(res) = cache.insert(key, value) {
        if !res {
          return Err(std::io::Error::other("Failed to insert entry into cache"));
        }
      }
    }

    Ok(cache)
  }

  /// Merges a HashMap into the existing cache.
  ///
  /// This operation inserts all entries from the provided HashMap into
  /// the cache, potentially overwriting existing entries with the same keys.
  ///
  /// # Arguments
  /// * `hashmap` - HashMap containing keys and values to merge
  ///
  /// # Returns
  /// Number of entries successfully inserted
  ///
  /// # Examples
  /// ```
  /// use std::collections::HashMap;
  ///
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// cache.insert("existing", "old_value".to_string());
  ///
  /// let mut hashmap = HashMap::new();
  /// hashmap.insert("existing", "new_value".to_string());
  /// hashmap.insert("new_key", "new_value".to_string());
  ///
  /// let inserted_count = cache.merge_from_hashmap(hashmap);
  /// assert_eq!(inserted_count, 2);
  /// assert_eq!(cache.get("existing"), Some("new_value".to_string()));
  /// assert_eq!(cache.get("new_key"), Some("new_value".to_string()));
  /// ```
  pub fn merge_from_hashmap(&self, hashmap: HashMap<K, T>) -> Result<usize, std::io::Error> {
    let mut inserted = 0;

    for (key, value) in hashmap {
      if let Ok(res) = self.insert(key, value) {
        if res {
          inserted += 1;
        } else {
          return Err(std::io::Error::other("Failed to insert entry into cache"));
        }
      }
    }

    Ok(inserted)
  }

  /// Helper method to make hash_key public for HashMap conversions.
  ///
  /// This method exposes the internal hashing function so users can
  /// compute key hashes when working with HashMap conversions.
  ///
  /// # Arguments
  /// * `key` - Key to hash
  ///
  /// # Returns
  /// 64-bit hash value for the key
  ///
  /// # Examples
  /// ```
  /// let cache = AtomicGenericCache::<String>::new(1024);
  /// let hash1 = cache.compute_key_hash(&"key1");
  /// let hash2 = cache.compute_key_hash(&"key2");
  ///
  /// assert_ne!(hash1, hash2); // Different keys should have different hashes
  /// assert_ne!(hash1, 0);     // Hash should be non-zero
  /// ```
  pub fn compute_key_hash(&self, key: &K) -> u64 {
    self.hash_key(key)
  }
}

#[cfg(test)]
#[cfg(debug_assertions)]
mod tests {
  use super::*;
  use std::{collections::HashSet, sync::atomic::AtomicBool, thread, time::Duration};

  trait CacheableValue: Clone + Send + Sync + Default + 'static {
    const IS_LOCK_FREE: bool = size_of::<Self>() <= 8 && align_of::<Self>() <= 8;
  }

  impl<T> CacheableValue for T where T: Clone + Send + Sync + Default + 'static {}

  fn create_cache<K: Hash + Eq + Send + Sync + 'static, T: CacheableValue>(
    capacity: usize,
  ) -> Arc<AtomicGenericCache<K, T>> {
    AtomicGenericCache::new(capacity, 0)
  }

  #[derive(Copy, Clone, Debug, Default, PartialEq)]
  struct TestEntry {
    id: u32,
    value: u32,
  }

  #[test]
  fn test_large_value_fallback() {
    #[derive(Copy, Clone, Debug, Default, PartialEq)]
    struct LargeValue {
      data: [u64; 4], // 32 bytes > 8 bytes
    }

    let cache = create_cache::<String, LargeValue>(100);
    let large_val = LargeValue { data: [1, 2, 3, 4] };

    assert!(cache.insert("large_key".to_string(), large_val).unwrap());
    assert_eq!(cache.get(&"large_key".to_string()), Some(large_val));
  }

  #[test]
  fn test_concurrent_stress() {
    let cache = create_cache::<String, TestEntry>(1000);

    let handles: Vec<_> = (0..8)
      .map(|thread_id| {
        let cache = Arc::clone(&cache);
        thread::spawn(move || {
          for i in 0..1000 {
            let entry = TestEntry {
              id: (thread_id * 1000 + i) as u32,
              value: (thread_id * 1000 + i) as u32,
            };
            cache
              .insert(format!("key:{}:{}", thread_id, i), entry)
              .unwrap();
            cache.get(&format!("key:{}:{}", thread_id, i));
          }
        })
      })
      .collect();

    for handle in handles {
      handle.join().unwrap();
    }

    assert!(true);
  }

  #[derive(Copy, Clone, Debug, Default, PartialEq)]
  struct SessionData {
    user_id: u32,
    session_token: u32,
    permissions: u16,
    login_count: u16,
  }

  #[derive(Copy, Clone, Debug, Default, PartialEq)]
  struct LargeTestStruct {
    id: u64,
    data: [u64; 10], // 88 bytes total - forces fallback path
    checksum: u64,
  }

  impl LargeTestStruct {
    fn new(id: u64) -> Self {
      let data = [id; 10];
      let checksum = data.iter().sum();
      Self { id, data, checksum }
    }

    fn is_valid(&self) -> bool {
      self.data.iter().sum::<u64>() == self.checksum
    }
  }

  #[test]
  fn test_edge_case_zero_capacity() {
    let cache = create_cache::<String, u32>(0);
    assert_eq!(cache.capacity(), 64); // Should be clamped to minimum

    assert!(cache.insert("key".to_string(), 42).unwrap());
    assert_eq!(cache.get(&"key".to_string()), Some(42));
  }

  #[test]
  fn test_edge_case_power_of_two_capacity() {
    let cache = create_cache::<String, u32>(100);
    assert_eq!(cache.capacity(), 128); // Next power of 2

    let cache2 = create_cache::<String, u32>(256);
    assert_eq!(cache2.capacity(), 256); // Already power of 2
  }

  #[test]
  fn test_hash_collision_handling() {
    let cache = create_cache::<String, u32>(64);

    // Create keys that are likely to hash to similar values
    let keys: Vec<String> = (0..1000).map(|i| format!("collision_key_{}", i)).collect();

    // Insert all keys
    for (i, key) in keys.iter().enumerate() {
      assert!(cache.insert(key.clone(), i as u32).unwrap());
    }

    // Verify all keys can be retrieved (some may have been evicted due to ring buffer)
    let mut found_count = 0;
    for (i, key) in keys.iter().enumerate() {
      if let Some(value) = cache.get(key) {
        assert_eq!(value, i as u32);
        found_count += 1;
      }
    }

    assert!(found_count > 50); // Should find at least some keys
  }

  #[test]
  fn test_key_overwrite_behavior() {
    let cache = create_cache::<String, u64>(100);

    // Insert initial value
    assert!(cache.insert("overwrite_key".to_string(), 100u64).unwrap());
    assert_eq!(cache.get(&"overwrite_key".to_string()), Some(100u64));

    // Overwrite with new value
    assert!(cache.insert("overwrite_key".to_string(), 200u64).unwrap());
    assert_eq!(cache.get(&"overwrite_key".to_string()), Some(200u64));

    // Overwrite multiple times rapidly
    for i in 300..400 {
      assert!(cache.insert("overwrite_key".to_string(), i).unwrap());
    }

    // Should have the last value
    if let Some(final_value) = cache.get(&"overwrite_key".to_string()) {
      assert!((300..400).contains(&final_value));
    }
  }

  #[test]
  fn test_memory_ordering_consistency() {
    let cache = create_cache::<String, u64>(1000);
    let inconsistency_detected = Arc::new(AtomicBool::new(false));

    // Writer thread that maintains sequence
    let writer_cache = Arc::clone(&cache);
    let writer_handle = thread::spawn(move || {
      for i in 0..10000u64 {
        writer_cache.insert(format!("seq_{}", i % 100), i).unwrap();
        if i % 1000 == 0 {
          thread::sleep(Duration::from_micros(1));
        }
      }
    });

    // Reader threads that check for consistency
    let readers: Vec<_> = (0..4)
      .map(|_| {
        let cache = Arc::clone(&cache);
        let inconsistency = Arc::clone(&inconsistency_detected);

        thread::spawn(move || {
          let mut last_seen = vec![0u64; 100];

          for _ in 0..5000 {
            for key_idx in 0..100 {
              let key = format!("seq_{}", key_idx);
              if let Some(value) = cache.get(&key) {
                if value < last_seen[key_idx] {
                  // Should never see a value go backwards (except for wraparound)
                  if last_seen[key_idx] - value > 5000 {
                    inconsistency.store(true, Ordering::Relaxed);
                  }
                }
                last_seen[key_idx] = value;
              }
            }

            if fastrand::u32(0..100) == 0 {
              thread::sleep(Duration::from_micros(1));
            }
          }
        })
      })
      .collect();

    writer_handle.join().unwrap();
    for reader in readers {
      reader.join().unwrap();
    }

    assert!(
      !inconsistency_detected.load(Ordering::Relaxed),
      "Memory ordering inconsistency detected!"
    );
  }

  #[test]
  fn test_large_value_fallback_path() {
    let cache = create_cache::<String, LargeTestStruct>(100);

    let large_val = LargeTestStruct::new(12345);
    assert!(large_val.is_valid());

    assert!(cache.insert("large_key".to_string(), large_val).unwrap());

    if let Some(retrieved) = cache.get(&"large_key".to_string()) {
      assert_eq!(retrieved.id, 12345);
      assert!(retrieved.is_valid());
      assert_eq!(retrieved, large_val);
    } else {
      panic!("Failed to retrieve large value");
    }

    // Test concurrent access to large values
    let handles: Vec<_> = (0..5)
      .map(|thread_id| {
        let cache = Arc::clone(&cache);
        thread::spawn(move || {
          for i in 0..100 {
            let key = format!("large_{}_{}", thread_id, i);
            let val = LargeTestStruct::new((thread_id * 100 + i) as u64);

            cache.insert(key.clone(), val).unwrap();

            if let Some(retrieved) = cache.get(&key) {
              assert!(retrieved.is_valid());
              assert_eq!(retrieved.id, (thread_id * 100 + i) as u64);
            }
          }
        })
      })
      .collect();

    for handle in handles {
      handle.join().unwrap();
    }
  }

  #[test]
  fn test_remove_and_tombstone_behavior() {
    let cache = create_cache::<String, SessionData>(100);

    // Insert some test data
    let sessions = vec![
      (
        "user1",
        SessionData {
          user_id: 1,
          session_token: 0xAAA,
          permissions: 0xFF,
          login_count: 5,
        },
      ),
      (
        "user2",
        SessionData {
          user_id: 2,
          session_token: 0xBBB,
          permissions: 0x0F,
          login_count: 2,
        },
      ),
      (
        "user3",
        SessionData {
          user_id: 3,
          session_token: 0xCCC,
          permissions: 0xF0,
          login_count: 10,
        },
      ),
    ];

    for (key, session) in &sessions {
      assert!(cache.insert(key.to_string(), *session).unwrap());
    }

    assert_eq!(cache.len(), 3);

    // Remove middle entry
    let removed = cache.remove(&"user2".to_string());
    assert_eq!(removed, Some(sessions[1].1));
    assert_eq!(cache.len(), 2);

    // Verify other entries still exist
    assert_eq!(cache.get(&"user1".to_string()), Some(sessions[0].1));
    assert_eq!(cache.get(&"user3".to_string()), Some(sessions[2].1));
    assert_eq!(cache.get(&"user2".to_string()), None);

    // Re-insert into tombstone slot
    let new_session = SessionData {
      user_id: 4,
      session_token: 0xDDD,
      permissions: 0xFF,
      login_count: 1,
    };
    assert!(cache.insert("user2".to_string(), new_session).unwrap());
    assert_eq!(cache.get(&"user2".to_string()), Some(new_session));
    assert_eq!(cache.len(), 3);
  }

  #[test]
  fn test_retain_functionality_comprehensive() {
    let cache = create_cache::<String, SessionData>(200);

    // Insert varied test data
    for i in 0..150 {
      let session = SessionData {
        user_id: i as u32,
        session_token: 0x1000 + i as u32,
        permissions: (i % 4) as u16 * 0x0F,
        login_count: (i % 20) as u16,
      };
      cache.insert(format!("user_{}", i), session).unwrap();
    }

    let initial_count = cache.len();

    // Retain only active users (login_count > 5)
    let removed_inactive = cache.retain(|session, _ts, _seq| session.login_count > 5);
    let after_activity_filter = cache.len();

    assert!(removed_inactive > 0);
    assert_eq!(initial_count, after_activity_filter + removed_inactive);

    // Retain only privileged users (permissions > 0)
    let removed_no_perms = cache.retain(|session, _ts, _seq| session.permissions > 0);
    let after_perms_filter = cache.len();

    assert_eq!(after_activity_filter, after_perms_filter + removed_no_perms);

    // Verify remaining entries meet criteria
    let all_remaining = cache.get_all();
    for (_, session, _, _) in all_remaining {
      assert!(session.login_count > 5);
      assert!(session.permissions > 0);
    }
  }

  #[test]
  fn test_metadata_timestamp_ordering() {
    let cache = create_cache::<String, u32>(100);

    let mut insert_order = Vec::new();

    // Insert entries with deliberate delays to ensure different timestamps
    for i in 0..10 {
      let key = format!("timed_key_{}", i);
      cache.insert(key.clone(), i as u32).unwrap();
      insert_order.push(key);
      thread::sleep(Duration::from_millis(2)); // Ensure different timestamps
    }

    // Get all entries and verify timestamp ordering
    let all_entries = cache.get_all();
    assert!(!all_entries.is_empty());

    // Should be sorted by sequence number (newest first)
    for i in 1..all_entries.len() {
      assert!(
        all_entries[i - 1].3 >= all_entries[i].3,
        "Entries should be ordered by sequence number"
      );
    }

    // Check metadata consistency
    for (i, key) in insert_order.iter().enumerate() {
      if let Some((value, timestamp, sequence)) = cache.get_with_metadata(key) {
        assert_eq!(value, i as u32);
        assert!(timestamp > 0);
        assert!(sequence > 0);
      }
    }
  }

  #[test]
  fn test_cache_overflow_ring_buffer() {
    let small_cache = create_cache::<String, u64>(32); // Small cache to force overflow

    // Fill way beyond capacity
    let overflow_factor = 5;
    let total_inserts = small_cache.capacity() * overflow_factor;

    for i in 0..total_inserts {
      small_cache
        .insert(format!("overflow_{}", i), i as u64)
        .unwrap();
    }

    // Cache should not exceed capacity
    assert!(small_cache.len() <= small_cache.capacity());

    // Most recent entries should still be findable
    let mut recent_found = 0;
    let recent_range = total_inserts - small_cache.capacity()..total_inserts;

    for i in recent_range {
      if small_cache.get(&format!("overflow_{}", i)).is_some() {
        recent_found += 1;
      }
    }

    assert!(recent_found > small_cache.capacity() / 4); // Should find reasonable portion

    // Older entries should mostly be gone
    let mut old_found = 0;
    for i in 0..small_cache.capacity() {
      if small_cache.get(&format!("overflow_{}", i)).is_some() {
        old_found += 1;
      }
    }

    assert!(old_found < small_cache.capacity() / 2); // Most old entries should be gone
  }

  #[test]
  fn test_edge_case_empty_and_invalid_keys() {
    let cache = create_cache::<String, u32>(100);

    // Test empty string key
    assert!(cache.insert("".to_string(), 42u32).unwrap());
    assert_eq!(cache.get(&"".to_string()), Some(42u32));

    // Test very long key
    let long_key = "a".repeat(1000);
    assert!(cache.insert(long_key.clone(), 123u32).unwrap());
    assert_eq!(cache.get(&long_key), Some(123u32));

    // Test special characters in keys
    let special_keys = [
      "key with spaces",
      "key\nwith\nnewlines",
      "key\twith\ttabs",
      "key-with-dashes",
      "key_with_underscores",
      "key.with.dots",
      "key/with/slashes",
      "key\\with\\backslashes",
      "🦀🔥 emoji key 🚀",
    ];

    for (i, key) in special_keys.iter().enumerate() {
      assert!(cache.insert(key.to_string(), i as u32).unwrap());
      assert_eq!(cache.get(&key.to_string()), Some(i as u32));
    }
  }

  #[test]
  fn test_deterministic_behavior_same_input() {
    // Test that same operations produce consistent results
    for _run in 0..3 {
      let cache = create_cache::<String, u64>(100);

      // Same sequence of operations
      let operations = vec![
        ("key1", 100u64),
        ("key2", 200u64),
        ("key3", 300u64),
        ("key1", 150u64), // Overwrite
        ("key4", 400u64),
      ];

      for (key, value) in &operations {
        cache.insert(key.to_string(), *value).unwrap();
      }

      // Results should be consistent across runs
      assert_eq!(cache.get(&"key1".to_string()), Some(150u64));
      assert_eq!(cache.get(&"key2".to_string()), Some(200u64));
      assert_eq!(cache.get(&"key3".to_string()), Some(300u64));
      assert_eq!(cache.get(&"key4".to_string()), Some(400u64));
      assert_eq!(cache.get(&"nonexistent".to_string()), None);
    }
  }

  #[test]
  fn test_drain() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add test data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());
    assert!(cache
      .insert("key2".to_string(), "value2".to_string())
      .unwrap());
    assert!(cache
      .insert("key3".to_string(), "value3".to_string())
      .unwrap());

    assert_eq!(cache.len(), 3);

    // Drain the cache
    let drained_entries = cache.drain();

    // Verify cache is empty after drain
    assert!(cache.is_empty());
    assert_eq!(cache.len(), 0);

    // Verify all entries were drained
    assert_eq!(drained_entries.len(), 3);

    // Extract key hashes and values for verification
    let mut drained_values: Vec<String> = drained_entries
      .iter()
      .map(|(_, value, _, _)| value.clone())
      .collect();
    drained_values.sort();

    let mut expected_values = vec![
      "value1".to_string(),
      "value2".to_string(),
      "value3".to_string(),
    ];
    expected_values.sort();

    assert_eq!(drained_values, expected_values);

    // Verify entries have metadata
    for (key_hash, _, timestamp, sequence) in &drained_entries {
      assert_ne!(*key_hash, 0); // Key hash should be non-zero
      assert_ne!(*timestamp, 0); // Timestamp should be set
      assert_ne!(*sequence, 0); // Sequence should be set
    }
  }

  #[test]
  fn test_drain_empty_cache() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    let drained_entries = cache.drain();

    assert!(drained_entries.is_empty());
    assert!(cache.is_empty());
  }

  #[test]
  fn test_drain_ordering_by_sequence() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Insert entries with forced time gaps
    assert!(cache
      .insert("first".to_string(), "value1".to_string())
      .unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100)); // time gap

    assert!(cache
      .insert("second".to_string(), "value2".to_string())
      .unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100)); // time gap

    assert!(cache
      .insert("third".to_string(), "value3".to_string())
      .unwrap());

    let drained_entries = cache.drain();

    // Verify we got all entries
    assert_eq!(drained_entries.len(), 3);

    let mut values: Vec<String> = drained_entries
      .iter()
      .map(|(_, value, _, _)| value.clone())
      .collect();
    values.sort();

    let mut expected_values = vec![
      "value1".to_string(),
      "value2".to_string(),
      "value3".to_string(),
    ];
    expected_values.sort();

    assert_eq!(values, expected_values);

    // Verify all entries have valid metadata
    for (key_hash, _, timestamp, sequence) in &drained_entries {
      assert_ne!(*key_hash, 0, "Key hash should be non-zero");
      assert_ne!(*timestamp, 0, "Timestamp should be non-zero");
      assert_ne!(*sequence, 0, "Sequence should be non-zero");
    }

    // Verify cache is empty after drain
    assert!(cache.is_empty());
  }

  #[test]
  fn test_entries() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add test data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());
    assert!(cache
      .insert("key2".to_string(), "value2".to_string())
      .unwrap());
    assert!(cache
      .insert("key3".to_string(), "value3".to_string())
      .unwrap());

    let entries = cache.entries();

    // Verify we get all entries
    assert_eq!(entries.len(), 3);

    // Extract values for comparison
    let mut values: Vec<String> = entries.iter().map(|(_, value)| value.clone()).collect();
    values.sort();

    let mut expected_values = vec![
      "value1".to_string(),
      "value2".to_string(),
      "value3".to_string(),
    ];
    expected_values.sort();

    assert_eq!(values, expected_values);

    // Verify all key hashes are non-zero
    for (key_hash, _) in &entries {
      assert_ne!(*key_hash, 0);
    }

    // Verify cache is unchanged
    assert_eq!(cache.len(), 3);
  }

  #[test]
  fn test_entries_empty_cache() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    let entries = cache.entries();

    assert!(entries.is_empty());
  }

  #[test]
  fn test_remove_existing() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add test data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());
    assert!(cache
      .insert("key2".to_string(), "value2".to_string())
      .unwrap());
    assert!(cache
      .insert("key3".to_string(), "value3".to_string())
      .unwrap());

    // Remove mix of existing and non-existing keys
    let keys = vec![
      "key1".to_string(),
      "nonexistent".to_string(),
      "key3".to_string(),
      "another_missing".to_string(),
    ];
    let removed_values = cache.remove_existing(&keys);

    // Should only return values that were actually removed
    assert_eq!(removed_values.len(), 2);

    let mut sorted_removed = removed_values.clone();
    sorted_removed.sort();

    let mut expected = vec!["value1".to_string(), "value3".to_string()];
    expected.sort();

    assert_eq!(sorted_removed, expected);

    // Verify cache state
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get(&"key2".to_string()), Some("value2".to_string()));
    assert_eq!(cache.get(&"key1".to_string()), None);
    assert_eq!(cache.get(&"key3".to_string()), None);
  }

  #[test]
  fn test_remove_existing_all_missing() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add some data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());

    // Try to remove only non-existing keys
    let keys = vec![
      "missing1".to_string(),
      "missing2".to_string(),
      "missing3".to_string(),
    ];
    let removed_values = cache.remove_existing(&keys);

    assert!(removed_values.is_empty());
    assert_eq!(cache.len(), 1); // Original data should remain
  }

  #[test]
  fn test_remove_existing_empty_keys() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());

    let removed_values = cache.remove_existing(&Vec::<String>::new());

    assert!(removed_values.is_empty());
    assert_eq!(cache.len(), 1);
  }

  #[test]
  fn test_remove_all() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add test data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());
    assert!(cache
      .insert("key2".to_string(), "value2".to_string())
      .unwrap());
    assert!(cache
      .insert("key3".to_string(), "value3".to_string())
      .unwrap());

    // Remove mix of existing and non-existing keys
    let keys = vec![
      "key1".to_string(),
      "nonexistent".to_string(),
      "key3".to_string(),
      "another_missing".to_string(),
    ];
    let removed_values = cache.remove_all(&keys);

    // Should return same length as input, with None for missing keys
    assert_eq!(removed_values.len(), 4);
    assert_eq!(removed_values[0], Some("value1".to_string())); // key1
    assert_eq!(removed_values[1], None); // nonexistent
    assert_eq!(removed_values[2], Some("value3".to_string())); // key3
    assert_eq!(removed_values[3], None); // another_missing

    // Verify cache state
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get(&"key2".to_string()), Some("value2".to_string()));
  }

  #[test]
  fn test_remove_all_preserve_order() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    assert!(cache
      .insert("a".to_string(), "value_a".to_string())
      .unwrap());
    assert!(cache
      .insert("b".to_string(), "value_b".to_string())
      .unwrap());
    assert!(cache
      .insert("c".to_string(), "value_c".to_string())
      .unwrap());

    let keys = vec![
      "c".to_string(),
      "a".to_string(),
      "missing".to_string(),
      "b".to_string(),
    ];
    let removed_values = cache.remove_all(&keys);

    // Verify order is preserved
    assert_eq!(removed_values.len(), 4);
    assert_eq!(removed_values[0], Some("value_c".to_string()));
    assert_eq!(removed_values[1], Some("value_a".to_string()));
    assert_eq!(removed_values[2], None);
    assert_eq!(removed_values[3], Some("value_b".to_string()));
  }

  #[test]
  fn test_remove_all_empty_cache() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    let keys = vec!["key1".to_string(), "key2".to_string()];
    let removed_values = cache.remove_all(&keys);

    assert_eq!(removed_values.len(), 2);
    assert_eq!(removed_values[0], None);
    assert_eq!(removed_values[1], None);
  }

  #[test]
  fn test_contains_all_keys() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add test data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());
    assert!(cache
      .insert("key2".to_string(), "value2".to_string())
      .unwrap());
    assert!(cache
      .insert("key3".to_string(), "value3".to_string())
      .unwrap());

    // Test with all existing keys
    let all_existing = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
    assert!(cache.contains_all_keys(&all_existing));

    // Test with subset of existing keys
    let subset_existing = vec!["key1".to_string(), "key3".to_string()];
    assert!(cache.contains_all_keys(&subset_existing));

    // Test with one missing key
    let one_missing = vec![
      "key1".to_string(),
      "key2".to_string(),
      "missing".to_string(),
    ];
    assert!(!cache.contains_all_keys(&one_missing));

    // Test with all missing keys
    let all_missing = vec!["missing1".to_string(), "missing2".to_string()];
    assert!(!cache.contains_all_keys(&all_missing));

    // Test with empty vector (should return true)
    let empty_keys: Vec<String> = vec![];
    assert!(cache.contains_all_keys(&empty_keys));
  }

  #[test]
  fn test_contains_all_keys_empty_cache() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Empty cache with any non-empty key list should return false
    let keys = vec!["key1".to_string()];
    assert!(!cache.contains_all_keys(&keys));

    // Empty cache with empty key list should return true
    let empty_keys: Vec<String> = vec![];
    assert!(cache.contains_all_keys(&empty_keys));
  }

  #[test]
  fn test_contains_any_key() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add test data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());
    assert!(cache
      .insert("key2".to_string(), "value2".to_string())
      .unwrap());
    assert!(cache
      .insert("key3".to_string(), "value3".to_string())
      .unwrap());

    // Test with all existing keys
    let all_existing = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
    assert!(cache.contains_any_key(&all_existing));

    // Test with one existing key
    let one_existing = vec![
      "missing1".to_string(),
      "key2".to_string(),
      "missing2".to_string(),
    ];
    assert!(cache.contains_any_key(&one_existing));

    // Test with no existing keys
    let no_existing = vec![
      "missing1".to_string(),
      "missing2".to_string(),
      "missing3".to_string(),
    ];
    assert!(!cache.contains_any_key(&no_existing));

    // Test with empty vector (should return false)
    let empty_keys: Vec<String> = vec![];
    assert!(!cache.contains_any_key(&empty_keys));
  }

  #[test]
  fn test_contains_any_key_empty_cache() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Empty cache should return false for any key list
    let keys = vec!["key1".to_string(), "key2".to_string()];
    assert!(!cache.contains_any_key(&keys));

    let empty_keys: Vec<String> = vec![];
    assert!(!cache.contains_any_key(&empty_keys));
  }

  #[test]
  fn test_get_all_by_keys() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add test data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());
    assert!(cache
      .insert("key2".to_string(), "value2".to_string())
      .unwrap());
    assert!(cache
      .insert("key3".to_string(), "value3".to_string())
      .unwrap());

    // Test with mix of existing and non-existing keys
    let keys = vec![
      "key1".to_string(),
      "missing".to_string(),
      "key3".to_string(),
      "another_missing".to_string(),
      "key2".to_string(),
    ];
    let values = cache.get_all_by_keys(&keys);

    // Should return values in same order as keys, with None for missing
    assert_eq!(values.len(), 5);
    assert_eq!(values[0], Some("value1".to_string())); // key1
    assert_eq!(values[1], None); // missing
    assert_eq!(values[2], Some("value3".to_string())); // key3
    assert_eq!(values[3], None); // another_missing
    assert_eq!(values[4], Some("value2".to_string())); // key2
  }

  #[test]
  fn test_get_all_by_keys_preserve_order() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    assert!(cache
      .insert("a".to_string(), "value_a".to_string())
      .unwrap());
    assert!(cache
      .insert("b".to_string(), "value_b".to_string())
      .unwrap());
    assert!(cache
      .insert("c".to_string(), "value_c".to_string())
      .unwrap());

    // Test different orderings
    let keys1 = vec!["c".to_string(), "a".to_string(), "b".to_string()];
    let values1 = cache.get_all_by_keys(&keys1);
    assert_eq!(values1[0], Some("value_c".to_string()));
    assert_eq!(values1[1], Some("value_a".to_string()));
    assert_eq!(values1[2], Some("value_b".to_string()));

    let keys2 = vec!["b".to_string(), "c".to_string(), "a".to_string()];
    let values2 = cache.get_all_by_keys(&keys2);
    assert_eq!(values2[0], Some("value_b".to_string()));
    assert_eq!(values2[1], Some("value_c".to_string()));
    assert_eq!(values2[2], Some("value_a".to_string()));
  }

  #[test]
  fn test_get_all_by_keys_empty_input() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());

    let empty_keys: Vec<String> = vec![];
    let values = cache.get_all_by_keys(&empty_keys);

    assert!(values.is_empty());
  }

  #[test]
  fn test_get_all_by_keys_empty_cache() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    let keys = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
    let values = cache.get_all_by_keys(&keys);

    assert_eq!(values.len(), 3);
    assert_eq!(values[0], None);
    assert_eq!(values[1], None);
    assert_eq!(values[2], None);
  }

  #[test]
  fn test_get_all_by_keys_duplicate_keys() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());

    // Test with duplicate keys
    let keys = vec![
      "key1".to_string(),
      "key1".to_string(),
      "missing".to_string(),
      "key1".to_string(),
    ];
    let values = cache.get_all_by_keys(&keys);

    assert_eq!(values.len(), 4);
    assert_eq!(values[0], Some("value1".to_string()));
    assert_eq!(values[1], Some("value1".to_string()));
    assert_eq!(values[2], None);
    assert_eq!(values[3], Some("value1".to_string()));
  }

  #[test]
  fn test_concurrent_drain() {
    let cache = Arc::new(AtomicGenericCache::<String, String>::new(256, 0));

    // Populate cache
    for i in 0..100 {
      assert!(cache
        .insert(format!("key{}", i), format!("value{}", i))
        .unwrap());
    }

    let initial_len = cache.len();
    assert_eq!(initial_len, 100);

    // Concurrent drain operations
    let mut handles = vec![];

    for _ in 0..3 {
      let cache_clone = Arc::clone(&cache);
      let handle = thread::spawn(move || cache_clone.drain());
      handles.push(handle);
    }

    let mut all_drained = vec![];
    for handle in handles {
      let drained = handle.join().unwrap();
      all_drained.extend(drained);
    }

    // Cache should be empty or nearly empty
    assert!(cache.len() < initial_len);

    // Should have drained some entries
    assert!(!all_drained.is_empty());

    // Verify no duplicate entries were drained
    let mut key_hashes: Vec<u64> = all_drained.iter().map(|(hash, _, _, _)| *hash).collect();
    key_hashes.sort();
    let original_len = key_hashes.len();
    key_hashes.dedup();
    assert_eq!(
      key_hashes.len(),
      original_len,
      "Duplicate entries were drained"
    );
  }

  #[test]
  fn test_into_hashmap() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add test data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());
    assert!(cache
      .insert("key2".to_string(), "value2".to_string())
      .unwrap());
    assert!(cache
      .insert("key3".to_string(), "value3".to_string())
      .unwrap());

    assert_eq!(cache.len(), 3);

    // Convert to HashMap (this drains the cache)
    let hashmap = cache.into_hashmap();

    // Verify conversion
    assert_eq!(hashmap.len(), 3);

    // Verify values are present (we can't easily verify keys since they're hashed)
    let values: HashSet<String> = hashmap.values().cloned().collect();
    let expected_values: HashSet<String> = ["value1", "value2", "value3"]
      .iter()
      .map(|s| s.to_string())
      .collect();

    assert_eq!(values, expected_values);
  }

  #[test]
  fn test_to_hashmap() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add test data
    assert!(cache
      .insert("key1".to_string(), "value1".to_string())
      .unwrap());
    assert!(cache
      .insert("key2".to_string(), "value2".to_string())
      .unwrap());
    assert!(cache
      .insert("key3".to_string(), "value3".to_string())
      .unwrap());

    assert_eq!(cache.len(), 3);

    // Convert to HashMap (this should NOT drain the cache)
    let hashmap = cache.to_hashmap();

    // Verify conversion
    assert_eq!(hashmap.len(), 3);
    assert_eq!(cache.len(), 3); // Cache should still have entries

    // Verify values are present
    let values: HashSet<String> = hashmap.values().cloned().collect();
    let expected_values: HashSet<String> = ["value1", "value2", "value3"]
      .iter()
      .map(|s| s.to_string())
      .collect();

    assert_eq!(values, expected_values);
  }

  #[test]
  fn test_from_hashmap() {
    let mut hashmap = HashMap::new();
    hashmap.insert(12345u64, "value1".to_string());
    hashmap.insert(67890u64, "value2".to_string());
    hashmap.insert(11111u64, "value3".to_string());

    let cache = AtomicGenericCache::<u64, String>::from_hashmap(hashmap, 128);

    // Verify cache was populated
    assert_eq!(cache.len(), 3);

    // Convert back to verify data integrity
    let result_hashmap = cache.to_hashmap();
    assert_eq!(result_hashmap.len(), 3);

    // Verify specific values are present
    let values: HashSet<String> = result_hashmap.values().cloned().collect();
    let expected_values: HashSet<String> = ["value1", "value2", "value3"]
      .iter()
      .map(|s| s.to_string())
      .collect();

    assert_eq!(values, expected_values);
  }

  #[test]
  fn test_from_hashmap_with_keys() {
    let mut hashmap = HashMap::new();
    hashmap.insert("key1".to_string(), "value1".to_string());
    hashmap.insert("key2".to_string(), "value2".to_string());
    hashmap.insert("key3".to_string(), "value3".to_string());

    let cache = AtomicGenericCache::from_hashmap_with_keys(hashmap, 128).unwrap();

    // Verify cache was populated
    assert_eq!(cache.len(), 3);

    // Verify we can retrieve values using original keys
    assert_eq!(cache.get(&"key1".to_string()), Some("value1".to_string()));
    assert_eq!(cache.get(&"key2".to_string()), Some("value2".to_string()));
    assert_eq!(cache.get(&"key3".to_string()), Some("value3".to_string()));
    assert_eq!(cache.get(&"nonexistent".to_string()), None);
  }

  #[test]
  fn test_merge_from_hashmap() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Add initial data
    assert!(cache
      .insert("existing".to_string(), "old_value".to_string())
      .unwrap());
    assert!(cache
      .insert("keep".to_string(), "keep_value".to_string())
      .unwrap());

    // Create HashMap to merge
    let mut hashmap = HashMap::new();
    hashmap.insert("existing".to_string(), "new_value".to_string()); // Should overwrite
    hashmap.insert("new_key".to_string(), "new_value".to_string()); // Should add
    hashmap.insert("another".to_string(), "another_value".to_string()); // Should add

    let inserted_count = cache.merge_from_hashmap(hashmap).unwrap();

    // All entries should be successfully inserted
    assert_eq!(inserted_count, 3);
    assert_eq!(cache.len(), 4); // 2 original + 1 overwrite + 2 new = 4 total

    // Verify values
    assert_eq!(
      cache.get(&"existing".to_string()),
      Some("new_value".to_string())
    ); // Overwritten
    assert_eq!(
      cache.get(&"keep".to_string()),
      Some("keep_value".to_string())
    ); // Preserved
    assert_eq!(
      cache.get(&"new_key".to_string()),
      Some("new_value".to_string())
    ); // Added
    assert_eq!(
      cache.get(&"another".to_string()),
      Some("another_value".to_string())
    ); // Added
  }

  #[test]
  fn test_compute_key_hash() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    let hash1 = cache.compute_key_hash(&"key1".to_string());
    let hash2 = cache.compute_key_hash(&"key2".to_string());
    let hash1_again = cache.compute_key_hash(&"key1".to_string());

    // Different keys should have different hashes
    assert_ne!(hash1, hash2);

    // Same key should always produce same hash
    assert_eq!(hash1, hash1_again);

    // Hashes should be non-zero
    assert_ne!(hash1, 0);
    assert_ne!(hash2, 0);
  }

  #[test]
  fn test_empty_cache_conversions() {
    let cache = AtomicGenericCache::<String, String>::new(64, 0);

    // Test to_hashmap with empty cache
    let hashmap = cache.to_hashmap();
    assert!(hashmap.is_empty());

    // Test into_hashmap with empty cache
    let hashmap2 = cache.into_hashmap();
    assert!(hashmap2.is_empty());

    // Test from_hashmap with empty HashMap
    let empty_hashmap: HashMap<u64, String> = HashMap::new();
    let cache2 = AtomicGenericCache::<u64, String>::from_hashmap(empty_hashmap, 64);
    assert!(cache2.is_empty());

    // Test from_hashmap_with_keys with empty HashMap
    let empty_hashmap2: HashMap<&str, String> = HashMap::new();
    let cache3 = AtomicGenericCache::from_hashmap_with_keys(empty_hashmap2, 64).unwrap();
    assert!(cache3.is_empty());
  }

  #[test]
  fn test_large_dataset_conversion() {
    let cache = AtomicGenericCache::<String, i32>::new(512, 0);

    // Insert many items
    for i in 0..100 {
      assert!(cache.insert(format!("key_{}", i), i).unwrap());
    }

    assert_eq!(cache.len(), 100);

    // Convert to HashMap
    let hashmap = cache.to_hashmap();
    assert_eq!(hashmap.len(), 100);

    // Verify all values are present
    let values: HashSet<i32> = hashmap.values().cloned().collect();
    let expected_values: HashSet<i32> = (0..100).collect();
    assert_eq!(values, expected_values);

    // Cache should still be intact
    assert_eq!(cache.len(), 100);
  }

  #[test]
  fn test_clear() {
    let cache = AtomicGenericCache::<String, i32>::new(512, 0);

    // Insert many items
    for i in 0..100 {
      assert!(cache.insert(format!("key_{}", i), i).unwrap());
    }

    assert_eq!(cache.len(), 100);

    cache.clear();

    assert_eq!(cache.len(), 0);
  }

  #[test]
  fn test_capacity() {
    let cache = AtomicGenericCache::<String, i32>::new(1000, 512);

    // Insert many items
    for i in 0..1000 {
      _ = cache.insert(format!("key_{}", i), i);
    }

    assert_eq!(cache.len(), 512);
  }
}
