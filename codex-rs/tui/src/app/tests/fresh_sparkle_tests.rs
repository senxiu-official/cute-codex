//! Covers the restored model-gated Astra starfield across thread and picker flows.

use super::*;
use crate::app::session_lifecycle::ThreadAttachPresentation;
use crate::terminal_palette::with_test_default_colors;
use crate::terminal_probe::DefaultColors;
use pretty_assertions::assert_eq;

fn started(model: &str) -> AppServerStartedThread {
    let mut session = test_thread_session(ThreadId::new(), test_path_buf("/tmp/project"));
    session.model = model.into();
    AppServerStartedThread {
        session,
        turns: Vec::new(),
        blocks_direct_input: false,
        task_tools_available: false,
    }
}

fn render_with_sparkle_palette(chat: &ChatWidget) -> String {
    with_test_default_colors(
        DefaultColors {
            fg: (230, 216, 255),
            bg: (36, 27, 53),
        },
        || render_bottom_popup(chat, /*width*/ 80),
    )
}

fn visible(chat: &ChatWidget) -> bool {
    render_with_sparkle_palette(chat)
        .chars()
        .any(|ch| "⠁⠂⠄⠈⠐⠠⡀⢀".contains(ch))
}

fn type_into(chat: &mut ChatWidget, text: &str) {
    for ch in text.chars() {
        chat.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
}

async fn choose_astra(
    app: &mut App,
    tui: &mut crate::tui::Tui,
    server: &mut AppServerSession,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    let mut preset = crate::test_support::TEST_MODEL_PRESETS[0].clone();
    preset.model = "gpt-6-astra".into();
    preset.display_name = "Astra 6".into();
    preset.show_in_picker = true;
    preset.supported_reasoning_efforts.truncate(/*len*/ 1);
    preset.default_reasoning_effort = preset.supported_reasoning_efforts[0].effort.clone();
    app.chat_widget.open_model_popup_with_presets(vec![preset]);
    app.chat_widget
        .handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));

    while let Ok(event) = events.try_recv() {
        if matches!(
            &event,
            AppEvent::UpdateModel(_)
                | AppEvent::SelectSessionModel { .. }
                | AppEvent::AstraSelectedFromModelPicker { .. }
        ) {
            if let AppEvent::AstraSelectedFromModelPicker { thread_id, .. } = &event {
                assert_eq!(Some(*thread_id), app.chat_widget.thread_id());
            }
            app.handle_event(tui, server, event).await?;
        }
    }
    assert!(!app.chat_widget.has_active_view());
    Ok(())
}

#[tokio::test]
async fn astra_sparkles_for_fresh_and_resumed_threads() -> Result<()> {
    for scenario in [
        "fresh",
        "empty_resume",
        "fork",
        "replay",
        "initial_prompt",
        "parent_owned",
    ] {
        let (mut app, _events, _ops) = make_test_app_with_channels().await;
        app.local_settings.tui.animations = true;
        app.local_settings.tui.effects.starfield = true;
        let mut tui = crate::tui::test_support::make_test_tui()?;
        let mut thread = started("gpt-6-astra");
        let presentation = if scenario == "empty_resume" {
            ThreadAttachPresentation::SessionLineage
        } else {
            ThreadAttachPresentation::Fresh
        };
        if scenario == "fork" {
            thread.session.forked_from_id = Some(ThreadId::new());
        }
        if scenario == "replay" {
            thread
                .turns
                .push(test_turn("existing", TurnStatus::Completed, Vec::new()));
        }
        if scenario == "parent_owned" {
            thread.blocks_direct_input = true;
        }
        let initial_message = (scenario == "initial_prompt")
            .then(|| create_initial_user_message(Some("hello".into()), Vec::new(), Vec::new()))
            .flatten();
        app.replace_chat_widget_with_app_server_thread(
            &mut tui,
            thread,
            presentation,
            initial_message,
        )
        .await?;

        assert!(visible(&app.chat_widget), "{scenario}");
        app.chat_widget.set_model("gpt-5.5");
        assert!(!visible(&app.chat_widget), "{scenario}");
        app.chat_widget.set_model("gpt-6-astra");
        assert!(visible(&app.chat_widget), "{scenario}");
    }
    Ok(())
}

