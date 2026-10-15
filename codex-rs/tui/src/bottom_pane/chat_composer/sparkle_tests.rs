//! Covers the restored model-gated Astra starfield and its rendering safeguards.

use super::*;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::BottomPaneParams;
use crate::render::renderable::Renderable;
use crate::terminal_palette::with_test_default_colors;
use crate::terminal_probe::DefaultColors;
use crate::tui::FrameRequester;
use ratatui::layout::Position;

use super::field::DOTS;

fn pane() -> BottomPane {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    BottomPane::new(BottomPaneParams {
        app_event_tx: AppEventSender::new(tx),
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Codex to do anything".into(),
        disable_paste_burst: true,
        animations_enabled: true,
        effects: Default::default(),
        skills: None,
    })
}

fn palette<T>(render: impl FnOnce() -> T) -> T {
    with_test_default_colors(
        DefaultColors {
            fg: (230, 216, 255),
            bg: (36, 27, 53),
        },
        render,
    )
}

fn enabled() -> Tui {
    Tui {
        animations: true,
        effects: Default::default(),
        ..Tui::default()
    }
}

fn draw(composer: &ChatComposer, width: u16, now: Instant) -> (Buffer, Rect) {
    let area = Rect::new(
        /*x*/ 0,
        /*y*/ 0,
        width,
        composer.desired_height(width),
    );
    let layout = composer.layout_with_options(area, Default::default());
    let phase = composer.sparkle.phase.replace(Phase::Unarmed);
    let mut buffer = Buffer::empty(area);
    composer.render(area, &mut buffer);
    composer.sparkle.phase.set(phase);
    composer.render_sparkle_at(
        layout.composer,
        layout.textarea,
        composer.cursor_pos(area),
        now,
        &mut buffer,
    );
    (buffer, layout.textarea)
}

fn dots(buffer: &Buffer) -> Vec<(u16, u16)> {
    buffer
        .content
        .iter()
        .enumerate()
        .filter(|&(_, cell)| DOTS.contains(&cell.symbol()))
        .map(|(index, _)| {
            let (x, y) = buffer.pos_of(index);
            (x, y)
        })
        .collect()
}

fn text(buffer: &Buffer) -> String {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(pane: &mut BottomPane, key: KeyCode) -> InputResult {
    pane.handle_key_event(KeyEvent::new(key, KeyModifiers::NONE))
}

#[test]
fn astra_sparkle_persists_and_protects_composer_content() {
    palette(|| {
        let now = Instant::now();
        let mut pane = pane();
        let (plain, textarea) = draw(&pane.composer, /*width*/ 80, now);
        let cursor = pane.composer.cursor_pos(plain.area);
        pane.sync_model_sparkle("gpt-6-astra", &enabled());

        let (active, _) = draw(&pane.composer, /*width*/ 80, now);
        let (later, _) = draw(
            &pane.composer,
            /*width*/ 80,
            now + Duration::from_secs(/*secs*/ 30),
        );
        assert!(!dots(&active).is_empty());
        assert!(!dots(&later).is_empty());

        let (typed, typed_textarea) = {
            key(&mut pane, KeyCode::Char('x'));
            draw(&pane.composer, /*width*/ 80, now)
        };
        assert!(text(&typed).contains('x'));
        assert!(!dots(&typed).is_empty());
        assert!(dots(&typed).iter().all(|&(x, y)| {
            cursor != Some((x, y)) && Some((x, y)) != pane.composer.cursor_pos(typed.area)
        }));
        let protected_line = Rect::new(
            typed.area.x,
            typed_textarea.y,
            typed.area.width,
            typed_textarea.height,
        );
        assert!(
            dots(&typed)
                .iter()
                .all(|&(x, y)| !protected_line.contains(Position::new(x, y)))
        );

        let protected = Rect::new(
            textarea.x,
            textarea.y,
            pane.composer.placeholder_text.width() as u16,
            /*height*/ 1,
        );
        assert!(
            dots(&active)
                .iter()
                .all(|&(x, y)| !protected.contains(Position::new(x, y)))
        );
    });
}

#[test]
fn effective_model_and_settings_control_the_persistent_sparkle() {
    palette(|| {
        let now = Instant::now();
        let mut pane = pane();
        pane.sync_model_sparkle("gpt-5.5", &enabled());
        assert!(dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());

        pane.sync_model_sparkle("gpt-6-astra", &enabled());
        assert!(!dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());

        pane.sync_model_sparkle("gpt-5.5", &enabled());
        assert!(dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());

        pane.sync_model_sparkle("gpt-6-astra", &enabled());
        assert!(!dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());

        let disabled = Tui {
            effects: codex_config::types::TuiEffects {
                starfield: false,
                ..Default::default()
            },
            ..enabled()
        };
        pane.sync_model_sparkle("gpt-6-astra", &disabled);
        assert!(dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());
    });
}

#[test]
fn temporary_focus_and_overlays_hide_the_sparkle_without_ending_it() {
    palette(|| {
        let now = Instant::now();
        let mut pane = pane();
        pane.sync_model_sparkle("gpt-6-astra", &enabled());
        assert!(!dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());

        pane.composer.set_task_running(/*running*/ true);
        assert!(!dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());
        pane.composer.set_task_running(/*running*/ false);
        assert!(!dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());

        pane.set_sparkle_terminal_focus(/*focused*/ false);
        assert!(dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());
        pane.set_sparkle_terminal_focus(/*focused*/ true);
        assert!(!dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());

        let help = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT);
        pane.handle_key_event(help);
        assert!(dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());
        pane.handle_key_event(help);
        assert!(!dots(&draw(&pane.composer, /*width*/ 80, now).0).is_empty());
    });
}
