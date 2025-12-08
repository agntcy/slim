// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use parking_lot::RwLock;
use slim_datapath::messages::Name;
use std::collections::HashMap;
use std::sync::Arc;

// each route is compose by a name and the related connection id
// this allows as to distinguish between multiple routes to the same name.
// the route boolean indicates whether it's a route (true) or a subscription (false)
#[derive(Hash, Eq, PartialEq, Clone)]
pub(crate) struct Route {
    pub(crate) name: Name,
    pub(crate) conn_id: u64,
    pub(crate) route: bool,
}

#[derive(Default, Clone)]
pub struct SessionRoutes {
    /// map where to store all the subscriptions and routes
    /// set by the running sessions. for each route we also store
    /// a counter so that on subscription or route removal the
    /// action is done only is the counter is zero.
    routes: Arc<RwLock<HashMap<Route, u32>>>,
}

impl SessionRoutes {
    /// Adds a new route to the session's route map.
    /// If the route already exists, increments its counter.
    pub(crate) fn add_route(&self, route: Route) {
        let mut routes = self.routes.write();
        let counter = routes.entry(route).or_insert(0);
        *counter += 1;
    }

    /// Removes a route from the session's route map.
    /// Decrements the counter and removes the route if the counter reaches zero.
    pub(crate) fn remove_route(&self, route: &Route) {
        let mut routes = self.routes.write();
        if let Some(counter) = routes.get_mut(route) {
            if *counter > 1 {
                *counter -= 1;
            } else {
                routes.remove(route);
            }
        }
    }

