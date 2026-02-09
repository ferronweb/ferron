use std::collections::BTreeMap;

use ferron_common::config::Conditional;

use crate::config::lookup::conditionals::{match_conditional, ConditionMatchData};

/// A single key for the configuration tree lookup
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigFilterTreeSingleKey {
  /// The configuration is a host configuration
  IsHostConfiguration,
  /// The port number
  Port(u16),
  /// An octet of an IPv4 address
  IPv4Octet(u8),
  /// An octet of an IPv6 address
  IPv6Octet(u8),
  /// The configuration is for localhost
  IsLocalhost,
  /// A hostname domain level
  HostDomainLevel(String),
  /// A hostname domain level with wildcard
  HostDomainLevelWildcard,
  /// A location path segment
  LocationSegment(String),
  /// A conditional
  Conditional(Conditional),
  // Note how error handler status isn't included in the tree key,
  // because error handlers are stored separately and don't affect the tree structure
}

impl ConfigFilterTreeSingleKey {
  /// Checks whether the single tree key contains a predicate
  pub fn is_predicate(&self) -> bool {
    matches!(self, Self::HostDomainLevelWildcard | Self::Conditional(_))
  }
}

/// A multi-key for the configuration tree lookup
#[derive(Debug, Eq, PartialEq)]
struct ConfigFilterTreeMultiKey(Vec<ConfigFilterTreeSingleKey>);

#[allow(clippy::non_canonical_partial_ord_impl)]
impl PartialOrd for ConfigFilterTreeMultiKey {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    for i in 0..self.0.len().max(other.0.len()) {
      let self_element = self.0.get(i);
      let other_element = other.0.get(i);
      match (self_element, other_element) {
        (Some(a), Some(b)) => {
          let cmp = a.cmp(b);
          if cmp != std::cmp::Ordering::Equal {
            return Some(cmp);
          }
        }
        _ => return None,
      }
    }
    Some(std::cmp::Ordering::Equal)
  }
}

impl Ord for ConfigFilterTreeMultiKey {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    // The `partial_cmp` method returns `None` if the keys are of different lengths,
    // but for the `Ord` trait we need to define a total order.
    // In this case, we can consider keys of different lengths as equal for ordering purposes,
    // since they will be handled separately in the tree structure.
    //
    // This is especially important in radix trees with BTreeMaps as children.
    self.partial_cmp(other).unwrap_or(std::cmp::Ordering::Equal)
  }
}

/// A node in the configuration filter tree
#[derive(Debug)]
struct ConfigFilterTreeNode<T> {
  /// The configuration value associated with this node, if any
  pub value: Option<T>,
  /// The child nodes of this node, indexed by their fixed value multi-key
  pub children_fixed: BTreeMap<ConfigFilterTreeMultiKey, ConfigFilterTreeNode<T>>,
  /// The child nodes of this node, indexed by their predicate single key
  pub children_predicate: BTreeMap<ConfigFilterTreeSingleKey, ConfigFilterTreeNode<T>>,
}

/// The configuration filter tree, used for efficient lookup of configurations based on multiple criteria.
/// The tree is a hybrid of a radix tree and a trie, where each node can have multiple fixed value children (like a radix tree)
/// and multiple predicate children (like a trie).
#[derive(Debug)]
pub struct ConfigFilterTree<T> {
  /// The root node of the configuration tree
  root: ConfigFilterTreeNode<T>,
}

impl<T> ConfigFilterTree<T> {
  /// Creates a new empty configuration filter tree
  pub fn new() -> Self {
    Self {
      root: ConfigFilterTreeNode {
        value: None,
        children_fixed: BTreeMap::new(),
        children_predicate: BTreeMap::new(),
      },
    }
  }

  /// Inserts a configuration value into the tree based on the provided multi-key
  #[allow(dead_code)]
  pub fn insert(&mut self, key: Vec<ConfigFilterTreeSingleKey>, value: T) {
    self.insert_node(key).replace(value);
  }

