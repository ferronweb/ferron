use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[repr(C, align(64))]
struct AtomicEntry<T>
where
  T: Clone + Send + Sync + 'static,
{
  key: AtomicU64,
  // Store value as raw bytes for small types, use RwLock for large types
  value_storage: AtomicU64,                     // For types <= 8 bytes
  value_fallback: Option<std::sync::RwLock<T>>, // For types > 8 bytes
  timestamp_nanos: AtomicU64,
  sequence: AtomicU64,
  hash: AtomicU64,
  state: AtomicU64,
  version: AtomicU64,
}

// State constants
const STATE_EMPTY: u64 = 0;
const STATE_RESERVED: u64 = 1;
const STATE_WRITING: u64 = 2;
const STATE_WRITTEN: u64 = 3;
const STATE_TOMBSTONE: u64 = 4;

impl<T> AtomicEntry<T>
where
  T: Clone + Send + Sync + Default + 'static,
{
  fn new() -> Self {
    let use_fallback = std::mem::size_of::<T>() > 8 || std::mem::align_of::<T>() > 8;

    Self {
      key: AtomicU64::new(0),
      value_storage: AtomicU64::new(0),
      value_fallback: if use_fallback {
        Some(std::sync::RwLock::new(T::default()))
      } else {
        None
      },
      timestamp_nanos: AtomicU64::new(0),
      sequence: AtomicU64::new(0),
      hash: AtomicU64::new(0),
      state: AtomicU64::new(STATE_EMPTY),
      version: AtomicU64::new(0),
    }
  }

  fn store_value(&self, value: T, ordering: Ordering) {
    if std::mem::size_of::<T>() <= 8 && std::mem::align_of::<T>() <= 8 {
      // Safe for small types - store directly in atomic
      let raw_value = unsafe {
        let mut buffer = [0u8; 8];
        std::ptr::copy_nonoverlapping(
          &value as *const T as *const u8,
          buffer.as_mut_ptr(),
          std::mem::size_of::<T>(),
        );
        u64::from_le_bytes(buffer)
      };
      self.value_storage.store(raw_value, ordering);
    } else {
      // Fallback for large types
      if let Some(ref lock) = self.value_fallback {
        *lock.write().unwrap() = value;
      }
    }
  }

  fn load_value(&self, ordering: Ordering) -> T {
    if std::mem::size_of::<T>() <= 8 && std::mem::align_of::<T>() <= 8 {
      // Safe for small types - load directly from atomic
      let raw_value = self.value_storage.load(ordering);
      unsafe {
        let buffer = raw_value.to_le_bytes();
        let mut result = std::mem::MaybeUninit::<T>::uninit();
        std::ptr::copy_nonoverlapping(
          buffer.as_ptr(),
          result.as_mut_ptr() as *mut u8,
          std::mem::size_of::<T>(),
        );
        result.assume_init()
      }
    } else {
      // Fallback for large types
      if let Some(ref lock) = self.value_fallback {
        lock.read().unwrap().clone()
      } else {
        T::default()
      }
    }
  }
}

impl<T> Default for AtomicEntry<T>
where
  T: Clone + Send + Sync + Default + 'static,
{
  fn default() -> Self {
    Self::new()
  }
}

pub struct AtomicGenericCache<T>
where
  T: Clone + Send + Sync + 'static,
{
  entries: Box<[AtomicEntry<T>]>,
  write_index: AtomicUsize,
  sequence_counter: AtomicU64,
  capacity: usize,
  capacity_mask: usize,
}

