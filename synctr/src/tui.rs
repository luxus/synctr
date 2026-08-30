use std::io::{self, stdout, BufRead, BufReader, Read};
use std::path::Path;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use synctr_engine::{
    read_last_run, resolve_rclone_live, spawn_sync, write_last_run, LastRun, Paths, Profile,
    ProfileStore, ResolvedRclone,
};

pub fn run(store: &ProfileStore, rclone_flag: Option<&Path>) -> synctr_engine::Result<()> {
    let mut terminal = setup()?;
    let result = event_loop(&mut terminal, store, rclone_flag);
    restore()?;
    result
}

fn setup() -> synctr_engine::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().map_err(synctr_engine::Error::from)?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen).map_err(synctr_engine::Error::from)?;
    Terminal::new(CrosstermBackend::new(out)).map_err(synctr_engine::Error::from)
}

fn restore() -> synctr_engine::Result<()> {
    disable_raw_mode().map_err(synctr_engine::Error::from)?;
    execute!(stdout(), LeaveAlternateScreen).map_err(synctr_engine::Error::from)?;
    Ok(())
}

struct Row {
    profile: Profile,
    last: Option<LastRun>,
}

struct Running {
    name: String,
    child: Child,
}

struct Ui {
    rows: Vec<Row>,
    state: ListState,
    log: Arc<Mutex<Vec<String>>>,
    running: Option<Running>,
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    store: &ProfileStore,
    rclone_flag: Option<&Path>,
) -> synctr_engine::Result<()> {
    let mut ui = load_ui(store)?;
    loop {
        reap(&mut ui, store.paths())?;
        terminal
            .draw(|frame| draw(frame, &mut ui, rclone_flag))
            .map_err(synctr_engine::Error::from)?;
        if !event::poll(Duration::from_millis(120)).map_err(synctr_engine::Error::from)? {
            continue;
        }
        let Event::Key(key) = event::read().map_err(synctr_engine::Error::from)? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                stop_running(&mut ui, store.paths())?;
                return Ok(());
            }
            KeyCode::Char('j') | KeyCode::Down => move_sel(&mut ui, 1),
            KeyCode::Char('k') | KeyCode::Up => move_sel(&mut ui, -1),
            KeyCode::Enter => toggle_selected(&mut ui, store, rclone_flag)?,
            _ => {}
        }
    }
}

fn load_ui(store: &ProfileStore) -> synctr_engine::Result<Ui> {
    let profiles = store.list()?;
    let rows = profiles
        .into_iter()
        .map(|profile| {
            let last = read_last_run(store.paths(), &profile.name)?;
            Ok(Row { profile, last })
        })
        .collect::<synctr_engine::Result<Vec<_>>>()?;
    let mut state = ListState::default();
    if !rows.is_empty() {
        state.select(Some(0));
    }
    Ok(Ui {
        rows,
        state,
        log: Arc::new(Mutex::new(Vec::new())),
        running: None,
    })
}

fn refresh_last(ui: &mut Ui, paths: &Paths, name: &str) -> synctr_engine::Result<()> {
    let last = read_last_run(paths, name)?;
    if let Some(row) = ui.rows.iter_mut().find(|r| r.profile.name == name) {
        row.last = last;
    }
    Ok(())
}

fn move_sel(ui: &mut Ui, delta: i32) {
    let len = ui.rows.len() as i32;
    if len == 0 {
        return;
    }
    let cur = ui.state.selected().unwrap_or(0) as i32;
    ui.state.select(Some((cur + delta).rem_euclid(len) as usize));
}

fn selected_profile(ui: &Ui) -> Option<&Profile> {
    ui.state
        .selected()
        .and_then(|i| ui.rows.get(i))
        .map(|r| &r.profile)
}

fn is_running(ui: &Ui, name: &str) -> bool {
    ui.running.as_ref().is_some_and(|r| r.name == name)
}

fn toggle_selected(
    ui: &mut Ui,
    store: &ProfileStore,
    rclone_flag: Option<&Path>,
) -> synctr_engine::Result<()> {
    let Some(name) = selected_profile(ui).map(|p| p.name.clone()) else {
        return Ok(());
    };
    if is_running(ui, &name) {
        stop_running(ui, store.paths())?;
        push_log(&ui.log, format!("stopped {name}"));
        return Ok(());
    }
    if ui.running.is_some() {
        stop_running(ui, store.paths())?;
    }
    let profile = ui
        .rows
        .iter()
        .find(|r| r.profile.name == name)
        .map(|r| r.profile.clone())
        .expect("selected profile");
    let resolved = resolve_rclone_live(rclone_flag, profile.rclone.as_deref())?;
    start_profile(ui, store.paths(), &profile, resolved)
}

fn start_profile(
    ui: &mut Ui,
    paths: &Paths,
    profile: &Profile,
    resolved: ResolvedRclone,
) -> synctr_engine::Result<()> {
    push_log(
        &ui.log,
        format!("--- {} {} ---", profile.mode, profile.name),
    );
    let mut child = spawn_sync(paths, profile, &resolved)?;
    if let Some(out) = child.stdout.take() {
        spawn_pipe_reader(out, ui.log.clone());
    }
    if let Some(err) = child.stderr.take() {
        spawn_pipe_reader(err, ui.log.clone());
    }
    ui.running = Some(Running {
        name: profile.name.clone(),
        child,
    });
    Ok(())
}

