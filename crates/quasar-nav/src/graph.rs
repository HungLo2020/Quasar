use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
};

/// Weighted directed/undirected graph edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neighbor<N> {
    pub node: N,
    pub cost: f32,
}

impl<N> Neighbor<N> {
    pub const fn new(node: N, cost: f32) -> Self {
        Self { node, cost }
    }
}

/// Caller-owned graph. Neighbors are generated on demand; games can adapt
/// tilemaps, voxel chunks, waypoint graphs or navigation meshes.
pub trait Graph {
    type Node: Copy + Eq + std::hash::Hash + Ord;
    fn neighbors(&self, node: Self::Node, output: &mut Vec<Neighbor<Self::Node>>);
}

#[derive(Clone, Debug)]
pub struct SearchOptions {
    pub max_visited: usize,
    pub max_cost: f64,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            max_visited: 1_000_000,
            max_cost: f64::MAX,
            cancel: None,
        }
    }
}

impl SearchOptions {
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }
    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(AtomicOrdering::Acquire))
    }
    fn cost_limit(&self) -> f64 {
        if self.max_cost.is_finite() && self.max_cost >= 0.0 {
            self.max_cost
        } else {
            f64::MAX
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Path<N> {
    pub nodes: Vec<N>,
    pub cost: f64,
    /// Nodes closed during the search that produced this path.
    pub visited: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SearchOutcome<N> {
    Found(Path<N>),
    NotFound { visited: usize },
    Cancelled { visited: usize },
    LimitReached { visited: usize },
}

impl<N> SearchOutcome<N> {
    pub fn path(self) -> Option<Path<N>> {
        match self {
            Self::Found(path) => Some(path),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Open<N> {
    total: f64,
    cost: f64,
    node: N,
    sequence: u64,
}

impl<N: Ord> PartialEq for Open<N> {
    fn eq(&self, other: &Self) -> bool {
        self.total.to_bits() == other.total.to_bits()
            && self.cost.to_bits() == other.cost.to_bits()
            && self.node == other.node
            && self.sequence == other.sequence
    }
}
impl<N: Ord> Eq for Open<N> {}
impl<N: Ord> PartialOrd for Open<N> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<N: Ord> Ord for Open<N> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .total
            .total_cmp(&self.total)
            // Among equal f-values, prefer the node closest to the goal. This
            // avoids expanding a large equal-cost plateau across the map.
            .then_with(|| self.cost.total_cmp(&other.cost))
            .then_with(|| other.node.cmp(&self.node))
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

/// A* with caller-defined heuristic. Invalid/negative heuristic values are
/// treated as zero. Negative or non-finite edge costs are skipped.
pub fn astar<G, F>(
    graph: &G,
    start: G::Node,
    goal: G::Node,
    heuristic: F,
    options: SearchOptions,
) -> SearchOutcome<G::Node>
where
    G: Graph,
    F: Fn(G::Node) -> f32,
{
    if options.cancelled() {
        return SearchOutcome::Cancelled { visited: 0 };
    }
    if start == goal {
        return SearchOutcome::Found(Path {
            nodes: vec![start],
            cost: 0.0,
            visited: 1,
        });
    }
    if options.max_visited == 0 {
        return SearchOutcome::LimitReached { visited: 0 };
    }
    let heuristic = |node: G::Node| {
        let value = heuristic(node);
        if value.is_finite() && value > 0.0 {
            f64::from(value)
        } else {
            0.0
        }
    };
    let mut open = BinaryHeap::new();
    let mut best = HashMap::<G::Node, f64>::new();
    let mut came_from = HashMap::<G::Node, G::Node>::new();
    let mut closed = HashSet::<G::Node>::new();
    let mut sequence = 0u64;
    best.insert(start, 0.0);
    open.push(Open {
        total: heuristic(start),
        cost: 0.0,
        node: start,
        sequence,
    });
    let mut neighbors = Vec::new();
    let mut visited = 0usize;
    let mut pruned_by_cost = false;
    while let Some(entry) = open.pop() {
        if options.cancelled() {
            return SearchOutcome::Cancelled { visited };
        }
        if !closed.insert(entry.node) {
            continue;
        }
        visited += 1;
        if visited > options.max_visited {
            return SearchOutcome::LimitReached { visited };
        }
        if entry.node == goal {
            let mut nodes = vec![goal];
            let mut current = goal;
            while let Some(previous) = came_from.get(&current) {
                nodes.push(*previous);
                current = *previous;
            }
            nodes.reverse();
            return SearchOutcome::Found(Path {
                nodes,
                cost: *best.get(&goal).unwrap_or(&0.0),
                visited,
            });
        }
        let current_cost = *best.get(&entry.node).unwrap_or(&f64::INFINITY);
        neighbors.clear();
        graph.neighbors(entry.node, &mut neighbors);
        for neighbor in &neighbors {
            if !neighbor.cost.is_finite() || neighbor.cost < 0.0 {
                continue;
            }
            if closed.contains(&neighbor.node) {
                continue;
            }
            let cost = current_cost + f64::from(neighbor.cost);
            if cost > options.cost_limit() {
                pruned_by_cost = true;
                continue;
            }
            if cost < *best.get(&neighbor.node).unwrap_or(&f64::INFINITY) {
                best.insert(neighbor.node, cost);
                came_from.insert(neighbor.node, entry.node);
                sequence = sequence.wrapping_add(1);
                open.push(Open {
                    total: cost + heuristic(neighbor.node),
                    cost,
                    node: neighbor.node,
                    sequence,
                });
            }
        }
    }
    if pruned_by_cost {
        SearchOutcome::LimitReached { visited }
    } else {
        SearchOutcome::NotFound { visited }
    }
}

/// Dijkstra optimality without a heuristic.
pub fn dijkstra<G: Graph>(
    graph: &G,
    start: G::Node,
    goal: G::Node,
    options: SearchOptions,
) -> SearchOutcome<G::Node> {
    astar(graph, start, goal, |_| 0.0, options)
}

/// Greedy string-pulling. `line_of_sight` must be symmetric and conservative
/// for the game's collision model.
pub fn smooth_path<N: Copy>(path: &[N], line_of_sight: impl Fn(N, N) -> bool) -> Vec<N> {
    if path.len() <= 2 {
        return path.to_vec();
    }
    let mut result = Vec::with_capacity(path.len());
    result.push(path[0]);
    let mut index = 0usize;
    while index + 1 < path.len() {
        let mut furthest = index + 1;
        for candidate in index + 2..path.len() {
            if line_of_sight(path[index], path[candidate]) {
                furthest = candidate;
            } else {
                break;
            }
        }
        result.push(path[furthest]);
        index = furthest;
    }
    result
}
