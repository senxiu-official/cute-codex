//! Shared colors and occurrence-time headers for Cutex presentation only.
use super::*;
use chrono::Datelike;

#[derive(Clone, Copy)]
pub(super) enum Entity {
    Agent,
    Task,
    Job,
}

pub(super) fn bullet() -> Span<'static> {
    "•"
        .fg(crate::terminal_palette::rgb_color((0xE0, 0x8E, 0xB2)))
        .bold()
}

pub(super) fn timestamp(epoch_millis: i64) -> Option<String> {
    let time = chrono::DateTime::from_timestamp_millis(epoch_millis)?.with_timezone(&chrono::Local);
    Some(format_time(time, chrono::Local::now()))
}

fn format_time(
    time: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let format = if time.year() != now.year() {
        "%Y-%m-%d %H:%M:%S"
    } else if time.date_naive() != now.date_naive() {
        "%m-%d %H:%M:%S"
    } else {
        "%H:%M:%S"
    };
    time.format(format).to_string()
}

pub(super) fn header(
    text: &str,
    entity: Entity,
    time: Option<String>,
    bullet: Span<'static>,
    width: u16,
) -> Vec<Line<'static>> {
    let clean = super::messages::sanitize_user_text(text.into());
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let color = crate::terminal_palette::rgb_color(match entity {
        Entity::Agent => (0xF7, 0xB3, 0xCD),
        Entity::Task => (0x74, 0xBA, 0xC3),
        Entity::Job => (0xD9, 0xB4, 0x5F),
    });
    let mut spans = Vec::new();
    let parts = clean.split(" · ").collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            spans.push(" · ".bold());
        }
        let message = [
            "Sent message to ",
            "Sending message to ",
            "Received message from ",
        ]
        .iter()
        .find_map(|prefix| part.strip_prefix(prefix).map(|name| (*prefix, name)));
        if let Some((prefix, name)) = message {
            spans.extend([prefix.to_owned().bold(), name.to_owned().fg(color).bold()]);
        } else if index > 0 && !matches!(entity, Entity::Agent)
            || index == 1 && !parts[0].contains("message ")
        {
            spans.push((*part).to_owned().fg(color).bold());
        } else {
            spans.push((*part).to_owned().bold());
        }
    }
    if let Some(time) = time {
        spans.extend([" · ".dim(), time.dim()]);
    }
    adaptive_wrap_line(
        &Line::from(spans),
        RtOptions::new(usize::from(width).max(1))
            .initial_indent(Line::from(vec![bullet, " ".into()]))
            .subsequent_indent("  ".into()),
    )
    .iter()
    .map(line_to_static)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    #[test]
    fn date_is_present_only_when_needed() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 14, 12, 0, 0)
            .unwrap();
        let examples = [
            (2026, 9, 14, "10:02:07"),
            (2026, 9, 13, "09-13 10:02:07"),
            (2025, 9, 14, "2025-09-14 10:02:07"),
        ];
        for (year, month, day, expected) in examples {
            let time = chrono::Local
                .with_ymd_and_hms(year, month, day, 10, 2, 7)
                .unwrap();
            assert_eq!(format_time(time, now), expected);
        }
    }
}
