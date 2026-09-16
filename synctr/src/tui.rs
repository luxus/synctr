use std::io::{self, stdout, BufRead, BufReader, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use synctr_engine::{
    age_label, display_rclone_line, live_progress, read_last_run, resolve_rclone_live, spawn_sync,
    write_last_run, LastRun, Paths, Profile, ProfileStore, ProgressSink, RcloneJson,
    ResolvedRclone, SyncChild, TransferProgress,
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
    child: SyncChild,
}

struct Ui {
    rows: Vec<Row>,
    state: ListState,
    log: Arc<Mutex<Vec<String>>>,
    log_offset: usize,
    log_height: usize,
    running: Option<Running>,
    help: bool,
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
            .draw(|frame| draw(frame, &mut ui, rclone_flag, store.paths()))
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
        if ui.help {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    stop_running(&mut ui, store.paths())?;
                    return Ok(());
                }
                KeyCode::Char('?') | KeyCode::Char('h') => ui.help = false,
                _ => {}
            }
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                stop_running(&mut ui, store.paths())?;
                return Ok(());
            }
            KeyCode::Char('?') | KeyCode::Char('h') => ui.help = true,
            KeyCode::Char('j') | KeyCode::Down => move_sel(&mut ui, 1),
            KeyCode::Char('k') | KeyCode::Up => move_sel(&mut ui, -1),
            KeyCode::PageUp => scroll_log(&mut ui, 1),
            KeyCode::PageDown => scroll_log(&mut ui, -1),
            KeyCode::Char('r') => reload_profiles(&mut ui, store)?,
            KeyCode::Enter => toggle_selected(&mut ui, store, rclone_flag, false)?,
            KeyCode::Char('d') => toggle_selected(&mut ui, store, rclone_flag, true)?,
            _ => {}
        }
    }
}

fn load_ui(store: &ProfileStore) -> synctr_engine::Result<Ui> {
    let mut ui = Ui {
        rows: Vec::new(),
        state: ListState::default(),
        log: Arc::new(Mutex::new(Vec::new())),
        log_offset: 0,
        log_height: 1,
        running: None,
        help: false,
    };
    load_rows(&mut ui, store)?;
    Ok(ui)
}

fn load_rows(ui: &mut Ui, store: &ProfileStore) -> synctr_engine::Result<()> {
    let selected = selected_profile(ui).map(|p| p.name.clone());
    let profiles = store.list()?;
    ui.rows = profiles
        .into_iter()
        .map(|profile| {
            let last = read_last_run(store.paths(), &profile.name)?;
            Ok(Row { profile, last })
        })
        .collect::<synctr_engine::Result<Vec<_>>>()?;
    let idx = selected
        .and_then(|name| ui.rows.iter().position(|r| r.profile.name == name))
        .or(if ui.rows.is_empty() { None } else { Some(0) });
    ui.state.select(idx);
    Ok(())
}

fn reload_profiles(ui: &mut Ui, store: &ProfileStore) -> synctr_engine::Result<()> {
    load_rows(ui, store)?;
    push_log(&ui.log, "reloaded profiles".into());
    Ok(())
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
    ui.state
        .select(Some((cur + delta).rem_euclid(len) as usize));
}

fn log_window(len: usize, height: usize, offset: usize) -> (usize, usize) {
    let inner = height.max(1);
    let max_off = len.saturating_sub(inner);
    let off = offset.min(max_off);
    let end = len.saturating_sub(off);
    let start = end.saturating_sub(inner);
    (start, end)
}

fn scroll_log(ui: &mut Ui, pages: i32) {
    let len = ui.log.lock().map(|g| g.len()).unwrap_or(0);
    let inner = ui.log_height.max(1);
    let max_off = len.saturating_sub(inner);
    let delta = pages * inner as i32;
    let next = ui.log_offset as i32 + delta;
    ui.log_offset = next.clamp(0, max_off as i32) as usize;
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
    dry_run: bool,
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
    start_profile(ui, store.paths(), &profile, resolved, dry_run)
}