#[tokio::test]
async fn drafts_do_not_permanently_end_the_astra_sparkle() -> Result<()> {
    let (mut app, _events, _ops) = make_test_app_with_channels().await;
    app.local_settings.tui.animations = true;
    app.local_settings.tui.effects.starfield = true;
    app.local_settings.tui.disable_paste_burst = Some(true);
    let mut tui = crate::tui::test_support::make_test_tui()?;
    app.replace_chat_widget_with_app_server_thread(
        &mut tui,
        started("gpt-6-astra"),
        ThreadAttachPresentation::Fresh,
        /*initial_user_message*/ None,
    )
    .await?;

    assert!(visible(&app.chat_widget));
    type_into(&mut app.chat_widget, "draft");
    assert!(visible(&app.chat_widget));
    for _ in "draft".chars() {
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    }
    assert!(visible(&app.chat_widget));
    Ok(())
}

#[test]
fn astra_picker_confirms_and_restores_the_sparkle() -> Result<()> {
    // The 0.157 event dispatcher has a large debug future; production launchers
    // provide a larger stack. Match that budget instead of reporting a false
    // picker regression on the default test-thread stack.
    std::thread::Builder::new()
        .name("tui-astra-picker-stack".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(async {
                    let (mut app, mut events, _ops) = make_test_app_with_channels().await;
                    app.local_settings.tui.animations = true;
                    app.local_settings.tui.effects.starfield = true;
                    app.local_settings.tui.disable_paste_burst = Some(true);
                    let mut server = start_config_write_test_app_server(&app).await?;
                    let mut tui = crate::tui::test_support::make_test_tui()?;
                    app.replace_chat_widget_with_app_server_thread(
                        &mut tui,
                        started("gpt-5.5"),
                        ThreadAttachPresentation::Fresh,
                        /*initial_user_message*/ None,
                    )
                    .await?;
                    assert!(!visible(&app.chat_widget));

                    type_into(&mut app.chat_widget, "/model");
                    app.chat_widget
                        .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    assert!(app.chat_widget.has_active_view());
                    choose_astra(&mut app, &mut tui, &mut server, &mut events).await?;
                    assert_eq!(app.chat_widget.current_model(), "gpt-6-astra");
                    assert!(visible(&app.chat_widget));

                    server.shutdown().await?;
                    Ok(())
                })
        })?
        .join()
        .expect("astra picker test thread")
}

#[test]
fn active_work_does_not_end_the_astra_sparkle() -> Result<()> {
    std::thread::Builder::new()
        .name("tui-active-work-sparkle-stack".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(async {
                    let (mut app, _events, _ops) = make_test_app_with_channels().await;
                    app.chat_widget.local_settings.tui.animations = true;
                    app.chat_widget.local_settings.tui.effects.starfield = true;
                    app.pending_startup_thread_start = true;
                    let mut server =
                        crate::start_embedded_app_server_for_picker(&app.config).await?;
                    let thread = started("gpt-6-astra");
                    let thread_id = thread.session.thread_id;
                    app.handle_startup_thread_started(&mut server, Ok(thread))
                        .await?;
                    assert!(visible(&app.chat_widget));

                    app.handle_app_server_event(
                        &server,
                        codex_app_server_client::AppServerEvent::ServerNotification(Box::new(
                            turn_started_notification(thread_id, "work"),
                        )),
                    )
                    .await;
                    let mut tui = crate::tui::test_support::make_test_tui()?;
                    app.drain_active_thread_events(&mut tui).await?;
                    assert!(visible(&app.chat_widget));
                    server.shutdown().await?;
                    Ok(())
                })
        })?
        .join()
        .expect("active work sparkle test thread")
}
