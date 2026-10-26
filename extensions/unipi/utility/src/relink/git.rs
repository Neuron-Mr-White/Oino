//! Git worktree links after a project move (unipi `src/move/git.ts`).

/// Worktree roots in `git worktree list --porcelain` output.
#[must_use]
pub fn worktrees(porcelain: &str) -> Vec<String> {
    porcelain
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .map(ToOwned::to_owned)
        .collect()
}

/// A listed worktree lives at (or under) the old root.
#[must_use]
pub fn mentions_old_root(porcelain: &str, old_root: &str) -> bool {
    worktrees(porcelain)
        .iter()
        .any(|w| w == old_root || w.starts_with(&format!("{old_root}/")))
}

/// Plan line.
#[must_use]
pub fn description(new_root: &str, common_dir: &str) -> String {
    format!(
        "git -C {new_root} worktree repair ({common_dir}); repair worktree links to moved repository"
    )
}

async fn git(args: &[&str]) -> Option<String> {
    let r = oino_extension::process::exec("git", args, None, Some(10_000))
        .await
        .ok()?;
    (r.code == Some(0)).then_some(r.stdout)
}

/// `git worktree repair` is needed: `Some(description)`.
pub async fn scan(old_root: &str, new_root: &str) -> Option<String> {
    let list = git(&["-C", new_root, "worktree", "list", "--porcelain"]).await?;
    let trees = worktrees(&list);
    if trees.len() < 2 && !mentions_old_root(&list, old_root) {
        return None;
    }
    let mut needs = mentions_old_root(&list, old_root);
    if !needs {
        // A linked worktree whose `.git` file still points into the old root no longer
        // resolves its git dir.
        for wt in trees.iter().skip(1) {
            if super::exists(wt).await == Some(true)
                && git(&["-C", wt, "rev-parse", "--git-dir"]).await.is_none()
            {
                needs = true;
                break;
            }
        }
    }
    if !needs {
        return None;
    }
    let common = git(&["-C", new_root, "rev-parse", "--git-common-dir"])
        .await
        .unwrap_or_default();
    Some(description(new_root, common.trim()))
}

/// Runs the repair.
///
/// # Errors
///
/// The git failure text.
pub async fn repair(new_root: &str) -> Result<(), String> {
    let r = oino_extension::process::exec(
        "git",
        &["-C", new_root, "worktree", "repair"],
        None,
        Some(30_000),
    )
    .await
    .map_err(|e| e.to_string())?;
    if r.code == Some(0) {
        Ok(())
    } else {
        Err(r.stderr.trim().to_owned())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_parsing() {
        let p = "worktree /new/repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /old/repo/wt\nHEAD def\n";
        assert_eq!(worktrees(p), ["/new/repo", "/old/repo/wt"]);
        assert!(mentions_old_root(p, "/old/repo"));
        assert!(!mentions_old_root(p, "/old/rep"));
        assert!(description("/n", ".git").contains("worktree repair (.git)"));
    }
}
