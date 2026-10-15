//! Ordered, single-pass display substitutions. Never changes a real working directory.
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Alias {
    from: String,
    to: String,
    #[serde(default = "default_bold")]
    bold: bool,
}
fn default_bold() -> bool {
    true
}

fn location(value: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    value.match_indices(needle).find_map(|(start, _)| {
        let end = start + needle.len();
        let boundary = |c| c == '/' || c == '\\';
        ((start == 0 || value[..start].ends_with(boundary))
            && (end == value.len() || value[end..].starts_with(boundary)))
        .then_some(start)
    })
}
fn valid(rule: &Alias) -> bool {
    !rule.to.is_empty() && rule.to.len() <= 128 && !rule.to.chars().any(char::is_control)
}
pub(crate) fn replace(value: &str, rules: &[Alias]) -> String {
    let normalized = value.replace('\\', "/");
    for rule in rules.iter().filter(|r| valid(r)) {
        let from = rule.from.replace('\\', "/");
        let from = from.trim_end_matches('/');
        if let Some(start) = location(&normalized, from) {
            // Slash normalization preserves byte offsets (also on Windows).
            return format!(
                "{}{}{}",
                &value[..start],
                rule.to,
                &value[start + from.len()..]
            );
        }
    }
    value.to_owned()
}
pub(crate) fn styled(value: &str, style: Style, rules: &[Alias]) -> Vec<Span<'static>> {
    for rule in rules.iter().filter(|r| valid(r) && r.bold) {
        if let Some(start) = location(value, &rule.to) {
            return vec![
                Span::styled(value[..start].to_owned(), style),
                Span::styled(
                    rule.to.clone(),
                    style
                        .add_modifier(Modifier::BOLD)
                        .remove_modifier(Modifier::DIM),
                ),
                Span::styled(value[start + rule.to.len()..].to_owned(), style),
            ];
        }
    }
    vec![Span::styled(value.to_owned(), style)]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_once_and_component_boundaries() {
        let rules = vec![
            Alias {
                from: "~/Developments".into(),
                to: "$DEV".into(),
                bold: true,
            },
            Alias {
                from: "agent-home".into(),
                to: "$A".into(),
                bold: true,
            },
        ];
        assert_eq!(
            replace("~/Developments/x/agent-home/a", &rules),
            "$DEV/x/agent-home/a"
        );
        assert_eq!(
            replace("~/Projects/x/agent-home/a", &rules),
            "~/Projects/x/$A/a"
        );
        assert_eq!(
            replace("~/Developments-extra/agent-home-old", &rules),
            "~/Developments-extra/agent-home-old"
        );
        assert_eq!(
            replace(r"C:\Projects\agent-home\a", &rules),
            r"C:\Projects\$A\a"
        );
        let spans = styled("$DEV/中文", Style::default(), &rules);
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(
            spans.iter().map(|s| s.content.as_ref()).collect::<String>(),
            "$DEV/中文"
        );
    }
}
