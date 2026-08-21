mod app;
mod backend;
mod config;
mod frontend;

use std::{
    io::stdout,
    time::{Duration, Instant},
};

use anyhow::Result;
use app::{App, AppAction, LaunchOptions};
use clap::Parser;
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

#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Resume the most recently used session for this project
    #[arg(short = 'c', long = "continue", conflicts_with_all = ["resume", "no_session", "session"])]
    continue_session: bool,
    /// Open the saved-session picker at startup
    #[arg(short = 'r', long, conflicts_with_all = ["continue_session", "no_session", "session"])]
    resume: bool,
    /// Run without creating or writing session files
    #[arg(long, conflicts_with_all = ["continue_session", "resume", "session"])]
    no_session: bool,
    /// Resume a specific session database
    #[arg(long, value_name = "PATH", conflicts_with_all = ["continue_session", "resume", "no_session"])]
    session: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
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
        Ok(()) => run(
            &mut terminal,
            LaunchOptions {
                continue_session: args.continue_session,
                resume: args.resume,
                no_session: args.no_session,
                session: args.session,
            },
        ),
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

fn run(terminal: &mut DefaultTerminal, launch: LaunchOptions) -> Result<()> {
    let runtime = tokio::runtime::Handle::current();
    let mut app = App::new_with_launch(runtime, launch);
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