fn stop_running(ui: &mut Ui, paths: &Paths) -> synctr_engine::Result<()> {
    let Some(mut running) = ui.running.take() else {
        return Ok(());
    };
    let _ = running.child.kill();
    let status = running.child.wait().map_err(synctr_engine::Error::from)?;
    let code = status.code().unwrap_or(1);
    write_last_run(paths, &running.name, LastRun::now(code))?;
    refresh_last(ui, paths, &running.name)?;
    Ok(())
}

fn reap(ui: &mut Ui, paths: &Paths) -> synctr_engine::Result<()> {
    let status = {
        let Some(running) = ui.running.as_mut() else {
            return Ok(());
        };
        running.child.try_wait().map_err(synctr_engine::Error::from)?
    };
    let Some(status) = status else {
        return Ok(());
    };
    let name = ui.running.take().expect("running").name;
    let code = status.code().unwrap_or(1);
    write_last_run(paths, &name, LastRun::now(code))?;
    refresh_last(ui, paths, &name)?;
    push_log(&ui.log, format!("{name} exit {code}"));
    Ok(())
}

fn spawn_pipe_reader<R: Read + Send + 'static>(reader: R, log: Arc<Mutex<Vec<String>>>) {
    thread::spawn(move || {
        let mut buf = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match buf.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
                    if !trimmed.is_empty() {
                        push_log(&log, trimmed);
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn push_log(log: &Mutex<Vec<String>>, line: String) {
    let Ok(mut g) = log.lock() else {
        return;
    };
    g.push(line);
    let extra = g.len().saturating_sub(500);
    if extra > 0 {
        g.drain(..extra);
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, ui: &mut Ui, rclone_flag: Option<&Path>) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(8),
            Constraint::Length(1),
        ])
        .split(frame.area());
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(root[0]);

    let items: Vec<ListItem> = ui
        .rows
        .iter()
        .map(|row| {
            let mark = if is_running(ui, &row.profile.name) {
                " running"
            } else {
                ""
            };
            ListItem::new(format!("{}{mark}", row.profile.name))
        })
        .collect();
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title("profiles"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("> "),
        top[0],
        &mut ui.state,
    );

    frame.render_widget(detail_pane(ui, rclone_flag), top[1]);
    frame.render_widget(log_pane(ui, root[1].height), root[1]);
    frame.render_widget(
        Paragraph::new("enter start/stop  j/k  q"),
        root[2],
    );
}

fn detail_pane(ui: &Ui, rclone_flag: Option<&Path>) -> Paragraph<'static> {
    let block = Block::default().borders(Borders::ALL).title("state");
    let Some(i) = ui.state.selected() else {
        return Paragraph::new("no profiles. synctr profile add <name> --local PATH --remote remote:path --mode sync")
            .block(block)
            .wrap(Wrap { trim: true });
    };
    let Some(row) = ui.rows.get(i) else {
        return Paragraph::new("").block(block);
    };
    let p = &row.profile;
    let rclone = match resolve_rclone_live(rclone_flag, p.rclone.as_deref()) {
        Ok(r) => format!("{} ({})", r.path.display(), r.source.explain()),
        Err(synctr_engine::Error::RcloneNotFound) => "not found".into(),
        Err(e) => e.to_string(),
    };
    let last = match &row.last {
        Some(run) if run.ok => format!("ok {} ({})", age_label(run.finished_at_unix), run.finished_at),
        Some(run) => format!(
            "exit {} {} ({})",
            run.exit_code,
            age_label(run.finished_at_unix),
            run.finished_at
        ),
        None => "never".into(),
    };
    let state = if is_running(ui, &p.name) {
        "running"
    } else {
        "idle"
    };
    let text = vec![
        Line::from(Span::styled(p.name.clone(), Style::default().add_modifier(Modifier::BOLD))),
        Line::from(format!("mode    {}", p.mode)),
        Line::from(format!("local   {}", p.local.display())),
        Line::from(format!("remote  {}", p.remote)),
        Line::from(format!("rclone  {rclone}")),
        Line::from(format!("last    {last}")),
        Line::from(format!("state   {state}")),
    ];
    Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: true })
}

fn log_pane(ui: &Ui, height: u16) -> Paragraph<'static> {
    let inner = height.saturating_sub(2) as usize;
    let lines = match ui.log.lock() {
        Ok(g) => {
            let start = g.len().saturating_sub(inner.max(1));
            g[start..]
                .iter()
                .map(|s| Line::from(s.clone()))
                .collect::<Vec<_>>()
        }
        Err(_) => Vec::new(),
    };
    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("rclone log"))
        .wrap(Wrap { trim: false })
}

fn age_label(unix: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let d = (now - unix).max(0);
    if d < 60 {
        format!("{d}s ago")
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86400 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}d ago", d / 86400)
    }
}
