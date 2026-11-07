//! Lanes for the history graph. Each lane keeps its column while it lives, so a row is drawn
//! from three lists: lines passing straight through, lines coming into the commit's node from
//! above, and lines leaving it downwards.

use serde::Serialize;

use crate::Commit;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphRow {
    pub commit: Commit,
    /// The node's column.
    pub lane: usize,
    /// Columns drawn top to bottom without touching the node.
    pub through: Vec<usize>,
    /// Columns that end in the node (from the row's top).
    pub into: Vec<usize>,
    /// Columns the node continues into (to the row's bottom).
    pub out: Vec<usize>,
    /// Columns this row needs.
    pub width: usize,
}

pub fn lanes(commits: Vec<Commit>) -> Vec<GraphRow> {
    // active[c] = the commit the line in column c is heading to.
    let mut active: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    for commit in commits {
        let into: Vec<usize> = active.iter().enumerate().filter(|(_, h)| h.as_deref() == Some(&commit.hash)).map(|(i, _)| i).collect();
        let lane = match into.first() {
            Some(&c) => c,
            None => free_column(&mut active),
        };
        for &c in &into {
            active[c] = None;
        }
        let through: Vec<usize> = active.iter().enumerate().filter(|(_, h)| h.is_some()).map(|(i, _)| i).collect();

        let mut out = Vec::new();
        for (i, p) in commit.parents.iter().enumerate() {
            if let Some(c) = active.iter().position(|h| h.as_deref() == Some(p.as_str())) {
                // That parent already has a line: join it.
                out.push(c);
            } else if i == 0 {
                active[lane] = Some(p.clone());
                out.push(lane);
            } else {
                let c = free_column(&mut active);
                active[c] = Some(p.clone());
                out.push(c);
            }
        }
        while active.last().is_some_and(|h| h.is_none()) {
            active.pop();
        }
        let width = [lane + 1, active.len(), into.iter().max().map_or(0, |c| c + 1), through.iter().max().map_or(0, |c| c + 1), out.iter().max().map_or(0, |c| c + 1)]
            .into_iter()
            .max()
            .unwrap_or(1);
        let into = into.into_iter().collect();
        rows.push(GraphRow { commit, lane, through, into, out, width });
    }
    rows
}

fn free_column(active: &mut Vec<Option<String>>) -> usize {
    match active.iter().position(Option::is_none) {
        Some(c) => c,
        None => {
            active.push(None);
            active.len() - 1
        }
    }
}
