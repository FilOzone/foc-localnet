//! Combined Docker log output for the active foc-devnet run.

use crate::docker::core::docker_command;
use crate::docker::logs::{list_all_containers, ContainerInfo};
use crate::run_id::load_current_run_id;
use chrono::{DateTime, Utc};
use std::cmp::Ordering;
use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

const DEFAULT_FOLLOW_TAIL_LINES: usize = 100;

#[derive(Debug, Clone, Eq, PartialEq)]
struct LogEntry {
    timestamp: Option<DateTime<Utc>>,
    timestamp_text: String,
    container_name: String,
    message: String,
    container_index: usize,
    line_index: usize,
}

/// Print combined logs for the active foc-devnet run.
pub fn logs(follow: bool, tail: Option<usize>) -> Result<(), Box<dyn Error>> {
    if !follow && tail.is_some() {
        return Err("--tail can only be used with --follow".into());
    }

    let run_id = load_current_run_id()?;
    let containers = current_run_containers(&run_id)?;

    if containers.is_empty() {
        return Err(format!(
            "No foc-devnet containers found for current run ID '{}'",
            run_id
        )
        .into());
    }

    if follow {
        follow_logs(containers, tail.unwrap_or(DEFAULT_FOLLOW_TAIL_LINES))
    } else {
        print_sorted_logs(containers)
    }
}

fn current_run_containers(run_id: &str) -> Result<Vec<ContainerInfo>, Box<dyn Error>> {
    let current_run_prefix = format!("foc-{}-", run_id);
    let mut containers: Vec<ContainerInfo> = list_all_containers()?
        .into_iter()
        .filter(|container| is_current_run_container_name(&container.name, &current_run_prefix))
        .collect();

    containers.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(containers)
}

fn is_current_run_container_name(container_name: &str, current_run_prefix: &str) -> bool {
    container_name.starts_with(current_run_prefix)
}

fn print_sorted_logs(containers: Vec<ContainerInfo>) -> Result<(), Box<dyn Error>> {
    let mut entries = Vec::new();

    for (container_index, container) in containers.iter().enumerate() {
        let output = docker_command(&["logs", "--timestamps", &container.name]).map_err(|e| {
            format!(
                "Failed to get logs for container '{}': {}",
                container.name, e
            )
        })?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        entries.extend(parse_container_logs(
            &container.name,
            container_index,
            stdout.lines().chain(stderr.lines()),
        ));
    }

    sort_log_entries(&mut entries);

    for entry in entries {
        println!(
            "{} {} | {}",
            entry.timestamp_text, entry.container_name, entry.message
        );
    }

    Ok(())
}

fn parse_container_logs<'a>(
    container_name: &str,
    container_index: usize,
    lines: impl Iterator<Item = &'a str>,
) -> Vec<LogEntry> {
    lines
        .enumerate()
        .map(|(line_index, line)| parse_log_line(container_name, container_index, line_index, line))
        .collect()
}

fn parse_log_line(
    container_name: &str,
    container_index: usize,
    line_index: usize,
    line: &str,
) -> LogEntry {
    if let Some((timestamp_text, message)) = line.split_once(' ') {
        if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp_text) {
            return LogEntry {
                timestamp: Some(timestamp.with_timezone(&Utc)),
                timestamp_text: timestamp_text.to_string(),
                container_name: container_name.to_string(),
                message: message.to_string(),
                container_index,
                line_index,
            };
        }
    }

    LogEntry {
        timestamp: None,
        timestamp_text: "NO_TIMESTAMP".to_string(),
        container_name: container_name.to_string(),
        message: line.to_string(),
        container_index,
        line_index,
    }
}