  /// Inserts a node into the tree based on the provided multi-key and returns a mutable reference to the value at that node.
  pub fn insert_node(&mut self, key: Vec<ConfigFilterTreeSingleKey>) -> &mut Option<T> {
    let mut current_node = &mut self.root;
    let mut key_iter = key.into_iter();
    let mut key_option = key_iter.next();
    while let Some(key) = key_option.take() {
      if key.is_predicate() {
        // This key is a predicate, so we need to look for it in the children_predicate map
        match current_node.children_predicate.entry(key) {
          std::collections::btree_map::Entry::Occupied(entry) => {
            current_node = entry.into_mut();
          }
          std::collections::btree_map::Entry::Vacant(entry) => {
            current_node = entry.insert(ConfigFilterTreeNode {
              value: None,
              children_fixed: BTreeMap::new(),
              children_predicate: BTreeMap::new(),
            });
          }
        }
        key_option = key_iter.next();
      } else {
        // This key is not a predicate, so we can try to insert it into the children_fixed map as part of a multi-key
        let mut multi_key = ConfigFilterTreeMultiKey(vec![key]);
        match current_node.children_fixed.entry(multi_key) {
          std::collections::btree_map::Entry::Occupied(mut entry) => {
            let entry_key = entry.key();
            for i in 1..=entry_key.0.len() {
              if i == entry_key.0.len() {
                // All keys match, continue with the existing multi-key

                key_option = key_iter.next();
                // Safety: after the current node is changed, we immediately break the inner loop and continue
                // the outer loop. Without "unsafe", the borrow checker would not allow us to borrow the entry
                // as mutable again in the next iteration of the outer loop.
                current_node = unsafe {
                  std::mem::transmute::<&mut ConfigFilterTreeNode<T>, &mut ConfigFilterTreeNode<T>>(entry.get_mut())
                };
                break;
              }
              key_option = key_iter.next();
              let mut break_multi_key = false;
              if let Some(key) = &key_option {
                if key != &entry_key.0[i] {
                  // Keys differ, break the multi-key and insert a new node
                  break_multi_key = true;
                }
              } else {
                // No more keys, break the multi-key
                break_multi_key = true;
              }
              if break_multi_key {
                // Break the multi-key at index i and insert a new node for the remaining keys
                let (mut entry_key, entry_value) = entry.remove_entry();
                let entry_key_right = ConfigFilterTreeMultiKey(entry_key.0.split_off(i));
                #[allow(clippy::mutable_key_type)]
                let mut new_children_fixed = BTreeMap::new();
                new_children_fixed.insert(entry_key_right, entry_value);
                match current_node.children_fixed.entry(entry_key) {
                  std::collections::btree_map::Entry::Occupied(entry) => {
                    current_node = entry.into_mut();
                  }
                  std::collections::btree_map::Entry::Vacant(entry) => {
                    current_node = entry.insert(ConfigFilterTreeNode {
                      value: None,
                      children_fixed: new_children_fixed,
                      children_predicate: BTreeMap::new(),
                    });
                  }
                }
                break;
              }
            }
          }
          std::collections::btree_map::Entry::Vacant(entry) => {
            multi_key = entry.into_key();

            key_option = key_iter.next();
            while let Some(key) = &key_option {
              if !key.is_predicate() {
                let key = key_option.take().expect("key_option should be Some here");
                multi_key.0.push(key);
                key_option = key_iter.next();
              } else {
                break;
              }
            }

            match current_node.children_fixed.entry(multi_key) {
              std::collections::btree_map::Entry::Occupied(entry) => {
                current_node = entry.into_mut();
              }
              std::collections::btree_map::Entry::Vacant(entry) => {
                current_node = entry.insert(ConfigFilterTreeNode {
                  value: None,
                  children_fixed: BTreeMap::new(),
                  children_predicate: BTreeMap::new(),
                });
              }
            };
          }
        }
      }
    }

    &mut current_node.value
  }

