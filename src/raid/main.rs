#[allow(dead_code)]
mod app;
#[allow(dead_code)]
mod backend;
mod frontend;

use std::{io::stdout, time::Duration};

use anyhow::Result;
use crossterm::{
    cursor::{SetCursorStyle, Show},
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind},
    execute,
};
use ratatui::DefaultTerminal;
use ratatui::layout::{Margin, Rect};
use tracing_subscriber::EnvFilter;

use frontend::composer::{ComposerAction, ComposerState, ComposerWidget};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let mut terminal = ratatui::init();
    let result = match execute!(stdout(), EnableBracketedPaste, SetCursorStyle::SteadyBlock) {
        Ok(()) => run(&mut terminal),
        Err(error) => Err(error.into()),
    };
    ratatui::restore();
    let cleanup = execute!(
        stdout(),
        DisableBracketedPaste,
        SetCursorStyle::DefaultUserShape,
        Show
    );

    result.and(cleanup.map_err(anyhow::Error::from))
}

fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut composer = ComposerState::default();

    loop {
        terminal.draw(|frame| draw(frame, &composer))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match composer.handle_key(key) {
                        ComposerAction::Quit => break,
                        ComposerAction::Submit(message) => {
                            tracing::info!(message = %message, "message submitted");
                        }
                        ComposerAction::None => {}
                    }
                }
                Event::Paste(pasted) => composer.insert_paste(&pasted),
                _ => {}
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut ratatui::Frame, composer: &ComposerState) {
    let padded = frame.area().inner(Margin {
        vertical: 1,
        horizontal: 3,
    });
    if padded.width == 0 || padded.height < 3 {
        return;
    }

    let composer_area = Rect {
        x: padded.x,
        y: padded.y + padded.height - 3,
        width: padded.width,
        height: 3,
    };
    frame.render_widget(ComposerWidget::new(composer), composer_area);
    if let Some(cursor) = ComposerWidget::cursor_position(composer_area, composer) {
        frame.set_cursor_position(cursor);
    }
}