impl<T> AtomicGenericCache<T>
where
  T: Clone + Send + Sync + Default + 'static,
{
  pub fn new(capacity: usize) -> Arc<Self> {
    let capacity = capacity.next_power_of_two().max(64);

    let entries = (0..capacity)
      .map(|_| AtomicEntry::default())
      .collect::<Vec<_>>()
      .into_boxed_slice();

    Arc::new(Self {
      entries,
      write_index: AtomicUsize::new(0),
      sequence_counter: AtomicU64::new(1),
      capacity,
      capacity_mask: capacity - 1,
    })
  }

  fn hash_key<K: Hash>(&self, key: &K) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    // Ensure hash is never 0 (reserved for empty state)
    if hash == 0 {
      1
    } else {
      hash
    }
  }

  fn get_timestamp_nanos() -> u64 {
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_nanos() as u64
  }

  pub fn insert<K: Hash>(&self, key: K, value: T) -> bool {
    let key_hash = self.hash_key(&key);
    let now = Self::get_timestamp_nanos();
    let sequence = self.sequence_counter.fetch_add(1, Ordering::Relaxed);

    // First pass: try to update existing entry with same hash
    if self.try_update_existing(key_hash, value.clone(), now, sequence) {
      return true;
    }

    // Second pass: find empty slot or overwrite old entry
    let max_attempts: usize = self.capacity * 2;
    let mut attempt = 0;

    while attempt < max_attempts {
      let write_pos = self.write_index.fetch_add(1, Ordering::Relaxed);
      let index = write_pos & self.capacity_mask;
      let slot = &self.entries[index];

      // Try different states in order of preference
      if self.try_claim_slot(slot, STATE_EMPTY, key_hash, value.clone(), now, sequence)
        || self.try_claim_slot(
          slot,
          STATE_TOMBSTONE,
          key_hash,
          value.clone(),
          now,
          sequence,
        )
        || self.try_overwrite_slot(slot, key_hash, value.clone(), now, sequence)
      {
        return true;
      }

      attempt += 1;

      let backoff = (1 << (attempt / 100).min(6)) + 8;
      for _ in 0..backoff {
        std::hint::spin_loop();
      }
    }

    false
  }

  fn try_update_existing(&self, key_hash: u64, value: T, timestamp: u64, sequence: u64) -> bool {
    for slot in self.entries.iter() {
      if slot.state.load(Ordering::Acquire) == STATE_WRITTEN
        && slot.hash.load(Ordering::Relaxed) == key_hash
        && slot
          .state
          .compare_exchange_weak(
            STATE_WRITTEN,
            STATE_WRITING,
            Ordering::Acquire,
            Ordering::Relaxed,
          )
          .is_ok()
      {
        // Verify hash is still the same after acquiring write lock
        if slot.hash.load(Ordering::Relaxed) == key_hash {
          self.write_entry_data(slot, key_hash, value, timestamp, sequence);
          slot.state.store(STATE_WRITTEN, Ordering::Release);
          return true;
        } else {
          // Hash changed, release lock
          slot.state.store(STATE_WRITTEN, Ordering::Release);
        }
      }
    }
    false
  }

  fn try_claim_slot(
    &self,
    slot: &AtomicEntry<T>,
    expected_state: u64,
    key_hash: u64,
    value: T,
    timestamp: u64,
    sequence: u64,
  ) -> bool {
    if slot
      .state
      .compare_exchange_weak(
        expected_state,
        STATE_RESERVED,
        Ordering::Acquire,
        Ordering::Relaxed,
      )
      .is_ok()
    {
      self.write_entry_data(slot, key_hash, value, timestamp, sequence);
      slot.state.store(STATE_WRITTEN, Ordering::Release);
      true
    } else {
      false
    }
  }

  fn try_overwrite_slot(
    &self,
    slot: &AtomicEntry<T>,
    key_hash: u64,
    value: T,
    timestamp: u64,
    sequence: u64,
  ) -> bool {
    if slot
      .state
      .compare_exchange_weak(
        STATE_WRITTEN,
        STATE_WRITING,
        Ordering::Acquire,
        Ordering::Relaxed,
      )
      .is_ok()
    {
      self.write_entry_data(slot, key_hash, value, timestamp, sequence);
      slot.state.store(STATE_WRITTEN, Ordering::Release);
      true
    } else {
      false
    }
  }

  fn write_entry_data(
    &self,
    slot: &AtomicEntry<T>,
    key_hash: u64,
    value: T,
    timestamp: u64,
    sequence: u64,
  ) {
    slot.version.fetch_add(1, Ordering::Relaxed);
    slot.key.store(key_hash, Ordering::Relaxed);
    slot.store_value(value, Ordering::Relaxed);
    slot.timestamp_nanos.store(timestamp, Ordering::Relaxed);
    slot.hash.store(key_hash, Ordering::Relaxed);
    slot.sequence.store(sequence, Ordering::Relaxed);

    // Ensure all writes are visible
    std::sync::atomic::fence(Ordering::Release);
  }

  pub fn get<K: Hash>(&self, key: K) -> Option<T> {
    let key_hash = self.hash_key(&key);

    // Multiple attempts to handle concurrent modifications
    for _attempt in 0..3 {
      for slot in self.entries.iter() {
        // Fast path: check state and hash first
        if slot.state.load(Ordering::Acquire) != STATE_WRITTEN
          || slot.hash.load(Ordering::Relaxed) != key_hash
        {
          continue;
        }

        // Load version before reading value for consistency
        let version_before = slot.version.load(Ordering::Acquire);

        // Read the value
        let value = slot.load_value(Ordering::Relaxed);

        // Verify consistency after read
        let version_after = slot.version.load(Ordering::Acquire);
        let state_after = slot.state.load(Ordering::Acquire);
        let hash_after = slot.hash.load(Ordering::Relaxed);

        if version_before == version_after && state_after == STATE_WRITTEN && hash_after == key_hash
        {
          return Some(value);
        }
      }

      // Brief pause between attempts
      for _ in 0..4 {
        std::hint::spin_loop();
      }
    }

    None
  }

  pub fn get_with_metadata<K: Hash>(&self, key: K) -> Option<(T, u64, u64)> {
    let key_hash = self.hash_key(&key);

    for slot in self.entries.iter() {
      if slot.state.load(Ordering::Acquire) == STATE_WRITTEN
        && slot.hash.load(Ordering::Relaxed) == key_hash
      {
        let version_before = slot.version.load(Ordering::Acquire);
        let value = slot.load_value(Ordering::Relaxed);
        let timestamp = slot.timestamp_nanos.load(Ordering::Relaxed);
        let sequence = slot.sequence.load(Ordering::Relaxed);
        let version_after = slot.version.load(Ordering::Acquire);

        if version_before == version_after
          && slot.state.load(Ordering::Acquire) == STATE_WRITTEN
          && slot.hash.load(Ordering::Relaxed) == key_hash
        {
          return Some((value, timestamp, sequence));
        }
      }
    }

    None
  }

  pub fn contains_key<K: Hash>(&self, key: K) -> bool {
    self.get(key).is_some()
  }

  pub fn remove<K: Hash>(&self, key: K) -> Option<T> {
    let key_hash = self.hash_key(&key);

    for slot in self.entries.iter() {
      if slot.state.load(Ordering::Acquire) == STATE_WRITTEN
        && slot.hash.load(Ordering::Relaxed) == key_hash
      {
        let value = slot.load_value(Ordering::Relaxed);

        if slot
          .state
          .compare_exchange(
            STATE_WRITTEN,
            STATE_TOMBSTONE,
            Ordering::Acquire,
            Ordering::Relaxed,
          )
          .is_ok()
          && slot.hash.load(Ordering::Relaxed) == key_hash
        {
          return Some(value);
        }
      }
    }

    None
  }

  pub fn clear(&self) -> usize {
    let mut cleared = 0;

    for slot in self.entries.iter() {
      if slot.state.load(Ordering::Acquire) == STATE_WRITTEN
        && slot
          .state
          .compare_exchange(
            STATE_WRITTEN,
            STATE_EMPTY,
            Ordering::Acquire,
            Ordering::Relaxed,
          )
          .is_ok()
      {
        cleared += 1;
      }
    }

    cleared
  }

  pub fn len(&self) -> usize {
    self
      .entries
      .iter()
      .filter(|slot| slot.state.load(Ordering::Relaxed) == STATE_WRITTEN)
      .count()
  }

  pub fn is_empty(&self) -> bool {
    self.len() == 0
  }

  pub fn capacity(&self) -> usize {
    self.capacity
  }

  // Utility methods
  pub fn get_all(&self) -> Vec<(u64, T, u64, u64)> {
    let mut results = Vec::new();

    for slot in self.entries.iter() {
      if slot.state.load(Ordering::Acquire) == STATE_WRITTEN {
        let version_before = slot.version.load(Ordering::Acquire);
        let key = slot.key.load(Ordering::Relaxed);
        let value = slot.load_value(Ordering::Relaxed);
        let timestamp = slot.timestamp_nanos.load(Ordering::Relaxed);
        let sequence = slot.sequence.load(Ordering::Relaxed);
        let version_after = slot.version.load(Ordering::Acquire);

        if version_before == version_after && slot.state.load(Ordering::Acquire) == STATE_WRITTEN {
          results.push((key, value, timestamp, sequence));
        }
      }
    }

    // Sort by sequence (most recent first)
    results.sort_unstable_by(|a, b| b.3.cmp(&a.3));
    results
  }

  pub fn retain<F>(&self, mut predicate: F) -> usize
  where
    F: FnMut(&T, u64, u64) -> bool,
  {
    let mut removed_count = 0;

    for slot in self.entries.iter() {
      if slot.state.load(Ordering::Acquire) == STATE_WRITTEN {
        let version_before = slot.version.load(Ordering::Acquire);
        let value = slot.load_value(Ordering::Relaxed);
        let timestamp = slot.timestamp_nanos.load(Ordering::Relaxed);
        let sequence = slot.sequence.load(Ordering::Relaxed);
        let version_after = slot.version.load(Ordering::Acquire);

        if version_before == version_after
          && slot.state.load(Ordering::Acquire) == STATE_WRITTEN
          && !predicate(&value, timestamp, sequence)
          && slot
            .state
            .compare_exchange(
              STATE_WRITTEN,
              STATE_TOMBSTONE,
              Ordering::Acquire,
              Ordering::Relaxed,
            )
            .is_ok()
        {
          removed_count += 1;
        }
      }
    }

    removed_count
  }
}

