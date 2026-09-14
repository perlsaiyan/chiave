//! Terminal ownership: the panic hook, the event loop and [`run`].

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
use ratatui::DefaultTerminal;

use chiave_clip::Clipboard;
use chiave_core::Vault;

use crate::app::App;
use crate::{PasswordPrompt, TuiOptions};

/// How often the UI wakes up on its own: TOTP countdown, clipboard countdown, idle lock.
const TICK: Duration = Duration::from_millis(250);

/// Leave raw mode, mouse capture and the alternate screen before the default
/// panic handler prints, so a crash does not leave the terminal unusable.
///
/// Mouse capture is turned off unconditionally: asking a terminal to stop
/// reporting mouse events it was never reporting is harmless, and that keeps the
/// hook independent of [`TuiOptions::mouse`].
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        previous(info);
    }));
}

/// Start reporting mouse events, once the alternate screen is up.
fn enable_mouse(on: bool) {
    if on {
        let _ = execute!(io::stdout(), EnableMouseCapture);
    }
}

/// Stop reporting them, before handing the terminal back.
fn disable_mouse(on: bool) {
    if on {
        let _ = execute!(io::stdout(), DisableMouseCapture);
    }
}

/// Run the TUI over `vault` until the user quits.
pub fn run(
    vault: Vault,
    clip: Box<dyn Clipboard>,
    opts: TuiOptions,
    prompt: Box<dyn PasswordPrompt>,
) -> anyhow::Result<()> {
    install_panic_hook();
    let mouse = opts.mouse;
    let mut terminal = ratatui::try_init()?;
    enable_mouse(mouse);
    let mut app = App::new(vault, clip, opts, prompt);
    let result = event_loop(&mut app, &mut terminal, mouse);
    disable_mouse(mouse);
    ratatui::restore();
    result
}

fn event_loop(app: &mut App, terminal: &mut DefaultTerminal, mouse: bool) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| app.draw(frame))?;
        if app.should_quit() {
            return Ok(());
        }
        if event::poll(TICK)? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Mouse(ev) => app.handle_mouse(ev),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        app.tick(Instant::now());
        if app.wants_external_prompt() {
            // Hand the real terminal back so the prompt can read a password,
            // then take it again.
            disable_mouse(mouse);
            ratatui::restore();
            let outcome = app.run_external_prompt();
            *terminal = ratatui::try_init()?;
            enable_mouse(mouse);
            terminal.clear()?;
            if let Err(e) = outcome {
                app.note_prompt_error(&e.to_string());
            }
        }
    }
}
