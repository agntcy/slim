// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::warn;

/// How the Gateway was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewaySource {
    /// Pinned by set_default_gateway. Auto-scan never overwrites this.
    Explicit,
    /// The only Edge connection in the table.
    Auto,
}

/// Cached / resolved gateway. This is the `Gateway` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gateway {
    // Need a scan (startup, or after establish / drop / type change).
    Dirty,
    Some {
        conn_id: u64,
        source: GatewaySource,
    },
    /// 0 Edge, or explicit id is gone.
    Empty,
    /// More than one Edge and no explicit pin.
    Ambiguous {
        count: usize,
    },
}

#[derive(Debug)]
pub struct DefaultGateway {
    /// Auto-scan on/off. Default true.
    enabled: AtomicBool,
    /// Pinned id. Auto never writes this.
    explicit: Mutex<Option<u64>>,
    /// Last auto/empty/ambiguous result. Ignored while explicit is Some.
    cache: Mutex<Gateway>,
}

impl DefaultGateway {
    pub fn new() -> Self {
        DefaultGateway {
            enabled: AtomicBool::new(true),
            explicit: Mutex::new(None),
            cache: Mutex::new(Gateway::Dirty),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_explicit(&self, conn_id: Option<u64>) {
        *self.explicit.lock() = conn_id;
    }

    pub fn explicit(&self) -> Option<u64> {
        *self.explicit.lock()
    }

    /// Mark cache Dirty. Do not clear explicit.
    pub fn invalidate(&self) {
        *self.cache.lock() = Gateway::Dirty;
    }

    /// Peek the cache without scanning. May be `Dirty`.
    pub fn cached(&self) -> Gateway {
        *self.cache.lock()
    }

    /// `scan` returns current Edge connection ids.
    pub fn resolve(&self, scan: impl FnOnce() -> Vec<u64>) -> Gateway {
        if let Some(id) = *self.explicit.lock() {
            let edges = scan();
            if edges.contains(&id) {
                return Gateway::Some {
                    conn_id: id,
                    source: GatewaySource::Explicit,
                };
            }
            warn!(conn_id = id, "explicit default gateway is gone");
            return Gateway::Empty;
        }

        if !self.is_enabled() {
            return Gateway::Empty;
        }

        let mut cache = self.cache.lock();
        if *cache != Gateway::Dirty {
            return *cache;
        }

        let edges = scan();
        let result = match edges.as_slice() {
            [] => Gateway::Empty,
            [id] => Gateway::Some {
                conn_id: *id,
                source: GatewaySource::Auto,
            },
            edges => Gateway::Ambiguous { count: edges.len() },
        };
        *cache = result;
        result
    }
}

impl Default for DefaultGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for DefaultGateway {
    fn clone(&self) -> Self {
        DefaultGateway {
            enabled: AtomicBool::new(self.is_enabled()),
            explicit: Mutex::new(self.explicit()),
            cache: Mutex::new(*self.cache.lock()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn auto(id: u64) -> Gateway {
        Gateway::Some {
            conn_id: id,
            source: GatewaySource::Auto,
        }
    }

    fn explicit(id: u64) -> Gateway {
        Gateway::Some {
            conn_id: id,
            source: GatewaySource::Explicit,
        }
    }

    #[test]
    fn resolve_dirty_zero_one_two_edges() {
        let gw = DefaultGateway::new();
        assert_eq!(gw.resolve(Vec::new), Gateway::Empty);

        let gw = DefaultGateway::new();
        assert_eq!(gw.resolve(|| vec![7]), auto(7));

        let gw = DefaultGateway::new();
        assert_eq!(gw.resolve(|| vec![7, 9]), Gateway::Ambiguous { count: 2 });
    }

    #[test]
    fn resolve_explicit_hit_and_missing() {
        let gw = DefaultGateway::new();
        gw.set_explicit(Some(7));

        assert_eq!(gw.resolve(|| vec![7, 9]), explicit(7));
        assert_eq!(gw.resolve(|| vec![9]), Gateway::Empty);
    }

    #[test]
    fn resolve_disabled_without_and_with_explicit() {
        let gw = DefaultGateway::new();
        gw.set_enabled(false);
        assert_eq!(gw.resolve(|| vec![7]), Gateway::Empty);

        gw.set_explicit(Some(7));
        assert_eq!(gw.resolve(|| vec![7]), explicit(7));
    }

    #[test]
    fn invalidate_does_not_clear_explicit() {
        let gw = DefaultGateway::new();
        gw.set_explicit(Some(7));
        gw.invalidate();

        assert_eq!(gw.explicit(), Some(7));
        assert_eq!(gw.resolve(|| vec![7]), explicit(7));
    }

    #[test]
    fn resolve_cache_hit_does_not_scan_again() {
        let gw = DefaultGateway::new();
        let scans = AtomicUsize::new(0);

        assert_eq!(
            gw.resolve(|| {
                scans.fetch_add(1, AtomicOrdering::Relaxed);
                vec![7]
            }),
            auto(7)
        );
        assert_eq!(scans.load(AtomicOrdering::Relaxed), 1);

        assert_eq!(gw.resolve(|| panic!("cache should not scan")), auto(7));

        gw.invalidate();
        assert_eq!(gw.resolve(|| vec![9]), auto(9));
    }

    #[test]
    fn resolve_empty_is_cached() {
        let gw = DefaultGateway::new();
        assert_eq!(gw.resolve(Vec::new), Gateway::Empty);
        assert_eq!(
            gw.resolve(|| panic!("cache should not scan")),
            Gateway::Empty
        );
    }
}
