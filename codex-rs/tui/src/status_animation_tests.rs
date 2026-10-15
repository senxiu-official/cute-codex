use super::*;
use pretty_assertions::assert_eq;
use ratatui::style::Color;
use serde_json::json;

#[test]
#[allow(clippy::disallowed_methods)] // Verify user-configured exact RGB colors.
fn per_character_colors_padding_and_timing() {
    let animation = Animation::parse(json!({"width":12,"repeat":true,"frames":[
        {"duration_ms":250,"spans":[{"text":"旅","style":{"fg":"#F7B3CD"}},{"text":"tu","style":{"fg":"#74BAC3","underlined":true}},{"text":"▏","style":{"fg":"#D9B45F"}}]},
        {"duration_ms":500,"spans":[{"text":"旅途","style":{"fg":"#E08EB2"}},{"text":" "}]}
    ]}), Style::default()).unwrap();
    let (frame, next) = animation.sample(Duration::from_millis(249));
    assert_eq!(next, Some(Duration::from_millis(1)));
    assert_eq!(frame[0].style.fg, Some(Color::Rgb(247, 179, 205)));
    assert_eq!(frame[1].style.fg, Some(Color::Rgb(116, 186, 195)));
    assert_eq!(frame.iter().map(Span::width).sum::<usize>(), 12);
    let snapshot = [0, 250, 750]
        .map(|ms| {
            let (spans, next) = animation.sample(Duration::from_millis(ms));
            format!("{ms}ms next={next:?}: {spans:?}")
        })
        .join("\n");
    insta::assert_snapshot!("pinyin_frames_with_individual_colors", snapshot);
}

#[test]
fn hundred_frames_stop_or_repeat_without_high_frequency_ticks() {
    for repeat in [true, false] {
        let frames: Vec<_> = (0..100)
            .map(|i| json!({"duration_ms":300,"spans":[{"text":format!("旅途愉快 {i}")}]}))
            .collect();
        let animation = Animation::parse(
            json!({"width":16,"repeat":repeat,"frames":frames}),
            Style::default(),
        )
        .unwrap();
        let (frame, next) = animation.sample(Duration::from_secs(30));
        assert_eq!(
            frame[0].content.as_ref(),
            if repeat {
                "旅途愉快 0"
            } else {
                "旅途愉快 99"
            }
        );
        assert_eq!(next, repeat.then_some(Duration::from_millis(300)));
    }
}
