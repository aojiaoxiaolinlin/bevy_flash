//! RAII free-list ownership, shared by GPU texture buckets and CPU lifetime tests.
use std::{
    ops::Deref,
    sync::{Arc, Mutex, Weak},
};

pub struct Pool<T> {
    available: Arc<Mutex<Vec<T>>>,
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self {
            available: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl<T> Pool<T> {
    pub fn take(&self, create: impl FnOnce() -> T) -> (Lease<T>, bool) {
        let item = self.available.lock().expect("pool poisoned").pop();
        let reused = item.is_some();
        (
            Lease {
                item: Some(item.unwrap_or_else(create)),
                pool: Arc::downgrade(&self.available),
            },
            reused,
        )
    }

    pub fn available_len(&self) -> usize {
        self.available.lock().expect("pool poisoned").len()
    }

    /// Drops every currently idle item without affecting outstanding leases.
    pub fn clear_available(&self) -> usize {
        let mut available = self.available.lock().expect("pool poisoned");
        let removed = available.len();
        available.clear();
        removed
    }
}

/// Intentionally not Clone. A live lease excludes its resource from future allocations.
pub struct Lease<T> {
    item: Option<T>,
    pool: Weak<Mutex<Vec<T>>>,
}

impl<T> Deref for Lease<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.item.as_ref().expect("live lease")
    }
}

impl<T> Drop for Lease<T> {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade()
            && let Some(item) = self.item.take()
        {
            pool.lock().expect("pool poisoned").push(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_borrows_are_distinct_and_siblings_reuse_in_same_frame() {
        let pool = Pool::default();
        let (parent, reused) = pool.take(|| 1);
        assert!(!reused);
        let (child, reused) = pool.take(|| 2);
        assert!(!reused);
        assert_ne!(*parent, *child);
        drop(child);
        let (sibling, reused) = pool.take(|| panic!("must reuse without frame reset"));
        assert!(reused);
        assert_eq!(*sibling, 2);
        assert_eq!(*parent, 1);
    }

    #[test]
    fn live_lease_survives_pool_eviction() {
        let pool = Pool::default();
        let (lease, _) = pool.take(|| String::from("alive"));
        drop(pool);
        assert_eq!(&**lease, "alive");
        drop(lease);
    }

    #[test]
    fn clearing_idle_items_preserves_live_leases() {
        let pool = Pool::default();
        let (live, _) = pool.take(|| String::from("live"));
        let (idle, _) = pool.take(|| String::from("idle"));
        drop(idle);
        assert_eq!(pool.clear_available(), 1);
        assert_eq!(&**live, "live");
        assert_eq!(pool.available_len(), 0);
    }
}
