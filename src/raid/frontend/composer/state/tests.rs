use super::ComposerState;
use crate::frontend::composer::action::ComposerAction;
use crate::frontend::composer::slash_commands::COMMANDS;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn edits_text_and_tracks_utf8_cursor() {
    let mut composer = ComposerState::default();

    composer.handle_key(key(KeyCode::Char('h')));
    composer.handle_key(key(KeyCode::Char('i')));
    composer.handle_key(key(KeyCode::Left));
    composer.handle_key(key(KeyCode::Char('é')));

    assert_eq!(composer.text, "héi");
    assert_eq!(composer.cursor, "hé".len());
}

#[test]
fn backspace_removes_the_previous_character() {
    let mut composer = ComposerState::default();
    composer.handle_key(key(KeyCode::Char('a')));
    composer.handle_key(key(KeyCode::Char('界')));
    composer.handle_key(key(KeyCode::Backspace));

    assert_eq!(composer.text, "a");
    assert_eq!(composer.cursor, 1);
}

#[test]
fn editing_stays_on_grapheme_boundaries() {
    let mut composer = ComposerState::default();
    composer.insert_paste("e\u{301}");
    composer.handle_key(key(KeyCode::Backspace));

    assert_eq!(composer.text, "");
    assert_eq!(composer.cursor, 0);
}

#[test]
fn paste_normalizes_lines_and_drops_control_characters() {
    let mut composer = ComposerState::default();
    composer.insert_paste("one\r\n\t two\u{2028}three\u{2029}four\u{0007}");

    assert_eq!(composer.text, "one\n     two\nthree\nfour");
}

#[test]
fn shift_enter_inserts_a_newline_without_submitting() {
    let mut composer = ComposerState::default();
    composer.handle_key(key(KeyCode::Char('a')));

    assert_eq!(
        composer.handle_key(modified_key(KeyCode::Enter, KeyModifiers::SHIFT)),
        ComposerAction::None
    );
    composer.handle_key(key(KeyCode::Char('b')));

    assert_eq!(composer.text, "a\nb");
    assert_eq!(composer.cursor, "a\nb".len());
}

#[test]
fn vertical_navigation_preserves_the_text_column() {
    let mut composer = ComposerState::default();
    composer.insert_paste("first\nsecond");
    composer.handle_key(key(KeyCode::Home));
    composer.handle_key(key(KeyCode::Up));

    assert_eq!(composer.cursor, 0);

    composer.handle_key(key(KeyCode::Down));
    composer.handle_key(key(KeyCode::End));
    assert_eq!(composer.cursor, composer.text.len());
}

#[test]
fn full_width_text_before_a_newline_does_not_add_a_caret_row() {
    let mut composer = ComposerState::default();
    composer.insert_paste("abc\ndef");
    composer.cursor = "abc".len();

    assert_eq!(composer.desired_height(3, 20), 4);
}

#[test]
fn vertical_navigation_follows_soft_wrapped_lines() {
    let mut composer = ComposerState::default();
    composer.insert_paste("abcdef");

    composer.handle_key_with_width(modified_key(KeyCode::Home, KeyModifiers::CONTROL), 3);
    composer.handle_key_with_width(key(KeyCode::Down), 3);
    assert_eq!(composer.cursor, 3);

    composer.handle_key_with_width(key(KeyCode::Up), 3);
    assert_eq!(composer.cursor, 0);

    composer.handle_key_with_width(key(KeyCode::End), 3);
    assert_eq!(composer.cursor, 3);
}

#[test]
fn desired_height_respects_tiny_maximums() {
    let composer = ComposerState::default();

    assert_eq!(composer.desired_height(8, 0), 0);
    assert_eq!(composer.desired_height(8, 1), 1);
    assert_eq!(composer.desired_height(8, 2), 2);
}

