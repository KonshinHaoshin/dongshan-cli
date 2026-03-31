use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::query::engine::QueryEngine;
use crate::query::history;
use crate::query::turn_runner::ExecutionMode;

pub async fn run_terminal_app(
    session: String,
    mode: ExecutionMode,
    initial_task: Option<String>,
) -> Result<()> {
    let mut engine = QueryEngine::new(session.clone(), mode);
    let mut input = String::new();
    let mut transcript = history::transcript_lines(&history::load(&session)?);
    let mut terminal = enter_terminal()?;

    if let Some(task) = initial_task {
        leave_terminal(&mut terminal)?;
        let reply = engine.handle_input(&task, mode).await?;
        transcript = history::transcript_lines(&history::load(&session)?);
        terminal = enter_terminal()?;
        if reply.exit {
            leave_terminal(&mut terminal)?;
            return Ok(());
        }
    }

    loop {
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(8),
                    Constraint::Length(8),
                    Constraint::Length(3),
                ])
                .split(frame.area());

            let transcript_widget = Paragraph::new(Text::from(
                transcript
                    .iter()
                    .map(|line| Line::raw(line.clone()))
                    .collect::<Vec<_>>(),
            ))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Session {} [{}]", session, mode.as_str())),
            )
            .wrap(Wrap { trim: false });
            frame.render_widget(transcript_widget, chunks[0]);

            let output = Paragraph::new(engine.runtime.last_output.clone())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("Status: {}", engine.runtime.last_status)),
                )
                .wrap(Wrap { trim: false });
            frame.render_widget(output, chunks[1]);

            let input_widget = Paragraph::new(input.as_str())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Input")
                        .border_style(Style::default().add_modifier(Modifier::BOLD)),
                );
            frame.render_widget(input_widget, chunks[2]);
        })?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char(c) => input.push(c),
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Esc => break,
                    KeyCode::Enter => {
                        let submitted = input.trim().to_string();
                        input.clear();
                        if submitted.is_empty() {
                            continue;
                        }
                        leave_terminal(&mut terminal)?;
                        let reply = engine.handle_input(&submitted, mode).await?;
                        transcript = history::transcript_lines(&history::load(&session)?);
                        terminal = enter_terminal()?;
                        if reply.exit {
                            break;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    leave_terminal(&mut terminal)?;
    Ok(())
}

fn enter_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
