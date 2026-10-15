//! Display-only hook; unknown returned shapes preserve upstream presentation.
use super::super::cutex_mcp_display;
use super::*;

pub(super) fn render(cell: &McpToolCallCell, width: u16) -> Option<Vec<Line<'static>>> {
    if !cell.compact_result_consistent {
        return None;
    }
    let mut running = cutex_mcp_display::title(&cell.invocation)?;
    if cell.invocation.server == "cutex_job"
        && let Some(id) = cell
            .invocation
            .arguments
            .as_ref()
            .and_then(|args| args["jobId"].as_str())
    {
        let verb = match cell.invocation.tool.as_str() {
            "query" => "Querying job",
            "read_output" => "Reading job output",
            "cancel" => "Cancelling job",
            _ => return None,
        };
        let label = cell
            .job_label
            .clone()
            .unwrap_or_else(|| super::super::job_labels::short_id(id));
        running = format!("{verb} · {label}");
    }
    let mut summary = None;
    let (header, bullet) = match &cell.result {
        None => (running, "•".dim()),
        Some(Err(_)) => (format!("Failed · {running}"), "•".red().bold()),
        Some(Ok(result)) => {
            let outcome = if result.content.len() == 1 {
                result.content[0]
                    .text()
                    .and_then(|text| cutex_mcp_display::outcome(&cell.invocation, text))
            } else {
                None
            };
            let outcome = outcome.map(|mut outcome| {
                if let Some(id) = &outcome.job_id
                    && let Some(prefix) = outcome.text.strip_suffix(id)
                {
                    let label = cell
                        .job_label
                        .clone()
                        .unwrap_or_else(|| super::super::job_labels::short_id(id));
                    // The submit prefix already contains this exact request's actionId. A
                    // historical mapping conflict must never remove that current-call fact.
                    outcome.text = format!("{prefix}{label}");
                }
                outcome
            });
            match outcome {
                Some(outcome) if outcome.uncertain => (outcome.text, "•".dim()),
                Some(outcome) if outcome.failed => {
                    let header = outcome.text.clone();
                    summary = Some(outcome);
                    (header, "•".red().bold())
                }
                Some(_) if result.is_error => return None,
                // Exact configured Bon voyage ! foreground, adapted to terminal capability.
                Some(outcome) => {
                    let header = outcome.text.clone();
                    summary = Some(outcome);
                    (
                        header,
                        "•"
                            .fg(crate::terminal_palette::rgb_color((0xE0, 0x8E, 0xB2)))
                            .bold(),
                    )
                }
                None => return None,
            }
        }
    };
    let body = if let Some(summary) = &summary {
        if cell.invocation.server == "cutex"
            && (cell.invocation.tool == "send"
                || cutex_mcp_display::previews_detail(&cell.invocation.tool))
        {
            summary
                .detail
                .iter()
                .map(|text| {
                    super::super::event_presentation::preview(
                        text,
                        2,
                        usize::from(width).saturating_sub(2).max(1),
                    )
                })
                .collect()
        } else {
            summary.detail.clone()
        }
    } else {
        match &cell.result {
            Some(Ok(result)) => result
                .content
                .iter()
                .map(|block| {
                    super::super::event_presentation::preview(
                        &block.render(),
                        TOOL_CALL_MAX_LINES,
                        usize::from(width).saturating_sub(2).max(1),
                    )
                })
                .collect(),
            Some(Err(error)) => vec![super::super::event_presentation::preview(
                &format!("Error: {error}"),
                TOOL_CALL_MAX_LINES,
                usize::from(width).saturating_sub(2).max(1),
            )],
            None => Vec::new(),
        }
    };
    let entity = if cell.invocation.server == "cutex_job" {
        super::super::custom_event_style::Entity::Job
    } else if cell.invocation.tool.contains("task") {
        super::super::custom_event_style::Entity::Task
    } else {
        super::super::custom_event_style::Entity::Agent
    };
    let mut lines = super::super::custom_event_style::header(
        &header,
        entity,
        summary
            .as_ref()
            .and_then(|s| s.occurred_at_millis)
            .and_then(super::super::custom_event_style::timestamp),
        bullet.clone(),
        width,
    );
    let details = super::super::event_presentation::render_event(None, &body, Some(bullet), width);
    lines.extend(
        details
            .into_iter()
            .map(|line| if summary.is_some() { line.dim() } else { line }),
    );
    if let Some(output) = summary.and_then(|summary| summary.output) {
        lines.extend(output.lines(width));
    }
    Some(lines)
}
