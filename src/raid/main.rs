mod app;
mod backend;
mod config;
mod frontend;

use std::{io::stdout, time::Duration};

use anyhow::Result;
use app::{App, AppAction};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
};
use ratatui::DefaultTerminal;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = runtime.enter();
    crate::app::install_default_stream_fn();

    let mut terminal = ratatui::init();
    let result = match execute!(
        stdout(),
        EnableBracketedPaste,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
    ) {
        Ok(()) => run(&mut terminal),
        Err(error) => Err(error.into()),
    };
    ratatui::restore();
    let cleanup = execute!(
        stdout(),
        DisableBracketedPaste,
        PopKeyboardEnhancementFlags,
        Show
    );

    result.and(cleanup.map_err(anyhow::Error::from))
}

fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    let runtime = tokio::runtime::Handle::current();
    let mut app = App::new(runtime);
    let mut content_width = 0;

    loop {
        app.tick();
        terminal.draw(|frame| {
            content_width = app.draw(frame);
        })?;
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.handle_key(key, content_width) == AppAction::Quit {
                        break;
                    }
                }
                Event::Paste(pasted) => app.insert_paste(&pasted),
                _ => {}
            }
        }
    }
    Ok(())
}
