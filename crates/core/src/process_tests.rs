use super::*;
use std::time::{Duration, Instant};

/// The scheduler state `/proc` reports for `pid`; `None` once it is gone.
fn state(pid: u32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit(')').next()?.trim_start().chars().next()
}

/// Dead means gone or a zombie: a killed child stays one until someone reaps it.
fn dies_within(pid: u32, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if matches!(state(pid), None | Some('Z')) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A shell and the `sleep` it started, which share the guard's process group.
fn shell_with_a_child() -> (ChildGuard, u32, u32) {
    let process = ExternalProcess::spawn("sh", &["-c", "sleep 30 & wait"]).unwrap();
    let leader = process.id().unwrap();
    let mut children = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while children.trim().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        children = std::fs::read_to_string(format!("/proc/{leader}/task/{leader}/children")).unwrap_or_default();
    }
    let child = children.split_whitespace().next().expect("the shell started its sleep").parse().unwrap();
    (ChildGuard::new(process), leader, child)
}

/// The leak this guards against: the kill was a task spawned on drop, which a runtime
/// shutting down right after never ran, so the whole group outlived the guard.
#[test]
fn dropping_the_guard_kills_the_group_even_as_the_runtime_ends() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (guard, leader, child) = runtime.block_on(async { shell_with_a_child() });
    assert_eq!(state(child).map(|s| s != 'Z'), Some(true), "the sleep is running before the drop");
    runtime.block_on(async { drop(guard) });
    drop(runtime);
    assert!(dies_within(leader, Duration::from_secs(2)), "the group's leader outlived its guard");
    assert!(dies_within(child, Duration::from_secs(2)), "the leader's child outlived its guard");
}

/// A guard dropped where no runtime runs still kills, and does not panic for want of one.
#[test]
fn a_guard_dropped_outside_a_runtime_still_kills() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (guard, leader, child) = runtime.block_on(async { shell_with_a_child() });
    drop(runtime);
    drop(guard);
    assert!(dies_within(leader, Duration::from_secs(2)), "the group's leader outlived its guard");
    assert!(dies_within(child, Duration::from_secs(2)), "the leader's child outlived its guard");
}

#[test]
fn a_disarmed_guard_leaves_the_process_running() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (mut guard, leader, _) = shell_with_a_child();
        let mut process = guard.disarm().expect("armed");
        drop(guard);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(state(leader).map(|s| s != 'Z'), Some(true), "a disarmed guard killed its process");
        process.kill().await.unwrap();
    });
}
