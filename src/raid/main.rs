mod app;
mod backend;
mod config;
mod frontend;

use std::{
    io::stdout,
    time::{Duration, Instant},
};

use anyhow::Result;
use app::{App, AppAction};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
};
use ratatui::DefaultTerminal;
use tracing_subscriber::EnvFilter;

const FRAME_INTERVAL: Duration = Duration::from_nanos(8_333_333);

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
        EnableMouseCapture,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
    ) {
        Ok(()) => run(&mut terminal),
        Err(error) => Err(error.into()),
    };
    ratatui::restore();
    let cleanup = execute!(
        stdout(),
        DisableBracketedPaste,
        DisableMouseCapture,
        PopKeyboardEnhancementFlags,
        Show
    );

    result.and(cleanup.map_err(anyhow::Error::from))
}

fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    let runtime = tokio::runtime::Handle::current();
    let mut app = App::new(runtime);
    let mut content_width = 0;

    'app: loop {
        let frame_started = Instant::now();
        app.tick();
        terminal.draw(|frame| {
            content_width = app.draw(frame);
        })?;

        loop {
            let remaining = FRAME_INTERVAL.saturating_sub(frame_started.elapsed());
            if remaining.is_zero() || !event::poll(remaining)? {
                break;
            }
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.handle_key(key, content_width) == AppAction::Quit {
                        break 'app;
                    }
                }
                Event::Paste(pasted) => app.insert_paste(&pasted),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }
    }
    Ok(())
}
