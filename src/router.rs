//! Matrix autorouter: assigns nets to matrix rows and computes solder jumper states.
//!
//! Mirrors protorouter's relay matrix logic: connections → nets (union-find) → row assignment → jumper states.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::protomatrix::{JumperStateProvider, ProtoSide, ProtomatrixConfig, ProtomatrixTarget};

/// Column identifier: (side, col). Pads in the same column share a conductor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColumnKey {
    pub side: ProtoSide,
    pub col: i32,
}

impl ColumnKey {
    pub fn from_pad(pad: &ProtomatrixTarget) -> Option<Self> {
        match pad {
            ProtomatrixTarget::Pad { side, col, .. } => Some(ColumnKey {
                side: *side,
                col: *col,
            }),
            _ => None,
        }
    }
}

/// Extracts ColumnKey from a pad target; returns None for non-pad targets.
fn pad_to_column(pad: &ProtomatrixTarget) -> Option<ColumnKey> {
    ColumnKey::from_pad(pad)
}

/// Union-Find (disjoint set) for computing nets.
struct UnionFind {
    parent: HashMap<ColumnKey, ColumnKey>,
}

impl UnionFind {
    fn new() -> Self {
        Self {
            parent: HashMap::new(),
        }
    }

    fn find(&mut self, key: ColumnKey) -> ColumnKey {
        let mut k = key;
        while let Some(&p) = self.parent.get(&k) {
            if p == k {
                break;
            }
            // Path compression
            if let Some(&gp) = self.parent.get(&p) {
                self.parent.insert(k, gp);
            }
            k = p;
        }
        self.parent.entry(key).or_insert(key);
        k
    }

    fn union(&mut self, a: ColumnKey, b: ColumnKey) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent.insert(ra, rb);
        }
    }

    /// Returns map from root -> set of all columns in that net.
    fn roots(&mut self, keys: &[ColumnKey]) -> HashMap<ColumnKey, HashSet<ColumnKey>> {
        let mut result: HashMap<ColumnKey, HashSet<ColumnKey>> = HashMap::new();
        for &k in keys {
            let r = self.find(k);
            result.entry(r).or_default().insert(k);
        }
        result
    }
}

/// Connection between two pads. Only Pad targets are valid; others are ignored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub a: ProtomatrixTarget,
    pub b: ProtomatrixTarget,
}

impl Connection {
    pub fn new(a: ProtomatrixTarget, b: ProtomatrixTarget) -> Self {
        Self { a, b }
    }
}

/// Result of autorouting: which solder jumpers should be closed and which net they belong to.
#[derive(Clone, Debug, Default)]
pub struct JumperState {
    /// Closed jumpers: (side, col, row) -> net index (0-based).
    pub jumper_net: HashMap<(ProtoSide, i32, i32), usize>,
    /// Canonical key for each net index (for stable color lookup). net_canonical_keys[i] = min ColumnKey in net i.
    pub net_canonical_keys: Vec<ColumnKey>,
}

impl JumperState {
    pub fn is_closed(&self, side: ProtoSide, col: i32, row: i32) -> bool {
        self.jumper_net.contains_key(&(side, col, row))
    }

    pub fn net_index(&self, side: ProtoSide, col: i32, row: i32) -> Option<usize> {
        self.jumper_net.get(&(side, col, row)).copied()
    }

    /// Net index for a proto column (pad); returns first closed jumper in that column.
    pub fn net_for_column(&self, side: ProtoSide, col: i32, matrix_size: i32) -> Option<usize> {
        for row in 0..matrix_size {
            if let Some(ni) = self.net_index(side, col, row) {
                return Some(ni);
            }
        }
        None
    }

    pub fn closed_count(&self) -> usize {
        self.jumper_net.len()
    }
}

impl JumperStateProvider for JumperState {
    fn is_closed(&self, side: ProtoSide, col: i32, row: i32) -> bool {
        self.jumper_net.contains_key(&(side, col, row))
    }

    fn net_index(&self, side: ProtoSide, col: i32, row: i32) -> Option<usize> {
        self.jumper_net.get(&(side, col, row)).copied()
    }
}

/// Result of autoroute; may contain an error.
#[derive(Debug)]
pub enum AutorouteResult {
    Ok(JumperState),
    Err(String),
}

/// Compute nets from connections using union-find (same as net.rkt).
/// Nets are sorted by canonical key (min ColumnKey in each net) for stable indices when adding connections.
fn compute_nets(connections: &[Connection]) -> Vec<HashSet<ColumnKey>> {
    let mut uf = UnionFind::new();
    let mut all_keys = HashSet::new();

    for conn in connections {
        if let (Some(ka), Some(kb)) = (pad_to_column(&conn.a), pad_to_column(&conn.b)) {
            uf.union(ka, kb);
            all_keys.insert(ka);
            all_keys.insert(kb);
        }
    }

    let roots = uf.roots(&all_keys.into_iter().collect::<Vec<_>>());
    let mut nets: Vec<HashSet<ColumnKey>> = roots.into_values().collect();
    nets.sort_by_key(|net| *net.iter().min().unwrap_or(&ColumnKey {
        side: ProtoSide::Lower,
        col: 0,
    }));
    nets
}

/// Assign nets to matrix rows. Cross-side nets get the same row on both sides.
/// Returns (net, row_index). Fails if too many nets.
fn assign_nets_to_rows(
    config: &ProtomatrixConfig,
    nets: &[HashSet<ColumnKey>],
) -> Result<Vec<(HashSet<ColumnKey>, i32)>, String> {
    let matrix_size = config.matrix_size;
    if nets.len() > matrix_size as usize {
        return Err(format!(
            "Too many nets ({}), max {} rows available",
            nets.len(),
            matrix_size
        ));
    }

    Ok(nets
        .iter()
        .enumerate()
        .map(|(i, net)| (net.clone(), i as i32))
        .collect())
}

/// Run autoroute: connections → nets → row assignment → jumper states.
pub fn autoroute(
    config: &ProtomatrixConfig,
    connections: &[Connection],
) -> AutorouteResult {
    let nets = compute_nets(connections);
    if nets.is_empty() {
        return AutorouteResult::Ok(JumperState::default());
    }

    let assignments = match assign_nets_to_rows(config, &nets) {
        Ok(a) => a,
        Err(e) => return AutorouteResult::Err(e),
    };

    let mut jumper_net = HashMap::new();
    let net_canonical_keys: Vec<ColumnKey> = assignments
        .iter()
        .map(|(net, _)| *net.iter().min().unwrap())
        .collect();
    for (net_idx, (net, row)) in assignments.into_iter().enumerate() {
        for col_key in &net {
            jumper_net.insert((col_key.side, col_key.col, row), net_idx);
        }
    }

    AutorouteResult::Ok(JumperState {
        jumper_net,
        net_canonical_keys,
    })
}
