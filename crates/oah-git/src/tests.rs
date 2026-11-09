use super::*;
use std::path::Path;

fn sh(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn repo() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().unwrap();
    sh(dir.path(), &["init", "-q", "-b", "main"]);
    sh(dir.path(), &["config", "user.name", "t"]);
    sh(dir.path(), &["config", "user.email", "t@t"]);
    sh(dir.path(), &["config", "core.autocrlf", "false"]);
    let r = Repo::new(Location::Local, dir.path().to_string_lossy());
    (dir, r)
}

#[test]
fn status_stage_commit_round_trip() {
    let (dir, r) = repo();
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    let st = status(&r).unwrap();
    assert!(st.is_repo);
    assert_eq!(st.branch.as_deref(), Some("main"));
    assert_eq!(st.untracked.len(), 1);
    assert!(log(&r, 10).unwrap().is_empty(), "no commits yet");

    stage(&r, &["a.txt".into()]).unwrap();
    let st = status(&r).unwrap();
    assert_eq!(st.staged[0].kind, ChangeKind::Added);
    unstage(&r, &["a.txt".into()]).unwrap();
    assert_eq!(status(&r).unwrap().untracked.len(), 1, "unstage works before the first commit");

    let hash = commit(&r, "first", false, true).unwrap();
    assert!(!hash.is_empty());
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::rename(dir.path().join("a.txt"), dir.path().join("b.txt")).unwrap();
    sh(dir.path(), &["add", "-A"]);
    std::fs::write(dir.path().join("b.txt"), "one\ntwo\nthree\n").unwrap();
    let st = status(&r).unwrap();
    assert_eq!(st.staged[0].kind, ChangeKind::Renamed);
    assert_eq!(st.staged[0].orig_path.as_deref(), Some("a.txt"));
    assert_eq!((st.unstaged[0].added, st.unstaged[0].removed), (1, 0));
    assert!(diff(&r, "b.txt", false, false).unwrap().contains("+three"));

    discard(&r, &["b.txt".into()], &[]).unwrap();
    assert!(status(&r).unwrap().unstaged.is_empty());
    std::fs::write(dir.path().join("junk.txt"), "x").unwrap();
    assert!(diff(&r, "junk.txt", false, true).unwrap().contains("+x"));
    discard(&r, &[], &["junk.txt".into()]).unwrap();
    assert!(!dir.path().join("junk.txt").exists());
}

#[test]
fn not_a_repo() {
    let dir = tempfile::tempdir().unwrap();
    let st = status(&Repo::new(Location::Local, dir.path().to_string_lossy())).unwrap();
    assert!(!st.is_repo);
}

#[test]
fn history_lanes_on_a_merge() {
    let (dir, r) = repo();
    let w = |name: &str, body: &str| std::fs::write(dir.path().join(name), body).unwrap();
    w("a", "1");
    sh(dir.path(), &["add", "-A"]);
    sh(dir.path(), &["commit", "-qm", "base"]);
    sh(dir.path(), &["switch", "-qc", "feature"]);
    w("b", "1");
    sh(dir.path(), &["add", "-A"]);
    sh(dir.path(), &["commit", "-qm", "feature work"]);
    sh(dir.path(), &["switch", "-q", "main"]);
    w("c", "1");
    sh(dir.path(), &["add", "-A"]);
    sh(dir.path(), &["commit", "-qm", "main work"]);
    sh(dir.path(), &["merge", "-q", "--no-ff", "feature", "-m", "merge feature"]);

    let rows = log(&r, 50).unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].commit.subject, "merge feature");
    assert_eq!(rows[0].out.len(), 2, "a merge leaves into two lanes");
    assert!(rows.iter().any(|r| r.width == 2));
    assert_eq!(rows[3].commit.subject, "base");
    assert!(rows[1..3].iter().any(|r| r.out.iter().any(|c| *c != r.lane)), "one side joins the other's line");
    assert_eq!(rows[3].into.len(), 1, "one line is left at the base");
    assert!(rows[0].commit.refs.iter().any(|r| r.contains("main")));

    let files = commit_files(&r, &rows[1].commit.hash).unwrap();
    assert_eq!(files.len(), 1);
    let b = branches(&r).unwrap();
    assert!(b.iter().any(|x| x.name == "feature" && !x.current));
    assert!(b.iter().any(|x| x.name == "main" && x.current));
}

#[test]
fn files_and_grep() {
    let (dir, r) = repo();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {\n    println!(\"Hello\");\n}\n").unwrap();
    std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir(dir.path().join("target")).unwrap();
    std::fs::write(dir.path().join("target/x"), "hello").unwrap();
    let f = files(&r, 100).unwrap();
    assert_eq!(f, [".gitignore", "src/main.rs"]);
    let hits = grep(&r, "hello", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].path.as_str(), hits[0].line), ("src/main.rs", 2));
    assert!(grep(&r, "zzz-nothing", 10).unwrap().is_empty());
    assert!(read_file(&r, "src/main.rs", 1000).unwrap().starts_with("fn main"));
    assert!(read_file(&r, "../etc/passwd", 1000).is_err());
}

