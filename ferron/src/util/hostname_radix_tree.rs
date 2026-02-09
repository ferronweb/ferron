use std::{borrow::Cow, collections::BTreeMap};

/// A multi-key for the hostname lookup
#[derive(Clone, Debug, Eq, PartialEq)]
struct HostnameRadixTreeMultiKey<'a>(Vec<Cow<'a, str>>);

#[allow(clippy::non_canonical_partial_ord_impl)]
impl<'a> PartialOrd for HostnameRadixTreeMultiKey<'a> {
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

impl<'a> Ord for HostnameRadixTreeMultiKey<'a> {
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

/// A node in the hostname radix tree
#[derive(Debug)]
struct HostnameRadixTreeNode<T> {
  /// The exact-match configuration value associated with this node, if any
  pub exact_value: Option<T>,
  /// The wildcard configuration value associated with this node, if any
  pub wildcard_value: Option<T>,
  /// The child nodes of this node
  pub children: BTreeMap<HostnameRadixTreeMultiKey<'static>, HostnameRadixTreeNode<T>>,
}

/// A radix tree for hostname lookups, supporting exact matches and wildcard matches
#[derive(Debug)]
pub struct HostnameRadixTree<T> {
  root: HostnameRadixTreeNode<T>,
}

impl<T> HostnameRadixTree<T> {
  /// Creates a new empty hostname radix tree
  pub fn new() -> Self {
    Self {
      root: HostnameRadixTreeNode {
        exact_value: None,
        wildcard_value: None,
        children: BTreeMap::new(),
      },
    }
  }