  /// Obtains a reference to the configuration value associated with the provided multi-key, if it exists
  pub fn get<'a, 'b>(
    &'a self,
    key: Vec<ConfigFilterTreeSingleKey>,
    condition_match_data: Option<ConditionMatchData<'b>>,
  ) -> Result<Option<&'a T>, Box<dyn std::error::Error + Send + Sync>> {
    let mut current_node = &self.root;
    let mut key = ConfigFilterTreeMultiKey(key);
    let mut value = current_node.value.as_ref();
    while !key.0.is_empty() {
      let key_end = key.0.split_off(1);
      let mut partial_key = ConfigFilterTreeMultiKey(key.0);
      key.0 = key_end;
      if let Some((child_key, child)) = current_node.children_fixed.get_key_value(&partial_key) {
        // If a fixed multi-key matches, continue with the child node and the remaining key
        current_node = child;
        let mut index = 0;
        let mut secondary_index = 1;
        let mut is_matching = true;
        while let (Some(key_single), Some(child_key_single)) = (key.0.get(index), child_key.0.get(secondary_index)) {
          if std::mem::discriminant(key_single) == std::mem::discriminant(child_key_single) {
            // The keys have the same variant, so we can compare them
            if key_single != child_key_single {
              // The keys differ, so the fixed multi-key does not match
              is_matching = false;
              break;
            } else {
              // The keys match, continue with the next key
              secondary_index += 1;
            }
          }
          index += 1;
        }
        if !is_matching || (index >= key.0.len() && secondary_index < child_key.0.len()) {
          // The keys differ or are out of bounds, so the fixed multi-key does not match
          break;
        }
        key.0 = key.0.split_off(index);
        if current_node.value.is_some() {
          value = current_node.value.as_ref();
        }
        continue;
      }

      let partial_key = partial_key.0.remove(0);
      if partial_key.is_predicate() {
        // The next key is a predicate, so try to match the predicate single keys
        if let Some(child) = current_node.children_predicate.get(&partial_key) {
          // If a predicate key matches, continue with the child node and the remaining key
          current_node = child;
          if current_node.value.is_some() {
            value = current_node.value.as_ref();
          }
          continue;
        }
      }

      // If no fixed multi-key matches, try to match the predicate single keys
      for (predicate_key, child) in &current_node.children_predicate {
        if predicate_key.is_predicate() {
          // This is a predicate key, so we need to check if it matches the condition data
          if predicate_key == &ConfigFilterTreeSingleKey::HostDomainLevelWildcard
            && matches!(partial_key, ConfigFilterTreeSingleKey::HostDomainLevel(_))
          {
            // Host domain level wildcard matches any host domain level
            current_node = child;
            let mut index = 0;
            while let Some(ConfigFilterTreeSingleKey::HostDomainLevel(_)) = key.0.get(index) {
              index += 1;
            }
            key.0 = key.0.split_off(index);
            if current_node.value.is_some() {
              value = current_node.value.as_ref();
            }
            break;
          } else if let ConfigFilterTreeSingleKey::Conditional(conditional) = predicate_key {
            // Conditional key, check if the condition matches
            if let Some(condition_match_data) = condition_match_data.as_ref() {
              if match_conditional(conditional, condition_match_data)? {
                current_node = child;
                if current_node.value.is_some() {
                  value = current_node.value.as_ref();
                }
                break;
              }
            }
          }
        }
      }

      // If nothing happened, probably no matching fixed multi-key or predicate single key found...
    }

