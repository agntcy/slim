// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::cell::{Ref, RefCell, RefMut};

/// A RefCell wrapper that implements Send and Sync.
/// This is safe to use in a single-threaded/sequentially executed context
/// (such as within a single Tokio task) where no concurrent multi-threaded
/// access occurs, but where types are required to be Send + Sync to satisfy
/// compiler bounds (e.g., for spawned Tokio tasks).
#[derive(Debug)]
pub struct SingleThreadedCell<T>(RefCell<T>);

impl<T> SingleThreadedCell<T> {
    pub fn new(value: T) -> Self {
        Self(RefCell::new(value))
    }

    pub fn borrow(&self) -> Ref<'_, T> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        self.0.borrow_mut()
    }
}

unsafe impl<T: Send> Send for SingleThreadedCell<T> {}
unsafe impl<T: Send> Sync for SingleThreadedCell<T> {}