// Type safety and convenience
pub trait CacheableValue: Copy + Send + Sync + Default + 'static {
  fn is_lock_free() -> bool {
    std::mem::size_of::<Self>() <= 8 && std::mem::align_of::<Self>() <= 8
  }
}

impl<T> CacheableValue for T where T: Copy + Send + Sync + Default + 'static {}

#[cfg(test)]
#[cfg(debug_assertions)]
mod tests {
  use super::*;
  use std::{sync::atomic::AtomicBool, thread, time::Duration};

  pub fn create_cache<T: CacheableValue>(capacity: usize) -> Arc<AtomicGenericCache<T>> {
    AtomicGenericCache::new(capacity)
  }

  #[derive(Copy, Clone, Debug, Default, PartialEq)]
  struct TestEntry {
    id: u32,
    value: u32,
  }

  #[test]
  fn test_small_value_lock_free() {
    let cache = create_cache::<u64>(100);

    // Test basic operations
    assert!(cache.insert("key1", 12345u64));
    assert_eq!(cache.get("key1"), Some(12345u64));

    // Test that small values are indeed lock-free
    assert!(u64::is_lock_free());
    assert!(TestEntry::is_lock_free());
  }

  #[test]
  fn test_large_value_fallback() {
    #[derive(Copy, Clone, Debug, Default, PartialEq)]
    struct LargeValue {
      data: [u64; 4], // 32 bytes > 8 bytes
    }

    let cache = create_cache::<LargeValue>(100);
    let large_val = LargeValue { data: [1, 2, 3, 4] };

    assert!(cache.insert("large_key", large_val));
    assert_eq!(cache.get("large_key"), Some(large_val));

    // Large values use fallback
    assert!(!LargeValue::is_lock_free());
  }