#[test]
fn image_files_inside_and_outside_the_project() {
    use base64::Engine;
    let (_dir, r) = repo();
    let outside = tempfile::tempdir().unwrap();
    let bytes = b"\x89PNG\r\n\x1a\n\0\xff\xfe";
    let path = outside.path().join("a picture's.PNG");
    std::fs::write(&path, bytes).unwrap();
    std::fs::write(std::path::Path::new(&r.path).join("picture.png"), bytes).unwrap();
    let expected = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
    assert_eq!(read_image(&r, "picture.png", bytes.len()).unwrap(), expected);
    assert_eq!(read_image(&r, path.to_str().unwrap(), bytes.len()).unwrap(), expected);
    assert!(read_image(&r, "picture.png", bytes.len() - 1).unwrap_err().contains("too big"));
    assert!(read_image(&r, "missing.png", 1024).is_err());
    assert!(read_image(&r, "not-an-image.txt", 1024).is_err());
}

#[test]
fn ahead_of_a_bare_remote() {
    let (dir, r) = repo();
    let remote = tempfile::tempdir().unwrap();
    sh(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    std::fs::write(dir.path().join("a"), "1").unwrap();
    commit(&r, "one", false, true).unwrap();
    sh(dir.path(), &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    sync(&r, SyncOp::Push).unwrap();
    std::fs::write(dir.path().join("a"), "2").unwrap();
    commit(&r, "two", false, true).unwrap();
    let st = status(&r).unwrap();
    assert_eq!(st.upstream.as_deref(), Some("origin/main"));
    assert_eq!((st.ahead, st.behind), (1, 0));
    sync(&r, SyncOp::Sync).unwrap();
    assert_eq!(status(&r).unwrap().ahead, 0);
}

#[test]
fn worktrees_add_and_remove() {
    let (dir, r) = repo();
    std::fs::write(dir.path().join("a"), "1").unwrap();
    commit(&r, "one", false, true).unwrap();
    let list = worktree::worktrees(&r).unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].main);

    let path = worktree::default_dir(&r, "", "feat/new thing");
    assert!(path.replace('\\', "/").ends_with("-worktrees/feat-new-thing"), "{path}");
    let base = worktree::default_base(&r, "");
    assert_eq!(base, "main");
    let w = worktree::add(&r, &path, "feat/new-thing", &base).unwrap();
    assert_eq!(w.branch.as_deref(), Some("feat/new-thing"));
    assert!(!w.main);
    std::fs::write(std::path::Path::new(&path).join("b"), "dirty").unwrap();
    assert!(worktree::remove(&r, &path, false, true).is_err(), "a dirty worktree needs force");
    worktree::remove(&r, &path, true, true).unwrap();
    assert_eq!(worktree::worktrees(&r).unwrap().len(), 1);
    assert!(!branches(&r).unwrap().iter().any(|b| b.name == "feat/new-thing"), "branch deleted");
}

/// A file named `*` is a file, not a pattern: discarding it leaves every other change alone.
#[cfg(unix)]
#[test]
fn discarding_a_file_named_star_discards_only_it() {
    let (dir, r) = repo();
    for f in ["keep.txt", "*"] {
        std::fs::write(dir.path().join(f), "1\n").unwrap();
    }
    sh(dir.path(), &["add", "-A"]);
    sh(dir.path(), &["commit", "-q", "-m", "init"]);
    for f in ["keep.txt", "*"] {
        std::fs::write(dir.path().join(f), "2\n").unwrap();
    }
    std::fs::write(dir.path().join("new.txt"), "n\n").unwrap();
    discard(&r, &["*".into()], &[]).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join("keep.txt")).unwrap(), "2\n", "keep.txt keeps its change");
    assert_eq!(std::fs::read_to_string(dir.path().join("*")).unwrap(), "1\n");
    // An untracked `*` beside other untracked files.
    std::fs::remove_file(dir.path().join("*")).unwrap();
    sh(dir.path(), &["rm", "-q", "--cached", "--", ":(literal)*"]);
    std::fs::write(dir.path().join("*"), "x\n").unwrap();
    discard(&r, &[], &["*".into()]).unwrap();
    assert!(!dir.path().join("*").exists());
    assert!(dir.path().join("new.txt").exists(), "other untracked files stay");
}

