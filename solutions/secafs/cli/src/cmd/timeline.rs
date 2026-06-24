use secafs_sdk::{toolcalls::ToolCall, SecAFSOptions, ToolCalls};
use anyhow::{Context, Result as AnyhowResult};
use chrono::TimeZone;
use std::io::Write;
use std::str::FromStr;

use crate::cmd::init::open_secafs;

/// Output format for timeline display
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Json,
}

impl FromStr for OutputFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "table" => Ok(OutputFormat::Table),
            "json" => Ok(OutputFormat::Json),
            _ => anyhow::bail!("Invalid format: {}", s),
        }
    }
}

/// Options for the timeline command
#[derive(Debug, Clone)]
pub struct TimelineOptions {
    pub limit: i64,
    pub filter: Option<String>,
    pub status: Option<String>,
    pub format: String,
}

/// Display agent action timeline from tool call audit log
pub async fn show_timeline(
    stdout: &mut impl Write,
    id_or_path: &str,
    options: &TimelineOptions,
) -> AnyhowResult<()> {
    let agent_options = SecAFSOptions::resolve(id_or_path)?;

    let secafs_inst = open_secafs(agent_options).await?;

    let toolcalls = ToolCalls::from_pool(secafs_inst.get_pool())
        .await
        .context("Failed to create tool calls tracker")?;

    // Query tool calls
    let mut calls = toolcalls
        .recent(Some(options.limit))
        .await
        .context("Failed to query tool calls")?;

    // Apply filters
    if let Some(tool_name) = &options.filter {
        calls.retain(|call| call.name == *tool_name);
    }

    if let Some(status_filter) = &options.status {
        calls.retain(|call| call.status.to_string() == *status_filter);
    }

    // Format and display
    let output_format: OutputFormat = options.format.parse()?;
    match output_format {
        OutputFormat::Table => format_table(stdout, &calls)?,
        OutputFormat::Json => format_json(stdout, &calls)?,
    }

    Ok(())
}

/// Truncate a string to a maximum length, adding ellipsis if truncated
fn truncate_with_ellipsis(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Format timestamp as YYYY-MM-DD HH:MM:SS
fn format_timestamp(timestamp: i64) -> String {
    chrono::Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| format!("Invalid timestamp: {}", timestamp))
}

/// Format tool calls in table format
fn format_table(stdout: &mut impl Write, calls: &[ToolCall]) -> AnyhowResult<()> {
    if calls.is_empty() {
        writeln!(stdout, "No tool calls found")?;
        return Ok(());
    }

    // Print header
    writeln!(
        stdout,
        "{:<4} {:<20} {:<10} {:>10} {:<20}",
        "ID", "TOOL", "STATUS", "DURATION", "STARTED"
    )?;

    // Print rows
    for call in calls {
        let tool_name = truncate_with_ellipsis(&call.name, 20);
        let status = call.status.to_string();
        let duration = call
            .duration_ms
            .map(|ms| format!("{}ms", ms))
            .unwrap_or_else(|| String::from("--"));
        let timestamp = format_timestamp(call.started_at);

        writeln!(
            stdout,
            "{:<4} {:<20} {:<10} {:>10} {:<20}",
            call.id, tool_name, status, duration, timestamp
        )?;
    }

    Ok(())
}

/// Format tool calls as JSON
fn format_json(stdout: &mut impl Write, calls: &[ToolCall]) -> AnyhowResult<()> {
    let json =
        serde_json::to_string_pretty(calls).context("Failed to serialize tool calls to JSON")?;
    writeln!(stdout, "{}", json)?;
    Ok(())
}

// Tests require a running PostgreSQL instance and are run separately.