fn sort_log_entries(entries: &mut [LogEntry]) {
    entries.sort_by(|a, b| match (&a.timestamp, &b.timestamp) {
        (Some(a_time), Some(b_time)) => a_time
            .cmp(b_time)
            .then_with(|| a.container_name.cmp(&b.container_name))
            .then_with(|| a.line_index.cmp(&b.line_index)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a
            .container_index
            .cmp(&b.container_index)
            .then_with(|| a.line_index.cmp(&b.line_index)),
    });
}

fn follow_logs(containers: Vec<ContainerInfo>, tail: usize) -> Result<(), Box<dyn Error>> {
    let mut children = Vec::new();
    let tail_arg = tail.to_string();

    for container in containers {
        let mut child = Command::new("docker")
            .args([
                "logs",
                "--timestamps",
                "--follow",
                "--tail",
                &tail_arg,
                &container.name,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!(
                    "Failed to follow logs for container '{}': {}",
                    container.name, e
                )
            })?;

        if let Some(stdout) = child.stdout.take() {
            let container_name = container.name.clone();
            thread::spawn(move || stream_prefixed_lines(container_name, stdout));
        }

        if let Some(stderr) = child.stderr.take() {
            let container_name = container.name.clone();
            thread::spawn(move || stream_prefixed_lines(container_name, stderr));
        }

        children.push(child);
    }

    let mut guard = ChildGuard { children };
    loop {
        let mut running_count = 0;
        for child in &mut guard.children {
            if child.try_wait()?.is_none() {
                running_count += 1;
            }
        }

        if running_count == 0 {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(250));
    }
}

fn stream_prefixed_lines<R>(container_name: String, reader: R)
where
    R: std::io::Read,
{
    let reader = BufReader::new(reader);
    for line in reader.lines() {
        match line {
            Ok(line) => print_prefixed_line(&container_name, &line),
            Err(e) => eprintln!(
                "NO_TIMESTAMP {} | failed to read log stream: {}",
                container_name, e
            ),
        }
    }
}

fn print_prefixed_line(container_name: &str, line: &str) {
    let entry = parse_log_line(container_name, 0, 0, line);
    println!(
        "{} {} | {}",
        entry.timestamp_text, entry.container_name, entry.message
    );
}

struct ChildGuard {
    children: Vec<Child>,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_log_entries_orders_by_timestamp_across_containers() {
        let mut entries = vec![
            parse_log_line("foc-run-curio-1", 1, 0, "2026-06-05T12:00:03Z curio"),
            parse_log_line("foc-run-lotus", 0, 0, "2026-06-05T12:00:01Z lotus"),
            parse_log_line("foc-run-builder", 2, 0, "2026-06-05T12:00:02Z builder"),
        ];

        sort_log_entries(&mut entries);

        let messages: Vec<&str> = entries.iter().map(|entry| entry.message.as_str()).collect();
        assert_eq!(messages, vec!["lotus", "builder", "curio"]);
    }

    #[test]
    fn test_parse_log_line_supports_nanosecond_timestamps() {
        let entry = parse_log_line(
            "foc-run-lotus",
            0,
            0,
            "2026-06-05T12:00:01.123456789Z ready",
        );

        assert!(entry.timestamp.is_some());
        assert_eq!(entry.timestamp_text, "2026-06-05T12:00:01.123456789Z");
        assert_eq!(entry.message, "ready");
    }

    #[test]
    fn test_malformed_lines_are_printable_with_deterministic_order() {
        let mut entries = vec![
            parse_log_line("foc-run-curio-1", 1, 0, "curio without timestamp"),
            parse_log_line("foc-run-lotus", 0, 0, "lotus without timestamp"),
            parse_log_line(
                "foc-run-lotus",
                0,
                1,
                "2026-06-05T12:00:01Z lotus timestamped",
            ),
        ];

        sort_log_entries(&mut entries);

        let printable: Vec<String> = entries
            .iter()
            .map(|entry| {
                format!(
                    "{} {} | {}",
                    entry.timestamp_text, entry.container_name, entry.message
                )
            })
            .collect();

        assert_eq!(
            printable,
            vec![
                "2026-06-05T12:00:01Z foc-run-lotus | lotus timestamped",
                "NO_TIMESTAMP foc-run-lotus | lotus without timestamp",
                "NO_TIMESTAMP foc-run-curio-1 | curio without timestamp",
            ]
        );
    }

    #[test]
    fn test_current_run_container_filter_prefix() {
        let run_id = "20260605T1234_TestRun";
        let current_run_prefix = format!("foc-{}-", run_id);
        let containers = [
            "foc-20260605T1234_OldRun-lotus",
            "foc-20260605T1234_TestRun-lotus",
            "foc-20260605T1234_TestRun-curio-1",
            "foc-20260605T1234_TestRun-portainer",
            "foc-observer-postgres-calibnet-1",
        ];

        let filtered: Vec<&str> = containers
            .iter()
            .copied()
            .filter(|name| is_current_run_container_name(name, &current_run_prefix))
            .collect();

        assert_eq!(
            filtered,
            vec![
                "foc-20260605T1234_TestRun-lotus",
                "foc-20260605T1234_TestRun-curio-1",
                "foc-20260605T1234_TestRun-portainer",
            ]
        );
    }

    #[test]
    fn test_tail_requires_follow() {
        let err = logs(false, Some(50)).unwrap_err().to_string();
        assert_eq!(err, "--tail can only be used with --follow");
    }
}