/// A worktree made from `origin/main` (the default base in a clone) pushes under its own name
/// instead of failing on a plain `git push`; one already tracking `origin/main` is fixed too.
#[test]
fn a_worktree_from_the_remote_default_pushes_under_its_own_name() {
    let (dir, r) = repo();
    let remote = tempfile::tempdir().unwrap();
    sh(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    std::fs::write(dir.path().join("a"), "1").unwrap();
    commit(&r, "one", false, true).unwrap();
    sh(dir.path(), &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    sync(&r, SyncOp::Push).unwrap();
    sh(dir.path(), &["fetch", "-q", "origin"]);

    let path = worktree::default_dir(&r, "", "feat/x");
    worktree::add(&r, &path, "feat/x", "origin/main").unwrap();
    let wt = Repo::new(Location::Local, path.clone());
    assert_eq!(status(&wt).unwrap().upstream, None, "not tracking origin/main");
    std::fs::write(std::path::Path::new(&path).join("b"), "2").unwrap();
    commit(&wt, "two", false, true).unwrap();
    sync(&wt, SyncOp::Push).unwrap();
    assert_eq!(status(&wt).unwrap().upstream.as_deref(), Some("origin/feat/x"));

    // A branch an older oah made, tracking origin/main.
    sh(dir.path(), &["branch", "-q", "--track", "old", "origin/main"]);
    let old_path = worktree::default_dir(&r, "", "old");
    worktree::add(&r, &old_path, "old", "").unwrap();
    let old = Repo::new(Location::Local, old_path.clone());
    std::fs::write(std::path::Path::new(&old_path).join("c"), "3").unwrap();
    commit(&old, "three", false, true).unwrap();
    sync(&old, SyncOp::Push).unwrap();
    assert_eq!(status(&old).unwrap().upstream.as_deref(), Some("origin/old"));
    worktree::remove(&r, &path, true, false).unwrap();
    worktree::remove(&r, &old_path, true, false).unwrap();
}

/// A project that's a folder inside a bigger repo sees its own changes, with paths from the
/// folder, and staging, diffs and commit-all work on them.
#[test]
fn a_project_inside_a_bigger_repo() {
    let (dir, _) = repo();
    std::fs::create_dir_all(dir.path().join("frontend/src")).unwrap();
    std::fs::create_dir_all(dir.path().join("backend")).unwrap();
    std::fs::write(dir.path().join("frontend/src/x.ts"), "1\n").unwrap();
    std::fs::write(dir.path().join("backend/y.rs"), "1\n").unwrap();
    sh(dir.path(), &["add", "-A"]);
    sh(dir.path(), &["commit", "-q", "-m", "init"]);
    std::fs::write(dir.path().join("frontend/src/x.ts"), "2\n").unwrap();
    std::fs::write(dir.path().join("frontend/new.ts"), "n\n").unwrap();
    std::fs::write(dir.path().join("backend/y.rs"), "2\n").unwrap();

    let r = Repo::new(Location::Local, dir.path().join("frontend").to_string_lossy());
    let st = status(&r).unwrap();
    let paths = |c: &[Change]| c.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths(&st.unstaged), ["src/x.ts"]);
    assert_eq!(paths(&st.untracked), ["new.ts"]);
    assert_eq!((st.unstaged[0].added, st.unstaged[0].removed), (1, 1));
    assert!(diff(&r, "src/x.ts", false, false).unwrap().contains("+2"));
    stage(&r, &["src/x.ts".into()]).unwrap();
    assert_eq!(paths(&status(&r).unwrap().staged), ["src/x.ts"]);
    commit(&r, "frontend", false, true).unwrap();
    let st = status(&r).unwrap();
    assert!(st.unstaged.is_empty() && st.untracked.is_empty());
    // The other folder's change wasn't swept into the commit.
    let top = Repo::new(Location::Local, dir.path().to_string_lossy());
    assert_eq!(paths(&status(&top).unwrap().unstaged), ["backend/y.rs"]);
}

#[test]
fn commit_message_diffs_skip_generated_files_and_cap_each_file() {
    let diff = format!(
        "diff --git a/Cargo.lock b/Cargo.lock\n@@ -1 +1 @@\n-{}\n+{}\ndiff --git a/src/a.rs b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/big.txt b/big.txt\n@@ -1 +1 @@\n+{}\n",
        "x".repeat(50_000),
        "y".repeat(50_000),
        "é".repeat(10_000)
    );
    let out = trim_diff_for_message(&diff, 1000);
    assert!(out.contains("diff --git a/Cargo.lock b/Cargo.lock\n(generated file changed; not shown)"));
    assert!(!out.contains("xxxx"));
    assert!(out.contains("diff --git a/src/a.rs b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\n"), "{out}");
    assert!(out.contains("(rest of this file's changes cut)"));
    assert!(out.len() < 2_000, "{}", out.len());
}
