//! Embedded audio host behavior without starting an installed STAR/AMP.

use super::*;
use crate::config::Config;
use crate::fold::sort::SortOrder;
use crate::ui::fake;
use starkit::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::time::{Duration, Instant};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

fn build() -> (App, fake::Fake) {
    let cfg = Config::default();
    let (core, fake) = fake::handle(cfg.core());
    let app = App::new(
        core,
        cfg,
        fake.home().join("config.toml"),
        None,
        Graphics::disabled(),
    );
    (app, fake)
}

fn settle(app: &mut App, fake: &fake::Fake) {
    fake.pump();
    app.tick();
    fake.pump();
    app.tick();
}

fn draw(app: &mut App, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    app.draw(area, &mut buffer);
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn fake_frame(presentation: &Presentation, symbol: &str) -> audio_embed::Frame {
    audio_embed::Frame {
        generation: presentation.generation,
        width: presentation.width,
        height: presentation.height,
        cells: vec![
            audio_embed::Cell {
                symbol: symbol.into(),
                fg: [255, 100, 30],
                bg: presentation.theme.bg,
                modifiers: 0,
            };
            usize::from(presentation.width) * usize::from(presentation.height)
        ],
        images: Vec::new(),
    }
}

fn pin_audio(app: &mut App, fake: &fake::Fake) {
    app.audio_path = Some(fake.fixture.path("blob.bin"));
    app.layout.audio_active = true;
    app.layout.preview_open = true;
}

#[cfg(unix)]
fn executable(path: &std::path::Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
fn fake_player(
    fake: &fake::Fake,
    name: &str,
    delay_hello: bool,
    response: &str,
) -> (PathBuf, PathBuf) {
    fake_player_with_hello(
        fake,
        name,
        delay_hello,
        r#"{"type":"hello","protocol":1,"extensions":["mp3"]}"#,
        response,
    )
}

#[cfg(unix)]
fn fake_player_with_hello(
    fake: &fake::Fake,
    name: &str,
    delay_hello: bool,
    hello: &str,
    response: &str,
) -> (PathBuf, PathBuf) {
    let program = fake.fixture.path(name);
    let log = fake.fixture.path(&format!("{name}.jsonl"));
    let delay = if delay_hello { "sleep 0.2\n" } else { "" };
    let script = format!(
        "#!/bin/sh\n{delay}printf '%s\\n' {}\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> {}\n  case \"$line\" in\n    *'\"type\":\"play\"'*) sleep 0.05; printf '%s\\n' {} ;;\n    *'\"type\":\"shutdown\"'*) exit 0 ;;\n  esac\ndone\n",
        shell_quote(hello),
        shell_quote(&log.display().to_string()),
        shell_quote(response),
    );
    executable(&program, &script);
    (program, log)
}

#[cfg(unix)]
fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for fake player"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn messages(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[cfg(unix)]
fn left_click(x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

#[cfg(unix)]
fn right_click(x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

#[cfg(unix)]
fn selected_row_position(app: &App, commander: bool) -> (u16, u16) {
    let regions = app.layout.last.as_ref().unwrap();
    let area = regions.rect_of(ModuleId::Stack);
    let rect = if commander {
        pane_rect(area, app.active_pane)
    } else {
        area
    };
    let view = if commander {
        app.pane_view(app.active_pane)
    } else {
        app.stack_view()
    };
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if panels::stack::hit(rect, &view, x, y) == Some(panels::stack::Hit::Row(view.cursor)) {
                return (x, y);
            }
        }
    }
    panic!("selected row has no hit target");
}

#[cfg(unix)]
fn media_app() -> (App, fake::Fake, PathBuf, PathBuf) {
    media_app_with_config(Config::default())
}

#[cfg(unix)]
fn media_app_with_config(cfg: Config) -> (App, fake::Fake, PathBuf, PathBuf) {
    let (core, fake) = fake::handle(cfg.core());
    let mut app = App::new(
        core,
        cfg,
        fake.home().join("config.toml"),
        None,
        Graphics::disabled(),
    );
    let first = fake.fixture.path("a.mp3");
    let second = fake.fixture.path("b.mp3");
    std::fs::write(&first, b"a").unwrap();
    std::fs::write(&second, b"b").unwrap();
    app.core.send(Command::Reload);
    app.core.send(Command::SetSort(SortOrder {
        reverse: true,
        ..SortOrder::default()
    }));
    app.core.send(Command::SetFilter("mp3".into()));
    settle(&mut app, &fake);
    app.core.send(Command::CursorTo(0));
    app.tick();
    (app, fake, second, first)
}

#[cfg(unix)]
fn switch_to_commander_media(app: &mut App, fake: &fake::Fake) {
    app.key(key(KeyCode::Char('v')));
    settle(app, fake);
    app.core.send(Command::SetSort(SortOrder {
        reverse: true,
        ..SortOrder::default()
    }));
    app.core.send(Command::SetFilter("mp3".into()));
    settle(app, fake);
    app.core.send(Command::CursorTo(0));
    settle(app, fake);
}

#[test]
fn activation_captures_only_the_active_filtered_sorted_listing_without_marks() {
    let (mut app, fake) = build();
    let first = fake.fixture.path("a.mp3");
    let second = fake.fixture.path("b.mp3");
    std::fs::write(&first, b"a").unwrap();
    std::fs::write(&second, b"b").unwrap();
    app.core.send(Command::Reload);
    let sort = SortOrder {
        reverse: true,
        ..SortOrder::default()
    };
    app.core.send(Command::SetSort(sort));
    app.core.send(Command::SetFilter("mp3".into()));
    settle(&mut app, &fake);

    let candidates = audio_candidates(&app.core.state());
    assert_eq!(candidates, vec![second.clone(), first]);
    assert!(fake.state().selection.is_empty());

    // The nonexistent fixture executable makes this an activation test with
    // no installed player or audio device. The capture happens synchronously.
    app.audio = AudioClient::with_executable(fake.fixture.path("missing-staramp"));
    app.core.send(Command::CursorTo(0));
    app.tick();
    app.key(key(KeyCode::Enter));
    assert_eq!(app.audio_path, Some(second));
    assert!(fake.state().selection.is_empty());
    assert!(fake.state().queue.is_empty());
}

#[test]
fn player_keys_are_scoped_to_preview_and_browser_navigation_remains_available() {
    let (mut app, fake) = build();
    pin_audio(&mut app, &fake);
    app.core
        .send(Command::Push(fake.fixture.path("projects/starwire")));
    settle(&mut app, &fake);

    app.key(alt('2'));
    assert_eq!(app.layout.focus(), ModuleId::Preview);
    app.key(key(KeyCode::Char(' ')));
    app.key(key(KeyCode::Left));
    assert!(fake.state().selection.is_empty());
    assert_eq!(
        fake.state().active_frame().dir,
        fake.fixture.path("projects/starwire")
    );

    app.key(alt('1'));
    assert_eq!(app.layout.focus(), ModuleId::Stack);
    app.key(key(KeyCode::Char('h')));
    settle(&mut app, &fake);
    assert_eq!(
        fake.state().active_frame().dir,
        fake.fixture.path("projects")
    );
    app.key(key(KeyCode::Char('h')));
    settle(&mut app, &fake);
    assert_eq!(fake.state().active_frame().dir, fake.home());
    assert!(app.audio_path.is_some());
    app.key(key(KeyCode::Char('q')));
    assert!(app.quit);
}

#[test]
fn button_toggle_is_preview_scoped_persisted_and_safe_without_graphics() {
    let (mut app, fake) = build();
    pin_audio(&mut app, &fake);
    let pinned = app.audio_path.clone();
    assert_eq!(app.cfg.preview.audio_buttons, AudioButtons::Auto);
    assert!(app.audio_graphics_config().is_none());
    app.key(key(KeyCode::Char('o')));
    assert_eq!(app.cfg.preview.audio_buttons, AudioButtons::Auto);
    app.key(alt('2'));
    app.key(key(KeyCode::Char('o')));
    assert_eq!(app.cfg.preview.audio_buttons, AudioButtons::Text);
    let saved: Config = toml::from_str(&std::fs::read_to_string(&app.cfg_path).unwrap()).unwrap();
    assert_eq!(saved.preview.audio_buttons, AudioButtons::Text);
    assert_eq!(app.audio_path, pinned);
    app.key(key(KeyCode::Char('o')));
    assert_eq!(app.cfg.preview.audio_buttons, AudioButtons::Auto);
    assert!(app.audio_graphics_config().is_none());
    assert_eq!(app.audio_path, pinned);
    app.key(key(KeyCode::Char('x')));
    assert!(app.audio_path.is_none());
}

#[test]
fn transport_measurement_preserves_cover_mode() {
    for mode in [Mode::Off, Mode::Blocks, Mode::Auto, Mode::Kitty] {
        let mut graphics = Graphics::disabled();
        graphics.set_mode(mode);
        let size = transport_cell_size(&mut graphics);
        assert_eq!(graphics.mode(), mode);
        if mode == Mode::Kitty {
            assert!(size.is_some());
        }
    }
}

#[cfg(unix)]
#[test]
fn helper_graphics_capability_and_focused_button_toggle_reconfigure_player() {
    let (mut app, fake, _selected, _other) = media_app();
    app.graphics.set_mode(Mode::Kitty);
    // Model the terminal's startup measurement without querying the test PTY.
    app.audio_cell_size = Some((8, 16));
    assert!(app.graphics.pictures_available());
    let hello =
        r#"{"type":"hello","protocol":1,"extensions":["mp3"],"capabilities":["transport_images"]}"#;
    let status = r#"{"type":"status","playing":true,"paused":false,"title":"track"}"#;
    let (program, log) = fake_player_with_hello(&fake, "pictures-staramp", false, hello, status);
    app.audio = AudioClient::with_executable(program);
    app.key(key(KeyCode::Enter));
    wait_for(|| {
        app.tick();
        app.layout.focus() == ModuleId::Preview && app.audio.transport_images_available()
    });

    draw(&mut app, 100, 30);
    wait_for(|| {
        messages(&log)
            .iter()
            .filter(|m| m["type"] == "configure")
            .any(|m| m["graphics"]["cell_width"] == 8 && m["graphics"]["cell_height"] == 16)
    });
    let pictures = app.audio_generation;
    app.key(key(KeyCode::Char('o')));
    draw(&mut app, 100, 30);
    let text = app.audio_generation;
    assert!(text > pictures);
    wait_for(|| {
        messages(&log)
            .iter()
            .any(|m| m["type"] == "configure" && m["generation"] == text)
    });
    app.key(key(KeyCode::Char('o')));
    draw(&mut app, 100, 30);
    let restored = app.audio_generation;
    assert!(restored > text);
    wait_for(|| {
        messages(&log)
            .iter()
            .any(|m| m["type"] == "configure" && m["generation"] == restored)
    });
    let sent = messages(&log);
    let config = |generation: u64| {
        sent.iter()
            .rev()
            .find(|m| m["type"] == "configure" && m["generation"] == generation)
            .unwrap()
    };
    assert_eq!(config(pictures)["graphics"]["cell_width"], 8);
    assert!(config(text).get("graphics").is_none());
    assert_eq!(config(restored)["graphics"]["cell_height"], 16);

    let (mut text_app, text_fake, _selected, _other) = media_app();
    text_app.graphics.set_mode(Mode::Kitty);
    text_app.audio_cell_size = Some((8, 16));
    let (program, text_log) = fake_player(&text_fake, "text-staramp", false, status);
    text_app.audio = AudioClient::with_executable(program);
    text_app.key(key(KeyCode::Enter));
    wait_for(|| {
        text_app.tick();
        text_app.layout.focus() == ModuleId::Preview
    });
    assert!(!text_app.audio.transport_images_available());
    draw(&mut text_app, 100, 30);
    wait_for(|| messages(&text_log).iter().any(|m| m["type"] == "configure"));
    assert!(messages(&text_log)
        .iter()
        .filter(|m| m["type"] == "configure")
        .all(|m| m.get("graphics").is_none()));
}

#[cfg(unix)]
#[test]
fn player_styles_are_scoped_to_preview_and_profile_is_capability_gated() {
    let (mut app, fake, _selected, _other) = media_app();
    let hello =
        r#"{"type":"hello","protocol":1,"extensions":["mp3"],"capabilities":["player_styles"]}"#;
    let status = r#"{"type":"status","playing":true,"paused":false,"title":"track"}"#;
    let notice = r#"{"type":"notice","message":"Could not save style profile"}"#;
    let (program, log) = fake_player_with_hello(
        &fake,
        "styles-staramp",
        false,
        hello,
        &format!("{status}\n{notice}"),
    );
    app.audio = AudioClient::with_executable(program);
    app.key(key(KeyCode::Enter));
    wait_for(|| {
        app.tick();
        app.layout.focus() == ModuleId::Preview
            && app.audio.player_styles_available()
            && app.note.as_ref().is_some_and(|(message, level, _)| {
                message == "Could not save style profile" && *level == NoteLevel::Warning
            })
    });
    assert!(app.audio_error.is_none());
    draw(&mut app, 100, 30);
    wait_for(|| {
        messages(&log)
            .iter()
            .any(|m| m["type"] == "configure" && m["profile"] == "starfold")
    });

    for c in ['w', 'W', 'd'] {
        app.key(key(KeyCode::Char(c)));
    }
    wait_for(|| {
        messages(&log)
            .iter()
            .filter(|m| m["type"] == "control")
            .count()
            >= 3
    });
    let sent = messages(&log);
    let actions: Vec<_> = sent
        .iter()
        .filter(|m| m["type"] == "control")
        .map(|m| m["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        ["next_visualizer", "prev_visualizer", "next_seek_style"]
    );
    assert!(
        fake.state().queue.is_empty(),
        "focused d must not queue delete"
    );

    let body = audio_body(app.layout.last.as_ref().unwrap().rect_of(ModuleId::Preview));
    app.mouse(right_click(body.x + 1, body.y + 1));
    wait_for(|| {
        messages(&log)
            .iter()
            .any(|m| m["type"] == "pointer" && m["button"] == "right")
    });
    app.key(alt('1'));
    app.key(key(KeyCode::Char('d')));
    assert!(
        !fake.state().queue.is_empty(),
        "browser d still queues delete when Stack has focus"
    );

    let (mut old_app, old_fake, _selected, _other) = media_app();
    let (program, old_log) = fake_player(&old_fake, "old-styles-staramp", false, status);
    old_app.audio = AudioClient::with_executable(program);
    old_app.key(key(KeyCode::Enter));
    wait_for(|| {
        old_app.tick();
        old_app.layout.focus() == ModuleId::Preview
    });
    assert!(!old_app.audio.player_styles_available());
    draw(&mut old_app, 100, 30);
    old_app.key(key(KeyCode::Char('d')));
    old_app.key(key(KeyCode::Char('w')));
    old_app.key(key(KeyCode::Char('W')));
    let body = audio_body(
        old_app
            .layout
            .last
            .as_ref()
            .unwrap()
            .rect_of(ModuleId::Preview),
    );
    old_app.mouse(right_click(body.x + 1, body.y + 1));
    assert!(old_fake.state().queue.is_empty());
    assert!(old_app.audio_error.is_none());
    assert!(old_app
        .note
        .as_ref()
        .is_some_and(|(message, _, _)| message.contains("Update STAR/AMP")));
    assert!(messages(&old_log)
        .iter()
        .filter(|m| m["type"] == "configure")
        .all(|m| m.get("profile").is_none()));
    assert!(!messages(&old_log)
        .iter()
        .any(|m| m["type"] == "control" || m["type"] == "pointer"));
}

#[test]
fn player_stays_pinned_while_cursor_and_view_change_then_close_restores_preview() {
    let (mut app, fake) = build();
    pin_audio(&mut app, &fake);
    let pinned = app.audio_path.clone();
    app.key(key(KeyCode::Char('j')));
    settle(&mut app, &fake);
    app.key(key(KeyCode::Char('v')));
    settle(&mut app, &fake);
    assert_eq!(app.audio_path, pinned);
    assert!(app.commander);

    app.key(key(KeyCode::Char('i')));
    assert!(app.audio_path.is_none());
    assert!(!app.layout.audio_active);
    assert!(!app.layout.preview_open);
    app.key(key(KeyCode::Char('i')));
    settle(&mut app, &fake);
    assert!(app.layout.preview_open);
    assert!(app.audio_frame.is_none());
}

#[test]
fn embedded_player_uses_canonical_heading_in_both_views() {
    for commander in [false, true] {
        for (width, height) in [(60, 21), (100, 30)] {
            let (mut app, fake) = build();
            if commander {
                app.key(key(KeyCode::Char('v')));
                settle(&mut app, &fake);
            }
            pin_audio(&mut app, &fake);
            for focus in [ModuleId::Stack, ModuleId::Preview] {
                app.layout.focus_set(focus);
                let area = Rect::new(0, 0, width, height);
                let mut buffer = Buffer::empty(area);
                app.draw(area, &mut buffer);
                let panel = app.layout.last.as_ref().unwrap().rect_of(ModuleId::Preview);
                let heading: String = (panel.x..panel.right())
                    .map(|x| buffer[(x, panel.y)].symbol())
                    .collect();
                assert!(
                    heading.contains("Preview — S T A R / A M P · embed"),
                    "{heading}"
                );
                let first_letter = (panel.x..panel.right())
                    .find(|&x| buffer[(x, panel.y)].symbol() == "S")
                    .unwrap();
                let cell = &buffer[(first_letter, panel.y)];
                assert!(cell.modifier.contains(Modifier::BOLD));
                assert_eq!(cell.fg, panels::rgb(app.theme.titlebar_active_fg));
            }
        }
    }
}

#[test]
fn synthetic_frame_draws_at_floor_and_roomy_sizes_and_resizes_discard_old_cells() {
    let (mut app, fake) = build();
    pin_audio(&mut app, &fake);
    draw(&mut app, 60, 21);
    let floor = app.audio_presentation.clone().unwrap();
    assert_eq!(floor.height, 5);
    assert_eq!(
        floor.theme.bg,
        [
            app.theme.panel_bg.r,
            app.theme.panel_bg.g,
            app.theme.panel_bg.b
        ]
    );
    app.audio_frame = Some(fake_frame(&floor, "@"));
    assert!(draw(&mut app, 60, 21).contains("@@@@"));

    let larger = draw(&mut app, 100, 30);
    let roomy = app.audio_presentation.clone().unwrap();
    assert_eq!(roomy.height, 10);
    assert_ne!(roomy.generation, floor.generation);
    assert!(app.audio_frame.is_none());
    assert!(!larger.contains("@@@@"));
    app.audio_frame = Some(fake_frame(&roomy, "#"));
    assert!(draw(&mut app, 100, 30).contains("####"));

    app.key(key(KeyCode::Char('t')));
    draw(&mut app, 100, 30);
    let recolored = app.audio_presentation.clone().unwrap();
    assert_ne!(recolored.theme, roomy.theme);
    assert!(app.audio_frame.is_none());
}

#[test]
fn stop_key_clears_audio_and_leaves_preview_available() {
    let (mut app, fake) = build();
    pin_audio(&mut app, &fake);
    app.key(alt('2'));
    app.key(key(KeyCode::Char('x')));
    assert!(app.audio_path.is_none());
    assert!(!app.layout.audio_active);
    assert!(app.layout.preview_open);
    settle(&mut app, &fake);
    assert!(app.view.preview.is_some());
}

#[cfg(unix)]
#[test]
fn fake_player_receives_sorted_filtered_queue_and_returns_a_real_protocol_frame() {
    let (mut app, fake, selected, other) = media_app();
    let cells = vec![
        serde_json::json!({
            "symbol":"@", "fg":[255,100,30], "bg":[10,20,30], "modifiers":0
        });
        58 * 5
    ];
    let status = serde_json::json!({
        "type":"status", "playing":true, "paused":false,
        "title":"fake track", "path":selected.display().to_string()
    });
    let frame = serde_json::json!({
        "type":"frame", "generation":1, "width":58, "height":5, "cells":cells
    });
    let (program, log) = fake_player(&fake, "fake-staramp", false, &format!("{status}\n{frame}"));
    app.audio = AudioClient::with_executable(program);

    app.key(key(KeyCode::Enter));
    wait_for(|| {
        app.tick();
        app.audio_frame.is_some()
    });
    let sent = messages(&log);
    let configure = sent.iter().find(|m| m["type"] == "configure").unwrap();
    let play = sent.iter().find(|m| m["type"] == "play").unwrap();
    assert_eq!(configure["generation"], 1);
    assert_eq!(play["generation"], 1);
    assert_eq!(play["index"], 0);
    assert_eq!(
        play["paths"],
        serde_json::json!([selected.display().to_string(), other.display().to_string()])
    );
    assert_eq!(app.audio_frame.as_ref().unwrap().cells[0].symbol, "@");
    assert_eq!(app.audio_path, Some(selected));
    assert_eq!(app.layout.focus(), ModuleId::Preview);
    assert!(fake.state().selection.is_empty());
}

#[cfg(unix)]
#[test]
fn accepted_song_focuses_player_and_space_then_stop_work_without_alt_two() {
    for commander in [false, true] {
        let (mut app, fake, selected, _other) = media_app();
        if commander {
            switch_to_commander_media(&mut app, &fake);
        }
        let status = r#"{"type":"status","playing":true,"paused":false,"title":"track"}"#;
        let (program, log) = fake_player(&fake, "focus-staramp", false, status);
        app.audio = AudioClient::with_executable(program);
        app.key(key(KeyCode::Enter));
        wait_for(|| {
            app.tick();
            app.layout.focus() == ModuleId::Preview
        });
        assert_eq!(app.audio_path, Some(selected));
        draw(&mut app, 100, 30);
        assert!(app.audio_presentation.as_ref().unwrap().focused);
        app.key(key(KeyCode::Char(' ')));
        wait_for(|| messages(&log).iter().any(|m| m["action"] == "toggle_pause"));
        app.key(key(KeyCode::Char('x')));
        assert!(app.audio_path.is_none());
        assert!(!app.layout.audio_active);
    }
}

#[cfg(unix)]
#[test]
fn double_click_song_focuses_player_in_fold_and_commander() {
    for commander in [false, true] {
        let (mut app, fake, selected, _other) = media_app();
        if commander {
            switch_to_commander_media(&mut app, &fake);
        }
        let status = r#"{"type":"status","playing":true,"paused":false,"title":"track"}"#;
        let (program, _log) = fake_player(&fake, "click-staramp", false, status);
        app.audio = AudioClient::with_executable(program);
        draw(&mut app, 100, 30);
        let (x, y) = selected_row_position(&app, commander);
        app.mouse(left_click(x, y));
        settle(&mut app, &fake);
        app.mouse(left_click(x, y));
        wait_for(|| {
            app.tick();
            app.layout.focus() == ModuleId::Preview
        });
        assert_eq!(app.audio_path, Some(selected));
    }
}

#[cfg(unix)]
#[test]
fn delayed_acceptance_does_not_steal_focus_after_navigation() {
    let (mut app, fake, _selected, _other) = media_app();
    let status = r#"{"type":"status","playing":true,"paused":false,"title":"track"}"#;
    let (program, log) = fake_player(&fake, "delayed-focus-staramp", true, status);
    app.audio = AudioClient::with_executable(program);
    app.key(key(KeyCode::Enter));
    app.key(key(KeyCode::Char('j')));
    settle(&mut app, &fake);
    wait_for(|| {
        app.tick();
        app.audio_focus_origin.is_none() && messages(&log).iter().any(|m| m["type"] == "play")
    });
    assert_eq!(app.layout.focus(), ModuleId::Stack);
    assert!(app.audio_path.is_some());
}

#[cfg(unix)]
#[test]
fn later_track_status_does_not_refocus_player_while_browsing() {
    let (mut app, fake, selected, other) = media_app();
    let program = fake.fixture.path("next-staramp");
    let first = serde_json::json!({
        "type":"status", "playing":true, "paused":false,
        "title":"first", "path":selected.display().to_string()
    });
    let next = serde_json::json!({
        "type":"status", "playing":true, "paused":false,
        "title":"next", "path":other.display().to_string()
    });
    executable(
        &program,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' {}\nwhile IFS= read -r line; do\n  case \"$line\" in\n    *'\"type\":\"play\"'*) printf '%s\\n' {} ;;\n    *'\"action\":\"next\"'*) printf '%s\\n' {} ;;\n    *'\"type\":\"shutdown\"'*) exit 0 ;;\n  esac\ndone\n",
            shell_quote(r#"{"type":"hello","protocol":1,"extensions":["mp3"]}"#),
            shell_quote(&first.to_string()),
            shell_quote(&next.to_string())
        ),
    );
    app.audio = AudioClient::with_executable(program);
    app.key(key(KeyCode::Enter));
    wait_for(|| {
        app.tick();
        app.layout.focus() == ModuleId::Preview && app.audio_path.as_ref() == Some(&selected)
    });
    app.key(alt('1'));
    assert_eq!(app.layout.focus(), ModuleId::Stack);
    app.audio.control("next", None).unwrap();
    wait_for(|| {
        app.tick();
        app.audio_path.as_ref() == Some(&other)
    });
    assert_eq!(app.layout.focus(), ModuleId::Stack);
}

#[cfg(unix)]
#[test]
fn resize_during_handshake_sends_latest_generation_with_play() {
    let (mut app, fake, _selected, _other) = media_app();
    let idle = r#"{"type":"status","playing":false,"paused":false,"title":"idle"}"#;
    let (program, log) = fake_player(&fake, "late-staramp", true, idle);
    app.audio = AudioClient::with_executable(program);
    app.key(key(KeyCode::Enter));
    draw(&mut app, 60, 21);
    draw(&mut app, 100, 30);
    let expected = app.audio_generation;
    wait_for(|| messages(&log).iter().any(|m| m["type"] == "play"));
    let sent = messages(&log);
    let play = sent.iter().find(|m| m["type"] == "play").unwrap();
    let configure = sent
        .iter()
        .rev()
        .find(|m| m["type"] == "configure")
        .unwrap();
    assert_eq!(play["generation"], expected);
    assert_eq!(configure["generation"], expected);
    assert_eq!(configure["height"], 10);
}

#[cfg(unix)]
#[test]
fn missing_player_falls_back_to_configured_external_opener() {
    let temp = tempfile::tempdir().unwrap();
    let opener = temp.path().join("opener");
    let log = temp.path().join("opened.txt");
    executable(
        &opener,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" > {}\n",
            shell_quote(&log.display().to_string())
        ),
    );
    let mut cfg = Config::default();
    cfg.open.command = opener.display().to_string();
    cfg.preview.audio_player = crate::config::AudioPlayer::Staramp;
    let (mut app, fake, selected, _other) = media_app_with_config(cfg);
    app.audio = AudioClient::with_executable(fake.fixture.path("missing-staramp"));
    app.key(key(KeyCode::Enter));
    wait_for(|| {
        app.tick();
        fake.pump();
        log.exists()
    });
    assert_eq!(
        std::fs::read_to_string(log).unwrap().trim(),
        selected.display().to_string()
    );
    assert!(app.audio_path.is_none());
    assert!(!app.layout.audio_active);
}

#[cfg(unix)]
#[test]
fn decoder_error_after_handshake_stays_in_embedded_preview() {
    let mut cfg = Config::default();
    cfg.preview.audio_player = crate::config::AudioPlayer::Staramp;
    cfg.open.command = "/bin/true".into();
    let (mut app, fake, selected, _other) = media_app_with_config(cfg);
    let error = r#"{"type":"error","message":"decoder failed"}"#;
    let (program, _log) = fake_player(&fake, "error-staramp", false, error);
    app.audio = AudioClient::with_executable(program);
    app.key(key(KeyCode::Enter));
    wait_for(|| {
        app.tick();
        app.audio_error.as_deref() == Some("decoder failed")
    });
    assert_eq!(app.audio_path, Some(selected));
    assert!(app.layout.audio_active);
    assert_eq!(app.audio_error.as_deref(), Some("decoder failed"));
}
