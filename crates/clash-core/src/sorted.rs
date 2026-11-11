//! Helpers for a `Vec` kept sorted and free of duplicates.
//!
//! A sorted `Vec` is the cheapest deterministic set or small map: it
//! iterates in order, serialises as a plain list and hashes as one. These
//! functions keep one in order. They assume the `Vec` is already sorted and
//! free of duplicates; [`from_unsorted`] makes one that is.
//!
//! ```
//! use clash_core::sorted;
//!
//! let mut tags = Vec::new();
//! assert!(sorted::insert(&mut tags, "wet"));
//! assert!(sorted::insert(&mut tags, "burning"));
//! assert!(!sorted::insert(&mut tags, "wet"));
//! assert_eq!(tags, ["burning", "wet"]);
//! assert!(sorted::contains(&tags, &"wet"));
//! assert!(sorted::remove(&mut tags, &"wet"));
//! assert_eq!(tags, ["burning"]);
//! ```
//!
//! The `_by_key` functions do the same for records ordered by a key taken
//! from each record, which makes a small ordered map:
//!
//! ```
//! use clash_core::sorted;
//!
//! let mut stock: Vec<(u32, &str)> = Vec::new();
//! sorted::insert_by_key(&mut stock, (7, "rope"), |entry| entry.0);
//! sorted::insert_by_key(&mut stock, (2, "lamp"), |entry| entry.0);
//! assert_eq!(sorted::find_by_key(&stock, &7, |entry| entry.0), Some(&(7, "rope")));
//! assert_eq!(stock[0], (2, "lamp"));
//! ```

/// Sorts a `Vec` and removes duplicates, making it fit for the other
/// functions of this module.
pub fn from_unsorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
    items.sort_unstable();
    items.dedup();
    items
}

/// Whether a slice is in strictly ascending order: sorted, with no
/// duplicates.
pub fn is_sorted_set<T: Ord>(items: &[T]) -> bool {
    items.windows(2).all(|pair| pair[0] < pair[1])
}

/// Whether the item is present.
pub fn contains<T: Ord>(items: &[T], item: &T) -> bool {
    items.binary_search(item).is_ok()
}

/// Inserts an item in its place. Returns `false`, changing nothing, if an
/// equal item is already present.
pub fn insert<T: Ord>(items: &mut Vec<T>, item: T) -> bool {
    match items.binary_search(&item) {
        Ok(_) => false,
        Err(at) => {
            items.insert(at, item);
            true
        }
    }
}

/// Removes an item. Returns whether it was present.
pub fn remove<T: Ord>(items: &mut Vec<T>, item: &T) -> bool {
    match items.binary_search(item) {
        Ok(at) => {
            items.remove(at);
            true
        }
        Err(_) => false,
    }
}

/// The record with the given key, in a slice ordered by that key.
pub fn find_by_key<'a, T, K: Ord>(
    items: &'a [T],
    key: &K,
    key_of: impl Fn(&T) -> K,
) -> Option<&'a T> {
    items
        .binary_search_by(|probe| key_of(probe).cmp(key))
        .ok()
        .map(|at| &items[at])
}

/// Inserts a record in its place by key. A record with the same key is
/// replaced and returned.
pub fn insert_by_key<T, K: Ord>(
    items: &mut Vec<T>,
    item: T,
    key_of: impl Fn(&T) -> K,
) -> Option<T> {
    let key = key_of(&item);
    match items.binary_search_by(|probe| key_of(probe).cmp(&key)) {
        Ok(at) => Some(core::mem::replace(&mut items[at], item)),
        Err(at) => {
            items.insert(at, item);
            None
        }
    }
}

/// Removes and returns the record with the given key, if there is one.
pub fn remove_by_key<T, K: Ord>(
    items: &mut Vec<T>,
    key: &K,
    key_of: impl Fn(&T) -> K,
) -> Option<T> {
    items
        .binary_search_by(|probe| key_of(probe).cmp(key))
        .ok()
        .map(|at| items.remove(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_keeps_order_and_refuses_duplicates() {
        let mut items = Vec::new();
        for value in [5, 1, 9, 3, 5, 1, 7] {
            insert(&mut items, value);
        }
        assert_eq!(items, [1, 3, 5, 7, 9]);
        assert!(is_sorted_set(&items));
        assert!(!insert(&mut items, 3));
        assert!(insert(&mut items, 4));
        assert_eq!(items, [1, 3, 4, 5, 7, 9]);
    }

    #[test]
    fn remove_and_contains() {
        let mut items = from_unsorted(vec![4, 2, 4, 8, 6, 2]);
        assert_eq!(items, [2, 4, 6, 8]);
        assert!(contains(&items, &6));
        assert!(!contains(&items, &5));
        assert!(remove(&mut items, &6));
        assert!(!remove(&mut items, &6));
        assert_eq!(items, [2, 4, 8]);
    }

    #[test]
    fn is_sorted_set_rejects_disorder_and_duplicates() {
        assert!(is_sorted_set::<u8>(&[]));
        assert!(is_sorted_set(&[1]));
        assert!(is_sorted_set(&[1, 2, 3]));
        assert!(!is_sorted_set(&[1, 1, 2]));
        assert!(!is_sorted_set(&[2, 1]));
    }

    #[test]
    fn keyed_records_behave_as_a_small_map() {
        let mut items: Vec<(u8, &str)> = Vec::new();
        assert_eq!(insert_by_key(&mut items, (3, "c"), |entry| entry.0), None);
        assert_eq!(insert_by_key(&mut items, (1, "a"), |entry| entry.0), None);
        assert_eq!(insert_by_key(&mut items, (2, "b"), |entry| entry.0), None);
        assert_eq!(items, [(1, "a"), (2, "b"), (3, "c")]);
        assert_eq!(
            insert_by_key(&mut items, (2, "B"), |entry| entry.0),
            Some((2, "b"))
        );
        assert_eq!(find_by_key(&items, &2, |entry| entry.0), Some(&(2, "B")));
        assert_eq!(find_by_key(&items, &9, |entry| entry.0), None);
        assert_eq!(
            remove_by_key(&mut items, &1, |entry| entry.0),
            Some((1, "a"))
        );
        assert_eq!(remove_by_key(&mut items, &1, |entry| entry.0), None);
        assert_eq!(items, [(2, "B"), (3, "c")]);
    }
}