    Ok(value)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_basic_get() {
    let mut tree = ConfigFilterTree::new();
    tree.insert(
      vec![
        ConfigFilterTreeSingleKey::Port(80),
        ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
        ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
        ConfigFilterTreeSingleKey::HostDomainLevelWildcard,
      ],
      "Example",
    );
    tree.insert(
      vec![
        ConfigFilterTreeSingleKey::Port(80),
        ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
        ConfigFilterTreeSingleKey::HostDomainLevel("example2".to_string()),
        ConfigFilterTreeSingleKey::HostDomainLevel("www".to_string()),
      ],
      "Example 2",
    );

    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("www".to_string()),
          ],
          None
        )
        .unwrap(),
      Some(&"Example")
    );

    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("subsite".to_string()),
          ],
          None
        )
        .unwrap(),
      Some(&"Example")
    );

    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("example2".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("www".to_string()),
          ],
          None
        )
        .unwrap(),
      Some(&"Example 2")
    );

    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("example3".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("www".to_string()),
          ],
          None
        )
        .unwrap(),
      None
    );

    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
          ],
          None
        )
        .unwrap(),
      None
    );
  }

  #[test]
  fn test_empty_tree() {
    let tree: ConfigFilterTree<&str> = ConfigFilterTree::new();
    assert_eq!(tree.get(vec![], None).unwrap(), None);
  }

  #[test]
  fn test_insert_and_get_single_key() {
    let mut tree = ConfigFilterTree::new();
    tree.insert(vec![ConfigFilterTreeSingleKey::Port(80)], "Port 80");
    assert_eq!(
      tree.get(vec![ConfigFilterTreeSingleKey::Port(80)], None).unwrap(),
      Some(&"Port 80")
    );
  }

  #[test]
  fn test_insert_and_get_multi_key() {
    let mut tree = ConfigFilterTree::new();
    tree.insert(
      vec![
        ConfigFilterTreeSingleKey::Port(80),
        ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
      ],
      "Port 80, com",
    );
    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
          ],
          None
        )
        .unwrap(),
      Some(&"Port 80, com")
    );
  }

  #[test]
  fn test_wildcard_matching() {
    let mut tree = ConfigFilterTree::new();
    tree.insert(
      vec![
        ConfigFilterTreeSingleKey::Port(80),
        ConfigFilterTreeSingleKey::HostDomainLevelWildcard,
      ],
      "Wildcard",
    );
    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
          ],
          None
        )
        .unwrap(),
      Some(&"Wildcard")
    );
  }

  #[test]
  fn test_partial_key_matching() {
    let mut tree = ConfigFilterTree::new();
    tree.insert(
      vec![
        ConfigFilterTreeSingleKey::Port(80),
        ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
        ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
      ],
      "Partial",
    );
    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
          ],
          None
        )
        .unwrap(),
      None
    );
  }

  #[test]
  fn test_overlapping_keys() {
    let mut tree = ConfigFilterTree::new();
    tree.insert(
      vec![
        ConfigFilterTreeSingleKey::Port(80),
        ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
      ],
      "First",
    );
    tree.insert(
      vec![
        ConfigFilterTreeSingleKey::Port(80),
        ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
        ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
      ],
      "Second",
    );
    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
          ],
          None
        )
        .unwrap(),
      Some(&"First")
    );
    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
          ],
          None
        )
        .unwrap(),
      Some(&"Second")
    );
  }

  #[test]
  fn test_mixed_predicate_and_fixed_keys() {
    let mut tree = ConfigFilterTree::new();
    tree.insert(
      vec![
        ConfigFilterTreeSingleKey::Port(80),
        ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
        ConfigFilterTreeSingleKey::HostDomainLevelWildcard,
      ],
      "Mixed",
    );
    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
          ],
          None
        )
        .unwrap(),
      Some(&"Mixed")
    );
  }

  #[test]
  fn test_keys_with_redundant_in_between() {
    let mut tree = ConfigFilterTree::new();
    tree.insert(
      vec![
        ConfigFilterTreeSingleKey::Port(80),
        ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
        ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
      ],
      "Value",
    );
    assert_eq!(
      tree
        .get(
          vec![
            ConfigFilterTreeSingleKey::Port(80),
            ConfigFilterTreeSingleKey::IPv4Octet(127),
            ConfigFilterTreeSingleKey::IPv4Octet(0),
            ConfigFilterTreeSingleKey::IPv4Octet(0),
            ConfigFilterTreeSingleKey::IPv4Octet(1),
            ConfigFilterTreeSingleKey::HostDomainLevel("com".to_string()),
            ConfigFilterTreeSingleKey::HostDomainLevel("example".to_string()),
          ],
          None
        )
        .unwrap(),
      Some(&"Value")
    );
  }
}