#[test]
fn alt_gr_character_is_inserted() {
    let mut composer = ComposerState::default();
    let alt_gr = KeyEvent::new(
        KeyCode::Char('@'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    );

    composer.handle_key(alt_gr);

    assert_eq!(composer.text, "@");
}

#[test]
fn control_key_characters_are_not_inserted() {
    let mut composer = ComposerState::default();

    composer.handle_key(KeyEvent::new(KeyCode::Char('\u{0007}'), KeyModifiers::NONE));

    assert_eq!(composer.text, "");
}

#[test]
fn enter_submits_and_resets_the_composer() {
    let mut composer = ComposerState::default();
    composer.handle_key(key(KeyCode::Char('r')));
    composer.handle_key(key(KeyCode::Char('u')));
    composer.handle_key(key(KeyCode::Char('n')));

    assert_eq!(
        composer.handle_key(key(KeyCode::Enter)),
        ComposerAction::Submit("run".to_owned())
    );
    assert_eq!(composer, ComposerState::default());
}

#[test]
fn escape_does_not_quit_or_change_text() {
    let mut composer = ComposerState::default();

    assert_eq!(composer.handle_key(key(KeyCode::Esc)), ComposerAction::None);

    composer.handle_key(key(KeyCode::Char('x')));
    assert_eq!(composer.handle_key(key(KeyCode::Esc)), ComposerAction::None);
    assert_eq!(composer.text, "x");
}

#[test]
fn ctrl_c_clears_text_without_quitting() {
    let mut composer = ComposerState::default();
    composer.insert_paste("draft\nline");
    composer.handle_key(key(KeyCode::Up));

    assert_eq!(
        composer.handle_key(modified_key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        ComposerAction::None
    );
    assert_eq!(composer, ComposerState::default());
    assert_eq!(
        composer.handle_key(modified_key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        ComposerAction::Quit
    );
}

#[test]
fn ctrl_c_quits_when_the_composer_is_empty() {
    let mut composer = ComposerState::default();

    assert_eq!(
        composer.handle_key(modified_key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        ComposerAction::Quit
    );
    assert_eq!(composer, ComposerState::default());
}

#[test]
fn ctrl_c_clears_whitespace_without_quitting() {
    let mut composer = ComposerState::default();
    composer.insert_paste("  \n");

    assert_eq!(
        composer.handle_key(modified_key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        ComposerAction::None
    );
    assert_eq!(composer, ComposerState::default());
}

#[test]
fn ctrl_alt_c_inserts_instead_of_clearing() {
    let mut composer = ComposerState::default();
    let alt_gr_c = KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    );

    assert_eq!(composer.handle_key(alt_gr_c), ComposerAction::None);
    assert_eq!(composer.text, "c");
}

#[test]
fn slash_opens_the_palette_and_enter_runs_the_selected_command() {
    let mut composer = ComposerState::default();
    composer.handle_key(key(KeyCode::Char('/')));

    assert!(composer.palette_visible());
    assert_eq!(
        composer.handle_key(key(KeyCode::Enter)),
        ComposerAction::Command {
            name: "settings".to_owned(),
            args: String::new(),
        }
    );
    assert_eq!(composer, ComposerState::default());
}

#[test]
fn slash_query_filters_and_down_selects_the_next_command() {
    let mut composer = ComposerState::default();
    composer.insert_paste("/m");
    assert_eq!(
        composer
            .palette_matches()
            .iter()
            .map(|command| command.name)
            .collect::<Vec<_>>(),
        ["model"]
    );

    let mut composer = ComposerState::default();
    composer.insert_paste("/s");
    composer.handle_key(key(KeyCode::Down));
    assert_eq!(
        composer.handle_key(key(KeyCode::Enter)),
        ComposerAction::Command {
            name: "scoped-models".to_owned(),
            args: String::new(),
        }
    );
}

#[test]
fn palette_wraps_from_the_last_command_to_the_first() {
    let mut composer = ComposerState::default();
    composer.handle_key(key(KeyCode::Char('/')));
    composer.handle_key(key(KeyCode::Up));

    assert_eq!(
        composer.handle_key(key(KeyCode::Enter)),
        ComposerAction::Command {
            name: COMMANDS[COMMANDS.len() - 1].name.to_owned(),
            args: String::new(),
        }
    );
}

#[test]
fn tab_completes_the_selected_command_name() {
    let mut composer = ComposerState::default();
    composer.insert_paste("/set");
    composer.handle_key(key(KeyCode::Tab));

    assert_eq!(composer.text, "/settings");
    assert!(composer.palette_visible());
    assert_eq!(composer.cursor, "/settings".len());
}

#[test]
fn tab_adds_a_space_when_the_command_takes_an_argument() {
    let mut composer = ComposerState::default();
    composer.insert_paste("/exp");
    composer.handle_key(key(KeyCode::Tab));

    assert_eq!(composer.text, "/export ");
}

#[test]
fn enter_passes_typed_arguments_to_the_command() {
    let mut composer = ComposerState::default();
    composer.insert_paste("/export ./out.html");

    assert_eq!(
        composer.handle_key(key(KeyCode::Enter)),
        ComposerAction::Command {
            name: "export".to_owned(),
            args: "./out.html".to_owned(),
        }
    );
}

#[test]
fn escape_dismisses_the_palette_without_quitting() {
    let mut composer = ComposerState::default();
    composer.insert_paste("/settings");

    assert_eq!(composer.handle_key(key(KeyCode::Esc)), ComposerAction::None);
    assert!(!composer.palette_visible());
    assert_eq!(composer.text, "/settings");
    assert_eq!(
        composer.handle_key(key(KeyCode::Enter)),
        ComposerAction::Submit("/settings".to_owned())
    );
}

#[test]
fn unknown_slash_token_submits_as_plain_text() {
    let mut composer = ComposerState::default();
    composer.insert_paste("/zzzz");

    assert!(composer.palette_visible());
    assert_eq!(
        composer.handle_key(key(KeyCode::Enter)),
        ComposerAction::Submit("/zzzz".to_owned())
    );
}

#[test]
fn multiline_input_hides_the_palette() {
    let mut composer = ComposerState::default();
    composer.insert_paste("/");
    assert!(composer.palette_visible());

    composer.handle_key(modified_key(KeyCode::Enter, KeyModifiers::SHIFT));
    assert!(!composer.palette_visible());
    assert_eq!(composer.text, "/\n");
}

#[test]
fn palette_up_down_do_not_move_the_text_cursor() {
    let mut composer = ComposerState::default();
    composer.insert_paste("/abcdef");
    let cursor = composer.cursor;

    composer.handle_key_with_width(key(KeyCode::Up), 3);
    composer.handle_key_with_width(key(KeyCode::Down), 3);
    assert_eq!(composer.cursor, cursor);
    assert_eq!(composer.text, "/abcdef");
}
