use chrono::{TimeZone, Utc};
use std::{fs, path::Path, process::Command};

byond_fn!(fn rg_git_revparse(rev) {
    let output = Command::new("git")
        .args(["rev-parse", rev])
        .output()
        .ok()?;
    if !output.status.success() {
        return Some(format!(
            "failed to open repository: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
});

byond_fn!(fn rg_git_commit_date(rev, format) {
    // git log -1 --format=%ct <rev> gives unix timestamp
    let output = Command::new("git")
        .args(["log", "-1", "--format=%ct", rev])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let timestamp_str = String::from_utf8_lossy(&output.stdout);
    let commit_time: i64 = timestamp_str.trim().parse().ok()?;
    let datetime = Utc.timestamp_opt(commit_time, 0).latest()?;
    Some(datetime.format(format).to_string())
});

byond_fn!(fn rg_git_commit_date_head(format) {
    let head_log_path = Path::new(".git").join("logs").join("HEAD");
    let head_log = fs::metadata(&head_log_path).ok()?;
    if !head_log.is_file() {
        return None;
    }
    let log_entries = fs::read_to_string(&head_log_path).ok()?;
    let last_entry = log_entries
        .split('\n')
        .rev()
        .find(|line| !line.is_empty())?
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    if last_entry.len() < 5 { // 5 is the timestamp
        return None;
    }
    let datetime = Utc.timestamp_opt(last_entry[4].parse().ok()?, 0).latest()?;
    Some(datetime.format(format).to_string())
});
