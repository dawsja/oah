//! Pull requests through the GitHub CLI (`gh`), run where the project is.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::Repo;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    /// "success", "failure", "pending", "skipped", "neutral"…
    pub state: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    /// OPEN, CLOSED, MERGED.
    pub state: String,
    pub draft: bool,
    pub url: String,
    pub base: String,
    pub head: String,
    pub additions: u64,
    pub deletions: u64,
    /// APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED or empty.
    pub review: String,
    /// CLEAN, BLOCKED, BEHIND, DIRTY, UNSTABLE…
    pub merge_state: String,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PrState {
    /// `gh` isn't installed or isn't signed in.
    Unavailable {
        reason: String,
    },
    /// No pull request for this branch yet.
    None {
        branch: String,
        base: String,
    },
    Open {
        pr: PullRequest,
    },
}

const FIELDS: &str =
    "number,title,state,isDraft,url,baseRefName,headRefName,additions,deletions,reviewDecision,mergeStateStatus,statusCheckRollup";

fn gh(repo: &Repo, args: &[&str], limit: Duration) -> Result<crate::Output, String> {
    repo.exec("gh", args, None, limit)
}

/// The current branch's pull request, if any.
pub fn pr_state(repo: &Repo) -> PrState {
    let out = match gh(repo, &["pr", "view", "--json", FIELDS], Duration::from_secs(30)) {
        Ok(o) => o,
        Err(e) if e.contains("isn't installed") => {
            return PrState::Unavailable { reason: "Install the GitHub CLI (gh) to work with pull requests.".into() }
        }
        Err(e) => return PrState::Unavailable { reason: e },
    };
    if out.ok() {
        return match serde_json::from_str::<Value>(&out.stdout) {
            Ok(v) => PrState::Open { pr: parse_pr(&v) },
            Err(e) => PrState::Unavailable { reason: format!("unexpected answer from gh: {e}") },
        };
    }
    let err = out.stderr.to_lowercase();
    if err.contains("no pull requests found") {
        let branch = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]).map(|b| b.trim().to_string()).unwrap_or_default();
        let base = crate::worktree::default_base(repo, "").trim_start_matches("origin/").to_string();
        return PrState::None { branch, base };
    }
    let reason = if err.contains("gh auth login") || err.contains("not logged") {
        "Sign in to GitHub with `gh auth login`.".to_string()
    } else if err.contains("no git remotes") || err.contains("none of the git remotes") || err.contains("not a git repository") {
        "This repository has no GitHub remote.".to_string()
    } else {
        out.stderr.trim().lines().last().unwrap_or("gh failed").to_string()
    };
    PrState::Unavailable { reason }
}

pub fn parse_pr(v: &Value) -> PullRequest {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let checks = v["statusCheckRollup"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            // CheckRun: name/status/conclusion; StatusContext: context/state.
            let name = c["name"].as_str().or(c["context"].as_str()).unwrap_or("check").to_string();
            let state = match (c["status"].as_str(), c["conclusion"].as_str(), c["state"].as_str()) {
                (Some(st), _, _) if st != "COMPLETED" => "pending".to_string(),
                (_, Some(con), _) if !con.is_empty() => con.to_lowercase(),
                (_, _, Some(st)) => match st {
                    "SUCCESS" => "success".into(),
                    "PENDING" | "EXPECTED" => "pending".into(),
                    other => other.to_lowercase(),
                },
                _ => "pending".into(),
            };
            Check { name, state, url: c["detailsUrl"].as_str().or(c["targetUrl"].as_str()).map(String::from) }
        })
        .collect();
    PullRequest {
        number: v["number"].as_u64().unwrap_or(0),
        title: s("title"),
        state: s("state"),
        draft: v["isDraft"].as_bool().unwrap_or(false),
        url: s("url"),
        base: s("baseRefName"),
        head: s("headRefName"),
        additions: v["additions"].as_u64().unwrap_or(0),
        deletions: v["deletions"].as_u64().unwrap_or(0),
        review: s("reviewDecision"),
        merge_state: s("mergeStateStatus"),
        checks,
    }
}

/// Pushes the branch if needed, then opens a pull request. Returns its URL.
pub fn pr_create(repo: &Repo, title: &str, body: &str, base: &str, draft: bool) -> Result<String, String> {
    if title.trim().is_empty() {
        return Err("Give the pull request a title.".into());
    }
    // A pull request for commits that never reached GitHub would be for the wrong thing.
    crate::sync(repo, crate::SyncOp::Push)?;
    let mut args = vec!["pr", "create", "--title", title, "--body", body];
    if !base.trim().is_empty() {
        args.extend(["--base", base.trim()]);
    }
    if draft {
        args.push("--draft");
    }
    let out = gh(repo, &args, Duration::from_secs(120))?;
    if out.ok() {
        Ok(out.stdout.lines().rev().find(|l| l.starts_with("http")).unwrap_or(out.stdout.trim()).to_string())
    } else {
        Err(out.stderr.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_pr() {
        let pr = parse_pr(&json!({
            "number": 42, "title": "Fix cart", "state": "OPEN", "isDraft": false, "url": "https://github.com/o/r/pull/42",
            "baseRefName": "main", "headRefName": "fix-cart", "additions": 6, "deletions": 2,
            "reviewDecision": "REVIEW_REQUIRED", "mergeStateStatus": "BLOCKED",
            "statusCheckRollup": [
                {"__typename": "CheckRun", "name": "test", "status": "COMPLETED", "conclusion": "SUCCESS", "detailsUrl": "https://ci/1"},
                {"__typename": "CheckRun", "name": "lint", "status": "IN_PROGRESS", "conclusion": ""},
                {"__typename": "StatusContext", "context": "deploy", "state": "FAILURE", "targetUrl": "https://ci/3"}
            ]
        }));
        assert_eq!(pr.number, 42);
        assert_eq!(pr.checks.iter().map(|c| c.state.as_str()).collect::<Vec<_>>(), ["success", "pending", "failure"]);
        assert_eq!(pr.checks[2].url.as_deref(), Some("https://ci/3"));
    }
}
