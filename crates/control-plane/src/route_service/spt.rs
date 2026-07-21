// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Shortest Path Tree (SPT) computation using Dijkstra.
//!
//! Given a link graph and a root node, computes the SPT as a directed graph
//! with edges pointing from root toward leaves. This defines the forwarding
//! tree for a given route name.

use std::collections::HashMap;

use petgraph::algo::dijkstra;
use petgraph::graph::{DiGraph, NodeIndex, UnGraph};
use petgraph::visit::EdgeRef;

/// A Shortest Path Tree rooted at a specific node.
/// Stored as a directed graph with edges pointing from parent to child
/// (root toward leaves).
#[derive(Debug, Clone)]
pub struct SptTree {
    pub root: NodeIndex,
    /// Directed tree: edges go parent → child (root toward leaves).
    /// Node weights are domain names, edge weights are costs.
    pub tree: DiGraph<String, u32>,
    /// Maps original graph NodeIndex → tree NodeIndex.
    pub index_map: HashMap<NodeIndex, NodeIndex>,
}

/// Compute the Shortest Path Tree rooted at `root` on the given undirected graph.
///
/// Uses Dijkstra to find shortest distances, then reconstructs the tree by
/// finding, for each non-root node, the neighbor whose distance is exactly
/// one edge weight less (i.e. the parent on the shortest path).
///
/// Returns `None` if root is not in the graph.
pub fn compute_spt(root: NodeIndex, graph: &UnGraph<String, u32>) -> Option<SptTree> {
    graph.node_weight(root)?;

    let distances = dijkstra(graph, root, None, |e| *e.weight());

    let mut tree = DiGraph::<String, u32>::new();
    let mut index_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();

    // Add all reachable nodes to the tree.
    for &orig_idx in distances.keys() {
        let weight = graph.node_weight(orig_idx).unwrap().clone();
        let tree_idx = tree.add_node(weight);
        index_map.insert(orig_idx, tree_idx);
    }

    let tree_root = index_map[&root];

    // Add directed edges from parent → child.
    for (&node, &dist) in &distances {
        if node == root {
            continue;
        }
        for edge in graph.edges(node) {
            let neighbor = edge.target();
            if neighbor == node {
                continue;
            }
            let weight = *edge.weight();
            if let Some(&neighbor_dist) = distances.get(&neighbor)
                && neighbor_dist + weight == dist
            {
                // neighbor is the parent of node
                let parent_tree = index_map[&neighbor];
                let child_tree = index_map[&node];
                tree.add_edge(parent_tree, child_tree, weight);
                break;
            }
        }
    }

    Some(SptTree {
        root: tree_root,
        tree,
        index_map,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use petgraph::Direction;

    fn make_full_mesh() -> (UnGraph<String, u32>, Vec<NodeIndex>) {
        let mut g = UnGraph::new_undirected();
        let a = g.add_node("a".into());
        let b = g.add_node("b".into());
        let c = g.add_node("c".into());
        let d = g.add_node("d".into());
        g.add_edge(a, b, 1);
        g.add_edge(a, c, 1);
        g.add_edge(a, d, 1);
        g.add_edge(b, c, 1);
        g.add_edge(b, d, 1);
        g.add_edge(c, d, 1);
        (g, vec![a, b, c, d])
    }

    fn make_star() -> (UnGraph<String, u32>, Vec<NodeIndex>) {
        let mut g = UnGraph::new_undirected();
        let hub = g.add_node("hub".into());
        let a = g.add_node("a".into());
        let b = g.add_node("b".into());
        let c = g.add_node("c".into());
        g.add_edge(hub, a, 1);
        g.add_edge(hub, b, 1);
        g.add_edge(hub, c, 1);
        (g, vec![hub, a, b, c])
    }

    fn make_chain() -> (UnGraph<String, u32>, Vec<NodeIndex>) {
        let mut g = UnGraph::new_undirected();
        let a = g.add_node("a".into());
        let b = g.add_node("b".into());
        let c = g.add_node("c".into());
        let d = g.add_node("d".into());
        g.add_edge(a, b, 1);
        g.add_edge(b, c, 1);
        g.add_edge(c, d, 1);
        (g, vec![a, b, c, d])
    }

    fn out_degree(tree: &SptTree, orig_idx: NodeIndex) -> usize {
        let tree_idx = tree.index_map[&orig_idx];
        tree.tree
            .edges_directed(tree_idx, Direction::Outgoing)
            .count()
    }

    fn in_degree(tree: &SptTree, orig_idx: NodeIndex) -> usize {
        let tree_idx = tree.index_map[&orig_idx];
        tree.tree
            .edges_directed(tree_idx, Direction::Incoming)
            .count()
    }

    #[test]
    fn full_mesh_root_at_c() {
        let (g, nodes) = make_full_mesh();
        let [a, b, c, d] = [nodes[0], nodes[1], nodes[2], nodes[3]];

        let spt = compute_spt(c, &g).unwrap();
        // All nodes are 1 hop from root in full mesh.
        assert_eq!(out_degree(&spt, c), 3);
        assert_eq!(in_degree(&spt, c), 0);
        for &n in &[a, b, d] {
            assert_eq!(in_degree(&spt, n), 1);
            assert_eq!(out_degree(&spt, n), 0);
        }
    }

    #[test]
    fn star_root_at_hub() {
        let (g, nodes) = make_star();
        let [hub, a, b, c] = [nodes[0], nodes[1], nodes[2], nodes[3]];

        let spt = compute_spt(hub, &g).unwrap();
        assert_eq!(out_degree(&spt, hub), 3);
        assert_eq!(in_degree(&spt, hub), 0);
        for &n in &[a, b, c] {
            assert_eq!(in_degree(&spt, n), 1);
            assert_eq!(out_degree(&spt, n), 0);
        }
    }

    #[test]
    fn star_root_at_spoke() {
        let (g, nodes) = make_star();
        let [hub, a, b, c] = [nodes[0], nodes[1], nodes[2], nodes[3]];

        // Root at spoke 'a': a→hub→{b,c}
        let spt = compute_spt(a, &g).unwrap();
        assert_eq!(out_degree(&spt, a), 1);
        assert_eq!(in_degree(&spt, a), 0);
        assert_eq!(in_degree(&spt, hub), 1);
        assert_eq!(out_degree(&spt, hub), 2);
        for &n in &[b, c] {
            assert_eq!(in_degree(&spt, n), 1);
            assert_eq!(out_degree(&spt, n), 0);
        }
    }

    #[test]
    fn chain_root_at_a() {
        let (g, nodes) = make_chain();
        let [a, b, c, d] = [nodes[0], nodes[1], nodes[2], nodes[3]];

        // a→b→c→d
        let spt = compute_spt(a, &g).unwrap();
        assert_eq!(out_degree(&spt, a), 1);
        assert_eq!(out_degree(&spt, b), 1);
        assert_eq!(out_degree(&spt, c), 1);
        assert_eq!(out_degree(&spt, d), 0);
        assert_eq!(in_degree(&spt, a), 0);
    }

    #[test]
    fn chain_root_at_middle() {
        let (g, nodes) = make_chain();
        let [a, b, c, d] = [nodes[0], nodes[1], nodes[2], nodes[3]];

        // Root at b: b→a, b→c→d
        let spt = compute_spt(b, &g).unwrap();
        assert_eq!(out_degree(&spt, b), 2);
        assert_eq!(out_degree(&spt, a), 0);
        assert_eq!(out_degree(&spt, c), 1);
        assert_eq!(out_degree(&spt, d), 0);
    }

    #[test]
    fn invalid_root() {
        let (g, _) = make_star();
        let bad_idx = NodeIndex::new(99);
        assert!(compute_spt(bad_idx, &g).is_none());
    }

    #[test]
    fn single_node() {
        let mut g = UnGraph::new_undirected();
        let a = g.add_node("a".into());

        let spt = compute_spt(a, &g).unwrap();
        assert_eq!(spt.tree.node_count(), 1);
        assert_eq!(spt.tree.edge_count(), 0);
    }

    #[test]
    fn disconnected_node_not_in_tree() {
        let mut g = UnGraph::new_undirected();
        let a = g.add_node("a".into());
        let b = g.add_node("b".into());
        let c = g.add_node("c".into());
        g.add_edge(a, b, 1);
        // c is disconnected

        let spt = compute_spt(a, &g).unwrap();
        assert!(spt.index_map.contains_key(&a));
        assert!(spt.index_map.contains_key(&b));
        assert!(!spt.index_map.contains_key(&c));
        assert_eq!(spt.tree.node_count(), 2);
    }

    #[test]
    fn weighted_edges_prefer_shorter_path() {
        let mut g = UnGraph::new_undirected();
        let a = g.add_node("a".into());
        let b = g.add_node("b".into());
        let c = g.add_node("c".into());
        // Direct a→c costs 10, but a→b→c costs 2
        g.add_edge(a, b, 1);
        g.add_edge(b, c, 1);
        g.add_edge(a, c, 10);

        let spt = compute_spt(a, &g).unwrap();
        // c's parent should be b (cheaper path)
        let c_tree = spt.index_map[&c];
        let b_tree = spt.index_map[&b];
        let c_incoming: Vec<_> = spt
            .tree
            .edges_directed(c_tree, Direction::Incoming)
            .collect();
        assert_eq!(c_incoming.len(), 1);
        assert_eq!(c_incoming[0].source(), b_tree);
    }
}
