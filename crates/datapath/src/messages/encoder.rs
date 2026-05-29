// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::hash::{Hash, Hasher};
use twox_hash::XxHash64;

pub fn calculate_hash<T: Hash + ?Sized>(t: &T) -> u64 {
    let mut hasher = XxHash64::default();
    t.hash(&mut hasher);
    hasher.finish()
}