fn start_profile(
    ui: &mut Ui,
    paths: &Paths,
    profile: &Profile,
    resolved: ResolvedRclone,
    dry_run: bool,
) -> synctr_engine::Result<()> {
    let tag = if dry_run {
        "dry-run"
    } else {
        profile.mode.as_str()
    };
    push_log(&ui.log, format!("--- {tag} {} ---", profile.name));
    let mut child = spawn_sync(paths, profile, &resolved, dry_run)?;
    let progress = child.progress.clone();
    if let Some(out) = child.stdout.take() {
        spawn_pipe_reader(out, ui.log.clone(), progress.clone());
    }
    if let Some(err) = child.stderr.take() {
        spawn_pipe_reader(err, ui.log.clone(), progress);
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
        running
            .child
            .try_wait()
            .map_err(synctr_engine::Error::from)?
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

fn spawn_pipe_reader<R: Read + Send + 'static>(
    reader: R,
    log: Arc<Mutex<Vec<String>>>,
    progress: ProgressSink,
) {
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
                        progress.push_line(&trimmed);
                        push_log(&log, display_rclone_line(&trimmed));
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

fn draw(frame: &mut ratatui::Frame<'_>, ui: &mut Ui, rclone_flag: Option<&Path>, paths: &Paths) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
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
            let mark = running_mark(ui, paths, &row.profile.name);
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

    frame.render_widget(detail_pane(ui, rclone_flag, paths), top[1]);
    ui.log_height = root[1].height.saturating_sub(2) as usize;
    frame.render_widget(log_pane(ui, root[1].height), root[1]);
    frame.render_widget(
        Paragraph::new("enter start/stop  d dry-run  j/k  pgup/pgdn log  r reload  ? help  q"),
        root[2],
    );
    if ui.help {
        frame.render_widget(Clear, frame.area());
        frame.render_widget(help_pane(), frame.area());
    }
}

fn help_pane() -> Paragraph<'static> {
    let text = vec![
        Line::from(Span::styled(
            "keys",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("enter     start or stop the selected profile"),
        Line::from("d         dry-run the selected profile (rclone --dry-run)"),
        Line::from("j / k     next / previous profile"),
        Line::from("pgup/pgdn scroll rclone log"),
        Line::from("r         reload profiles from disk"),
        Line::from("? / h     close this pane"),
        Line::from("q / esc   quit"),
        Line::from(""),
        Line::from("one rclone child at a time. Enter on another profile stops the current one."),
        Line::from("state pane shows live rclone transfer progress (bytes, %, ETA, file)."),
        Line::from("directory watch is `synctr watch`, not this TUI."),
    ];
    Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("help"))
        .wrap(Wrap { trim: false })
}

fn running_mark(ui: &Ui, paths: &Paths, name: &str) -> String {
    match live_progress(paths, name).or_else(|| {
        if is_running(ui, name) {
            Some(TransferProgress::starting(None, false))
        } else {
            None
        }
    }) {
        Some(p) => match p.percent {
            Some(pct) => format!(" running {pct}%"),
            None => " running".into(),
        },
        None => String::new(),
    }
}

fn detail_pane(ui: &Ui, rclone_flag: Option<&Path>, paths: &Paths) -> Paragraph<'static> {
    let block = Block::default().borders(Borders::ALL).title("state");
    let Some(i) = ui.state.selected() else {
        return Paragraph::new(
            "no profiles. synctr profile add <name> --local PATH --remote remote:path --mode sync",
        )
        .block(block)
        .wrap(Wrap { trim: true });
    };
    let Some(row) = ui.rows.get(i) else {
        return Paragraph::new("").block(block);
    };
    let p = &row.profile;
    let rclone = match RcloneJson::from_live(rclone_flag, p.rclone.as_deref()) {
        Ok(j) => j.human_line(),
        Err(e) => e.to_string(),
    };
    let last = match &row.last {
        Some(run) if run.ok => format!(
            "ok {} ({})",
            age_label(run.finished_at_unix),
            run.finished_at
        ),
        Some(run) => format!(
            "exit {} {} ({})",
            run.exit_code,
            age_label(run.finished_at_unix),
            run.finished_at
        ),
        None => "never".into(),
    };
    let xfer = live_progress(paths, &p.name);
    let state = if xfer.is_some() || is_running(ui, &p.name) {
        "running"
    } else {
        "idle"
    };
    let mut text = vec![
        Line::from(Span::styled(
            p.name.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("mode    {}", p.mode)),
        Line::from(format!("local   {}", p.local.display())),
        Line::from(format!("remote  {}", p.remote)),
        Line::from(format!("rclone  {rclone}")),
        Line::from(format!("last    {last}")),
        Line::from(format!("state   {state}")),
    ];
    if let Some(xfer) = xfer {
        text.push(Line::from(format!("xfer    {}", xfer.summary())));
        if let Some(file) = xfer.file {
            text.push(Line::from(format!("file    {file}")));
        }
    }
    Paragraph::new(text).block(block).wrap(Wrap { trim: true })
}

fn log_pane(ui: &Ui, height: u16) -> Paragraph<'static> {
    let inner = height.saturating_sub(2) as usize;
    let lines = match ui.log.lock() {
        Ok(g) => {
            let (start, end) = log_window(g.len(), inner, ui.log_offset);
            g[start..end]
                .iter()
                .map(|s| Line::from(s.clone()))
                .collect::<Vec<_>>()
        }
        Err(_) => Vec::new(),
    };
    let title = if ui.log_offset == 0 {
        "rclone log".to_string()
    } else {
        format!("rclone log  +{}", ui.log_offset)
    };
    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false })
}

#[cfg(test)]
mod tests {
    use super::log_window;

    #[test]
    fn log_window_pins_to_tail_then_scrolls() {
        assert_eq!(log_window(10, 4, 0), (6, 10));
        assert_eq!(log_window(10, 4, 2), (4, 8));
        assert_eq!(log_window(10, 4, 99), (0, 4));
        assert_eq!(log_window(3, 8, 0), (0, 3));
        assert_eq!(log_window(0, 4, 0), (0, 0));
    }
}