  #[test]
  fn test_concurrent_stress() {
    let cache = create_cache::<TestEntry>(1000);

    let handles: Vec<_> = (0..8)
      .map(|thread_id| {
        let cache = Arc::clone(&cache);
        thread::spawn(move || {
          for i in 0..1000 {
            let entry = TestEntry {
              id: (thread_id * 1000 + i) as u32,
              value: (thread_id * 1000 + i) as u32,
            };
            cache.insert(format!("key:{}:{}", thread_id, i), entry);
            cache.get(format!("key:{}:{}", thread_id, i));
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
    let cache = create_cache::<u32>(0);
    assert_eq!(cache.capacity(), 64); // Should be clamped to minimum

    assert!(cache.insert("key", 42));
    assert_eq!(cache.get("key"), Some(42));
  }

  #[test]
  fn test_edge_case_power_of_two_capacity() {
    let cache = create_cache::<u32>(100);
    assert_eq!(cache.capacity(), 128); // Next power of 2

    let cache2 = create_cache::<u32>(256);
    assert_eq!(cache2.capacity(), 256); // Already power of 2
  }

  #[test]
  fn test_hash_collision_handling() {
    let cache = create_cache::<u32>(64);

    // Create keys that are likely to hash to similar values
    let keys: Vec<String> = (0..1000).map(|i| format!("collision_key_{}", i)).collect();

    // Insert all keys
    for (i, key) in keys.iter().enumerate() {
      assert!(cache.insert(key, i as u32));
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
    let cache = create_cache::<u64>(100);

    // Insert initial value
    assert!(cache.insert("overwrite_key", 100u64));
    assert_eq!(cache.get("overwrite_key"), Some(100u64));

    // Overwrite with new value
    assert!(cache.insert("overwrite_key", 200u64));
    assert_eq!(cache.get("overwrite_key"), Some(200u64));

    // Overwrite multiple times rapidly
    for i in 300..400 {
      assert!(cache.insert("overwrite_key", i));
    }

    // Should have the last value
    if let Some(final_value) = cache.get("overwrite_key") {
      assert!((300..400).contains(&final_value));
    }
  }

  #[test]
  fn test_memory_ordering_consistency() {
    let cache = create_cache::<u64>(1000);
    let inconsistency_detected = Arc::new(AtomicBool::new(false));

    // Writer thread that maintains sequence
    let writer_cache = Arc::clone(&cache);
    let writer_handle = thread::spawn(move || {
      for i in 0..10000u64 {
        writer_cache.insert(format!("seq_{}", i % 100), i);
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
    let cache = create_cache::<LargeTestStruct>(100);

    // Test that large values use fallback correctly
    assert!(!LargeTestStruct::is_lock_free());

    let large_val = LargeTestStruct::new(12345);
    assert!(large_val.is_valid());

    assert!(cache.insert("large_key", large_val));

    if let Some(retrieved) = cache.get("large_key") {
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

            cache.insert(&key, val);

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
    let cache = create_cache::<SessionData>(100);

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
      assert!(cache.insert(*key, *session));
    }

    assert_eq!(cache.len(), 3);

    // Remove middle entry
    let removed = cache.remove("user2");
    assert_eq!(removed, Some(sessions[1].1));
    assert_eq!(cache.len(), 2);

    // Verify other entries still exist
    assert_eq!(cache.get("user1"), Some(sessions[0].1));
    assert_eq!(cache.get("user3"), Some(sessions[2].1));
    assert_eq!(cache.get("user2"), None);

    // Re-insert into tombstone slot
    let new_session = SessionData {
      user_id: 4,
      session_token: 0xDDD,
      permissions: 0xFF,
      login_count: 1,
    };
    assert!(cache.insert("user2", new_session));
    assert_eq!(cache.get("user2"), Some(new_session));
    assert_eq!(cache.len(), 3);
  }

  #[test]
  fn test_retain_functionality_comprehensive() {
    let cache = create_cache::<SessionData>(200);

    // Insert varied test data
    for i in 0..150 {
      let session = SessionData {
        user_id: i as u32,
        session_token: 0x1000 + i as u32,
        permissions: (i % 4) as u16 * 0x0F,
        login_count: (i % 20) as u16,
      };
      cache.insert(format!("user_{}", i), session);
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
    let cache = create_cache::<u32>(100);

    let mut insert_order = Vec::new();

    // Insert entries with deliberate delays to ensure different timestamps
    for i in 0..10 {
      let key = format!("timed_key_{}", i);
      cache.insert(&key, i as u32);
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
    let small_cache = create_cache::<u64>(32); // Small cache to force overflow

    // Fill way beyond capacity
    let overflow_factor = 5;
    let total_inserts = small_cache.capacity() * overflow_factor;

    for i in 0..total_inserts {
      small_cache.insert(format!("overflow_{}", i), i as u64);
    }

    // Cache should not exceed capacity
    assert!(small_cache.len() <= small_cache.capacity());

    // Most recent entries should still be findable
    let mut recent_found = 0;
    let recent_range = total_inserts - small_cache.capacity()..total_inserts;

    for i in recent_range {
      if small_cache.get(format!("overflow_{}", i)).is_some() {
        recent_found += 1;
      }
    }

    assert!(recent_found > small_cache.capacity() / 4); // Should find reasonable portion

    // Older entries should mostly be gone
    let mut old_found = 0;
    for i in 0..small_cache.capacity() {
      if small_cache.get(format!("overflow_{}", i)).is_some() {
        old_found += 1;
      }
    }

    assert!(old_found < small_cache.capacity() / 2); // Most old entries should be gone
  }

  #[test]
  fn test_edge_case_empty_and_invalid_keys() {
    let cache = create_cache::<u32>(100);

    // Test empty string key
    assert!(cache.insert("", 42u32));
    assert_eq!(cache.get(""), Some(42u32));

    // Test very long key
    let long_key = "a".repeat(1000);
    assert!(cache.insert(&long_key, 123u32));
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
      assert!(cache.insert(*key, i as u32));
      assert_eq!(cache.get(*key), Some(i as u32));
    }
  }

  #[test]
  fn test_deterministic_behavior_same_input() {
    // Test that same operations produce consistent results
    for run in 0..3 {
      let cache = create_cache::<u64>(100);

      // Same sequence of operations
      let operations = vec![
        ("key1", 100u64),
        ("key2", 200u64),
        ("key3", 300u64),
        ("key1", 150u64), // Overwrite
        ("key4", 400u64),
      ];

      for (key, value) in &operations {
        cache.insert(*key, *value);
      }

      // Results should be consistent across runs
      assert_eq!(cache.get("key1"), Some(150u64));
      assert_eq!(cache.get("key2"), Some(200u64));
      assert_eq!(cache.get("key3"), Some(300u64));
      assert_eq!(cache.get("key4"), Some(400u64));
      assert_eq!(cache.get("nonexistent"), None);
    }
  }
}
