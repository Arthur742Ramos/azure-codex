//! Utility to compute the current Git diff for the working directory.
//!
//! The implementation mirrors the behaviour of the TypeScript version in
//! `codex-cli`: it returns the diff for tracked changes as well as any
//! untracked files. When the current directory is not inside a Git
//! repository, the function returns `Ok((false, String::new()))`.

use std::io;
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Semaphore;

#[derive(Clone, Debug)]
struct CapturedOutput {
    stdout: String,
    truncated: bool,
}

fn clamp_char_boundary(text: &str, mut idx: usize) -> usize {
    idx = idx.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx = idx.saturating_sub(1);
    }
    idx
}

fn truncate_at_newline(text: &str, max_bytes: usize) -> usize {
    if text.len() <= max_bytes {
        return text.len();
    }

    let max_bytes = clamp_char_boundary(text, max_bytes);
    let candidate = &text[..max_bytes];
    candidate
        .rfind('\n')
        .map(|idx| idx.saturating_add(1))
        .unwrap_or(candidate.len())
}

async fn run_git_capture_stdout_limited(
    args: &[&str],
    max_bytes: usize,
    allow_exit_code_1: bool,
) -> io::Result<CapturedOutput> {
    let mut child = Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    let Some(mut stdout) = child.stdout.take() else {
        return Err(io::Error::other("failed to capture git stdout"));
    };

    let mut buf: Vec<u8> = Vec::with_capacity(max_bytes.min(64 * 1024));
    let mut chunk = [0u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let n = stdout.read(&mut chunk).await?;
        if n == 0 {
            break;
        }

        let remaining = max_bytes.saturating_sub(buf.len());
        if remaining == 0 {
            truncated = true;
            break;
        }

        if n <= remaining {
            buf.extend_from_slice(&chunk[..n]);
        } else {
            buf.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
    }

    if truncated {
        let _ = child.kill().await;
    }
    let status = child.wait().await?;

    if !(truncated || status.success() || (allow_exit_code_1 && status.code() == Some(1))) {
        return Err(io::Error::other(format!(
            "git {args:?} failed with status {status}"
        )));
    }

    let mut stdout = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
        let kept = truncate_at_newline(&stdout, max_bytes);
        stdout.truncate(kept);
    }

    Ok(CapturedOutput { stdout, truncated })
}

async fn run_git_capture_diff_limited(
    args: &[&str],
    max_bytes: usize,
) -> io::Result<CapturedOutput> {
    run_git_capture_stdout_limited(args, max_bytes, true).await
}

/// Return value of [`get_git_diff`].
///
/// * `bool` - Whether the current working directory is inside a Git repo.
/// * `String` - The concatenated diff (may be empty).
pub(crate) async fn get_git_diff() -> io::Result<(bool, String)> {
    const MAX_DIFF_BYTES: usize = 1024 * 1024;
    const MAX_UNTRACKED_FILES: usize = 200;
    const MAX_UNTRACKED_FILE_BYTES: usize = 128 * 1024;
    const MAX_GIT_CONCURRENCY: usize = 4;

    // First check if we are inside a Git repository.
    if !inside_git_repo().await? {
        return Ok((false, String::new()));
    }

    let tracked = run_git_capture_diff_limited(&["diff", "--color"], MAX_DIFF_BYTES).await?;
    if tracked.truncated {
        return Ok((
            true,
            format!(
                "{}\n... (diff truncated; run `git diff` for full output)\n",
                tracked.stdout
            ),
        ));
    }

    let untracked_output = run_git_capture_stdout_limited(
        &["ls-files", "--others", "--exclude-standard"],
        MAX_DIFF_BYTES,
        false,
    )
    .await?;
    let null_device: &Path = if cfg!(windows) {
        Path::new("NUL")
    } else {
        Path::new("/dev/null")
    };

    let null_path = null_device.to_str().unwrap_or("/dev/null").to_string();

    let untracked_files: Vec<String> = untracked_output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let extra_untracked_files = untracked_files.len().saturating_sub(MAX_UNTRACKED_FILES);
    let untracked_files = untracked_files
        .into_iter()
        .take(MAX_UNTRACKED_FILES)
        .collect::<Vec<_>>();

    let semaphore = std::sync::Arc::new(Semaphore::new(MAX_GIT_CONCURRENCY));
    let mut join_set: tokio::task::JoinSet<(usize, String, io::Result<CapturedOutput>)> =
        tokio::task::JoinSet::new();

    for (idx, file) in untracked_files.iter().cloned().enumerate() {
        let semaphore = semaphore.clone();
        let null_path = null_path.clone();
        join_set.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            let args = [
                "diff",
                "--color",
                "--no-index",
                "--",
                null_path.as_str(),
                file.as_str(),
            ];
            let res = run_git_capture_diff_limited(&args, MAX_UNTRACKED_FILE_BYTES).await;
            (idx, file, res)
        });
    }

    let mut results: Vec<Option<(String, CapturedOutput)>> = vec![None; untracked_files.len()];
    while let Some(res) = join_set.join_next().await {
        let Ok((idx, file, res)) = res else {
            continue;
        };
        match res {
            Ok(output) => results[idx] = Some((file, output)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }

    let mut untracked_diff = String::new();
    for (idx, file) in untracked_files.iter().enumerate() {
        let Some((result_file, output)) = results[idx].take() else {
            continue;
        };
        // Keep order stable even if tasks completed out-of-order.
        if &result_file != file {
            continue;
        }

        untracked_diff.push_str(&output.stdout);
        if output.truncated {
            untracked_diff.push_str(&format!(
                "\n... (truncated diff output for {file}; run `git diff --no-index -- /dev/null {file}` for full output)\n"
            ));
        }
    }

    if extra_untracked_files > 0 {
        untracked_diff.push_str(&format!(
            "\n... (+{extra_untracked_files} untracked files omitted)\n"
        ));
    }

    let mut combined = tracked.stdout;
    combined.push_str(&untracked_diff);
    if combined.len() > MAX_DIFF_BYTES {
        let kept = truncate_at_newline(&combined, MAX_DIFF_BYTES);
        combined.truncate(kept);
        combined.push_str("\n... (diff truncated; run `git diff` for full output)\n");
    }

    Ok((true, combined))
}

/// Determine if the current directory is inside a Git repository.
async fn inside_git_repo() -> io::Result<bool> {
    let status = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    match status {
        Ok(s) if s.success() => Ok(true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false), // git not installed
        Err(e) => Err(e),
    }
}