  /// Inserts a configuration value into the tree based on the provided key
  pub fn insert(&mut self, matcher: String, value: T) {
    let mut key: Vec<Cow<'static, str>> = Vec::new();
    let mut is_wildcard = false;
    for part in matcher.split('.').rev() {
      if part.is_empty() {
        continue;
      }
      if part == "*" {
        is_wildcard = true;
        continue;
      }
      if is_wildcard {
        is_wildcard = false;
        key.push(Cow::Owned("*".to_string()));
      }
      key.push(Cow::Owned(part.to_string()));
    }

    let mut current_node = &mut self.root;
    let mut key_iter = key.into_iter();
    let mut key_option = key_iter.next();
    while let Some(key) = key_option.take() {
      let mut multi_key = HostnameRadixTreeMultiKey(vec![key]);
      match current_node.children.entry(multi_key) {
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
                std::mem::transmute::<&mut HostnameRadixTreeNode<T>, &mut HostnameRadixTreeNode<T>>(entry.get_mut())
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
              let entry_key_right = HostnameRadixTreeMultiKey(entry_key.0.split_off(i));
              #[allow(clippy::mutable_key_type)]
              let mut new_children = BTreeMap::new();
              new_children.insert(entry_key_right, entry_value);
              match current_node.children.entry(entry_key) {
                std::collections::btree_map::Entry::Occupied(entry) => {
                  current_node = entry.into_mut();
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                  current_node = entry.insert(HostnameRadixTreeNode {
                    exact_value: None,
                    wildcard_value: None,
                    children: new_children,
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
          while let Some(key) = key_option.take() {
            multi_key.0.push(key);
            key_option = key_iter.next();
          }

          match current_node.children.entry(multi_key) {
            std::collections::btree_map::Entry::Occupied(entry) => {
              current_node = entry.into_mut();
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
              current_node = entry.insert(HostnameRadixTreeNode {
                exact_value: None,
                wildcard_value: None,
                children: BTreeMap::new(),
              });
            }
          };
        }
      }
    }

    if is_wildcard {
      current_node.wildcard_value = Some(value);
    } else {
      current_node.exact_value = Some(value);
    }
  }

  /// Obtains a reference to the configuration value associated with the provided multi-key, if it exists
  pub fn get<'a>(&'a self, hostname: &'a str) -> Option<&'a T> {
    let mut key: HostnameRadixTreeMultiKey<'a> = HostnameRadixTreeMultiKey(
      hostname
        .split('.')
        .rev()
        .filter_map(|s| if s.is_empty() { None } else { Some(Cow::Borrowed(s)) })
        .collect(),
    );

    let mut current_node = &self.root;
    let mut previous_value = None;
    let mut value = current_node.wildcard_value.as_ref();
    while !key.0.is_empty() {
      if let Some((child_key, child)) = current_node.children.get_key_value(&key) {
        // If a fixed multi-key matches, continue with the child node and the remaining key
        if child_key.0.len() > key.0.len() {
          // The child key is longer than the remaining key, so the fixed multi-key does not match
          break;
        }
        current_node = child;
        key.0 = key.0.split_off(child_key.0.len());
        if current_node.wildcard_value.is_some() {
          previous_value = value;
          value = current_node.wildcard_value.as_ref();
        }
        continue;
      }

      // If nothing happened, probably no matching fixed multi-key found, so break the loop...
      break;
    }

    if key.0.is_empty() {
      // If the whole key is consumed, that means the whole hostname matches.
      // In this case, set the value to the exact value, since it has higher priority than
      // the wildcard value
      value = previous_value;
      if let Some(exact_value) = current_node.exact_value.as_ref() {
        value = Some(exact_value);
      }
    }

    value
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::borrow::Cow;

  #[test]
  fn test_hostname_radix_tree_multi_key_partial_ord() {
    let a = HostnameRadixTreeMultiKey(vec![Cow::Borrowed("com"), Cow::Borrowed("example")]);
    let b = HostnameRadixTreeMultiKey(vec![Cow::Borrowed("com"), Cow::Borrowed("example")]);
    let c = HostnameRadixTreeMultiKey(vec![Cow::Borrowed("com"), Cow::Borrowed("test")]);
    let d = HostnameRadixTreeMultiKey(vec![Cow::Borrowed("com")]);

    assert_eq!(a.partial_cmp(&b), Some(std::cmp::Ordering::Equal));
    assert_eq!(a.partial_cmp(&c), Some(std::cmp::Ordering::Less));
    assert_eq!(c.partial_cmp(&a), Some(std::cmp::Ordering::Greater));
    assert_eq!(a.partial_cmp(&d), None);
  }

  #[test]
  fn test_hostname_radix_tree_multi_key_ord() {
    let a = HostnameRadixTreeMultiKey(vec![Cow::Borrowed("com"), Cow::Borrowed("example")]);
    let b = HostnameRadixTreeMultiKey(vec![Cow::Borrowed("com"), Cow::Borrowed("example")]);
    let c = HostnameRadixTreeMultiKey(vec![Cow::Borrowed("com"), Cow::Borrowed("test")]);
    let d = HostnameRadixTreeMultiKey(vec![Cow::Borrowed("com")]);

    assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
    assert_eq!(a.cmp(&c), std::cmp::Ordering::Less);
    assert_eq!(c.cmp(&a), std::cmp::Ordering::Greater);
    assert_eq!(a.cmp(&d), std::cmp::Ordering::Equal);
  }

  #[test]
  fn test_hostname_radix_tree_insert_and_get_exact() {
    let mut tree = HostnameRadixTree::new();
    tree.insert("example.com".to_string(), 42);
    assert_eq!(tree.get("example.com"), Some(&42));
    assert_eq!(tree.get("test.com"), None);
  }

  #[test]
  fn test_hostname_radix_tree_insert_and_get_wildcard() {
    let mut tree = HostnameRadixTree::new();
    tree.insert("*.example.com".to_string(), 42);
    assert_eq!(tree.get("test.example.com"), Some(&42));
    assert_eq!(tree.get("example.com"), None);
  }

  #[test]
  fn test_hostname_radix_tree_insert_and_get_overlap() {
    let mut tree = HostnameRadixTree::new();
    tree.insert("example.com".to_string(), 42);
    tree.insert("*.example.com".to_string(), 100);
    assert_eq!(tree.get("example.com"), Some(&42));
    assert_eq!(tree.get("test.example.com"), Some(&100));
  }

  #[test]
  fn test_hostname_radix_tree_insert_and_get_non_existent() {
    let mut tree = HostnameRadixTree::new();
    tree.insert("example.com".to_string(), 42);
    assert_eq!(tree.get("nonexistent.com"), None);
  }

  #[test]
  fn test_hostname_radix_tree_insert_and_get_empty_key() {
    let mut tree = HostnameRadixTree::new();
    tree.insert("".to_string(), 42);
    assert_eq!(tree.get(""), Some(&42));
  }
}