    /// Checks if a route exists in the session's route map.
    pub(crate) fn has_route(&self, route: &Route) -> bool {
        let routes = self.routes.read();
        routes.contains_key(route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_new_session_routes_is_empty() {
        let routes = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["test", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };
        assert!(!routes.has_route(&route));
    }

    #[test]
    fn test_add_route_creates_new_entry() {
        let routes = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["test", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        routes.add_route(route.clone());
        assert!(routes.has_route(&route));
    }

    #[test]
    fn test_add_route_increments_counter() {
        let routes = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["test", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        // Add route multiple times
        routes.add_route(route.clone());
        routes.add_route(route.clone());
        routes.add_route(route.clone());

        // Route should exist
        assert!(routes.has_route(&route));

        // Check the counter is 3
        let guard = routes.routes.read();
        assert_eq!(*guard.get(&route).unwrap(), 3);
    }

    #[test]
    fn test_remove_route_decrements_counter() {
        let routes = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["test", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        // Add route 3 times
        routes.add_route(route.clone());
        routes.add_route(route.clone());
        routes.add_route(route.clone());

        // Remove once
        routes.remove_route(&route);

        // Route should still exist with counter = 2
        assert!(routes.has_route(&route));
        let guard = routes.routes.read();
        assert_eq!(*guard.get(&route).unwrap(), 2);
    }

    #[test]
    fn test_remove_route_deletes_when_counter_reaches_zero() {
        let routes = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["test", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        // Add route once
        routes.add_route(route.clone());
        assert!(routes.has_route(&route));

        // Remove once - should delete the entry
        routes.remove_route(&route);
        assert!(!routes.has_route(&route));
    }

    #[test]
    fn test_remove_route_nonexistent_does_nothing() {
        let routes = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["test", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        // Remove a route that doesn't exist - should not panic
        routes.remove_route(&route);
        assert!(!routes.has_route(&route));
    }

    #[test]
    fn test_multiple_routes_independent() {
        let routes = SessionRoutes::default();
        let route1 = Route {
            name: Name::from_strings(["route", "one", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };
        let route2 = Route {
            name: Name::from_strings(["route", "two", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };
        let route3 = Route {
            name: Name::from_strings(["route", "three", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        // Add different routes
        routes.add_route(route1.clone());
        routes.add_route(route2.clone());
        routes.add_route(route2.clone());
        routes.add_route(route3.clone());

        // All should exist
        assert!(routes.has_route(&route1));
        assert!(routes.has_route(&route2));
        assert!(routes.has_route(&route3));

        // Remove one route
        routes.remove_route(&route1);
        assert!(!routes.has_route(&route1));
        assert!(routes.has_route(&route2));
        assert!(routes.has_route(&route3));

        // Remove name2 once - should still exist
        routes.remove_route(&route2);
        assert!(routes.has_route(&route2));

        // Remove name2 again - should be gone
        routes.remove_route(&route2);
        assert!(!routes.has_route(&route2));
        assert!(routes.has_route(&route3));
    }

    #[test]
    fn test_has_route_returns_false_for_nonexistent() {
        let routes = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["nonexistent", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        assert!(!routes.has_route(&route));
    }

    #[test]
    fn test_concurrent_access() {
        use std::thread;

        let routes = Arc::new(SessionRoutes::default());
        let route = Route {
            name: Name::from_strings(["concurrent", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        let mut handles = vec![];

        // Spawn 10 threads that each add the route 10 times
        for _ in 0..10 {
            let routes_clone = Arc::clone(&routes);
            let route_clone = route.clone();
            let handle = thread::spawn(move || {
                for _ in 0..10 {
                    routes_clone.add_route(route_clone.clone());
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Counter should be 100
        assert!(routes.has_route(&route));
        let guard = routes.routes.read();
        assert_eq!(*guard.get(&route).unwrap(), 100);
    }

    #[test]
    fn test_add_and_remove_alternating() {
        let routes = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["alternating", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        // Add and remove alternately
        routes.add_route(route.clone());
        assert!(routes.has_route(&route));

        routes.add_route(route.clone());
        assert!(routes.has_route(&route));

        routes.remove_route(&route);
        assert!(routes.has_route(&route));

        routes.remove_route(&route);
        assert!(!routes.has_route(&route));

        routes.add_route(route.clone());
        assert!(routes.has_route(&route));

        routes.remove_route(&route);
        assert!(!routes.has_route(&route));
    }

    #[test]
    fn test_route_with_different_ids() {
        let routes = SessionRoutes::default();
        let route1 = Route {
            name: Name::from_strings(["test", "route", "v1"]).with_id(0),
            conn_id: 100,
            route: true,
        };
        let route2 = Route {
            name: Name::from_strings(["test", "route", "v1"]).with_id(0),
            conn_id: 200,
            route: true,
        };

        // These should be treated as different routes if conn_ids differ
        routes.add_route(route1.clone());
        routes.add_route(route2.clone());

        assert!(routes.has_route(&route1));
        assert!(routes.has_route(&route2));

        routes.remove_route(&route1);
        assert!(!routes.has_route(&route1));
        assert!(routes.has_route(&route2));
    }

    #[test]
    fn test_clone_shares_routes() {
        let routes1 = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["shared", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        routes1.add_route(route.clone());

        // Clone shares the Arc, so both instances point to the same HashMap
        let routes2 = routes1.clone();

        assert!(routes1.has_route(&route));
        assert!(routes2.has_route(&route));

        // Adding via routes2 affects routes1 since they share the same Arc<RwLock<HashMap>>
        routes2.add_route(route.clone());

        let guard = routes1.routes.read();
        assert_eq!(*guard.get(&route).unwrap(), 2);
    }

    #[test]
    fn test_remove_multiple_times_safe() {
        let routes = SessionRoutes::default();
        let route = Route {
            name: Name::from_strings(["test", "route", "v1"]).with_id(0),
            conn_id: 1,
            route: true,
        };

        routes.add_route(route.clone());

        // Remove multiple times - should not panic
        routes.remove_route(&route);
        routes.remove_route(&route);
        routes.remove_route(&route);

        assert!(!routes.has_route(&route));
    }

    #[test]
    fn test_route_and_subscription_with_same_name_are_distinct() {
        let routes = SessionRoutes::default();
        let name = Name::from_strings(["test", "name", "v1"]).with_id(0);

        // Create a route (route: true)
        let route = Route {
            name: name.clone(),
            conn_id: 1,
            route: true,
        };

        // Create a subscription (route: false)
        let subscription = Route {
            name: name.clone(),
            conn_id: 1,
            route: false,
        };

        // Add both
        routes.add_route(route.clone());
        routes.add_route(subscription.clone());

        // Both should exist independently
        assert!(routes.has_route(&route));
        assert!(routes.has_route(&subscription));

        // Remove the route - subscription should still exist
        routes.remove_route(&route);
        assert!(!routes.has_route(&route));
        assert!(routes.has_route(&subscription));

        // Remove the subscription - both should be gone
        routes.remove_route(&subscription);
        assert!(!routes.has_route(&route));
        assert!(!routes.has_route(&subscription));
    }
}
