//! Terminal UI: local files on the left, calculator on the right.

pub mod kitty;
mod ui;
mod worker;

use haccerlink::calc::{CalcInfo, CalcVar, Screenshot};
use haccerlink::keys;
use haccerlink::tifiles::{self, Piece, TiFile};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::TableState;
use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
use worker::{Job, Msg};

#[derive(Clone)]
pub struct Options {
    pub sim: bool,
    pub device: Option<String>,
    pub timeout: Duration,
    pub log_hex: Option<PathBuf>,
    pub images: kitty::ImagePref,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Local,
    Calc,
}

#[derive(Clone)]
struct LocalEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LogKind {
    Info,
    Error,
}

enum Confirm {
    Delete(Vec<CalcVar>),
    SyncClock,
}

enum Mode {
    Normal,
    Confirm(Confirm),
    Rename { var: CalcVar, input: String },
    Screen,
    Remote,
    Info,
    Help,
}

const LOG_LINES: usize = 500;

pub struct App {
    opts: Options,
    jobs: Sender<Job>,
    msgs: Receiver<Msg>,

    focus: Pane,
    mode: Mode,
    quit: bool,

    local_dir: PathBuf,
    local: Vec<LocalEntry>,
    local_state: TableState,
    local_sel: BTreeSet<PathBuf>,
    show_all_files: bool,
    local_detail: Option<(PathBuf, String)>,

    vars: Vec<CalcVar>,
    calc_state: TableState,
    calc_sel: BTreeSet<(String, u8)>,
    mem: Option<(u64, u64)>,

    connected: Option<(String, CalcInfo)>,
    busy: Option<String>,
    progress: Option<(usize, usize)>,
    log: VecDeque<(LogKind, String)>,
    packets: VecDeque<String>,
    show_packets: bool,
    screen: Option<Screenshot>,
    info: Option<CalcInfo>,
    /// Set when screenshots are drawn with the kitty graphics protocol.
    kitty: Option<kitty::KittyImage>,
    /// Bumped for every new screenshot so the image is only re-sent when needed.
    screen_gen: u64,
    /// Where ui::draw left room for the kitty image this frame.
    image_area: Option<ratatui::layout::Rect>,
}

fn is_ti_file(name: &str) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    // .8xp .8ek .8ci ..., TI Connect CE bundles (.b84/.b83), TiLP groups (.tig)
    let single = ext.len() >= 3
        && ext.len() <= 5
        && ext.starts_with('8')
        && ext[1..].chars().all(|c| c.is_ascii_alphanumeric());
    single || matches!(ext.as_str(), "b84" | "b83" | "tig")
}

impl App {
    fn new(opts: Options) -> Self {
        let kitty = kitty::use_kitty(opts.images).then(kitty::KittyImage::default);
        let (job_tx, job_rx) = channel();
        let (msg_tx, msg_rx) = channel();
        worker::spawn(opts.clone(), msg_tx, job_rx);
        let mut app = Self {
            opts,
            jobs: job_tx,
            msgs: msg_rx,
            focus: Pane::Calc,
            mode: Mode::Normal,
            quit: false,
            local_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            local: Vec::new(),
            local_state: TableState::default(),
            local_sel: BTreeSet::new(),
            show_all_files: false,
            local_detail: None,
            vars: Vec::new(),
            calc_state: TableState::default(),
            calc_sel: BTreeSet::new(),
            mem: None,
            connected: None,
            busy: None,
            progress: None,
            log: VecDeque::new(),
            packets: VecDeque::new(),
            show_packets: false,
            screen: None,
            info: None,
            kitty,
            screen_gen: 0,
            image_area: None,
        };
        app.reload_local();
        app.job(Job::Connect);
        app
    }

    fn job(&self, j: Job) {
        let _ = self.jobs.send(j);
    }

    fn push_log(&mut self, kind: LogKind, s: String) {
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back((kind, s));
    }

    fn reload_local(&mut self) {
        let mut entries = Vec::new();
        if let Some(parent) = self.local_dir.parent() {
            entries.push(LocalEntry {
                name: "..".into(),
                path: parent.to_path_buf(),
                is_dir: true,
                size: 0,
            });
        }
        let mut items: Vec<LocalEntry> = std::fs::read_dir(&self.local_dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') {
                            return None;
                        }
                        let md = e.metadata().ok()?;
                        let is_dir = md.is_dir();
                        if !is_dir && !self.show_all_files && !is_ti_file(&name) {
                            return None;
                        }
                        Some(LocalEntry {
                            name,
                            path: e.path(),
                            is_dir,
                            size: md.len(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        items.sort_by_key(|a| (!a.is_dir, a.name.to_lowercase()));
        entries.extend(items);
        self.local = entries;
        let sel = self
            .local_state
            .selected()
            .unwrap_or(0)
            .min(self.local.len().saturating_sub(1));
        self.local_state.select(if self.local.is_empty() {
            None
        } else {
            Some(sel)
        });
        self.local_sel.retain(|p| p.exists());
        self.update_local_detail();
    }

    fn update_local_detail(&mut self) {
        let Some(e) = self.local_state.selected().and_then(|i| self.local.get(i)) else {
            self.local_detail = None;
            return;
        };
        if self
            .local_detail
            .as_ref()
            .is_some_and(|(p, _)| p == &e.path)
        {
            return;
        }
        let text = if e.is_dir {
            "directory".to_string()
        } else if e.size > 8 * 1024 * 1024 {
            format!("{} bytes", e.size)
        } else {
            match TiFile::load(&e.path) {
                Ok(TiFile::Bundle(b)) => {
                    let (pieces, _) = TiFile::Bundle(b.clone()).pieces();
                    let os = pieces.iter().filter(|p| matches!(p, Piece::Os(_))).count();
                    let apps = pieces.iter().filter(|p| matches!(p, Piece::App(_))).count();
                    let vars = pieces.len() - os - apps;
                    let mut t = format!("bundle: {vars} vars, {apps} apps");
                    if os > 0 {
                        t += &format!(", {os} OS (won't be sent)");
                    }
                    if b.checksum_ok == Some(false) {
                        t += ", BAD CHECKSUM";
                    }
                    t
                }
                Ok(file) => file
                    .pieces()
                    .0
                    .iter()
                    .map(|p| match p {
                        Piece::Var(x) => format!(
                            "{} {} {}b{}",
                            x.wire_name(),
                            haccerlink::names::type_name(x.type_id),
                            x.data.len(),
                            if x.archived { " arc" } else { "" }
                        ),
                        other => tifiles::describe(other),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                Err(err) => format!("{err}"),
            }
        };
        self.local_detail = Some((e.path.clone(), text));
    }

    fn handle_msg(&mut self, m: Msg) {
        match m {
            Msg::Log(s) => self.push_log(LogKind::Info, s),
            Msg::Error(s) => self.push_log(LogKind::Error, s),
            Msg::Packet(s) => {
                if self.packets.len() >= LOG_LINES {
                    self.packets.pop_front();
                }
                self.packets.push_back(s);
            }
            Msg::Progress(p) => self.progress = p,
            Msg::Busy(b) => self.busy = b,
            Msg::Connected(desc, info) => {
                self.push_log(
                    LogKind::Info,
                    format!(
                        "connected: {} ({}), OS {}",
                        info.model(),
                        desc,
                        info.os_version.as_deref().unwrap_or("?")
                    ),
                );
                self.connected = Some((desc, info));
            }
            Msg::Disconnected(why) => {
                self.connected = None;
                self.push_log(
                    LogKind::Error,
                    format!("link error: {why} (press c to reconnect)"),
                );
                if matches!(self.mode, Mode::Remote) {
                    self.mode = Mode::Screen;
                }
            }
            Msg::Vars(v, mem) => {
                self.vars = v;
                self.mem = mem;
                let keys: BTreeSet<_> = self
                    .vars
                    .iter()
                    .map(|v| (v.name.clone(), v.type_id))
                    .collect();
                self.calc_sel.retain(|k| keys.contains(k));
                let sel = self
                    .calc_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.vars.len().saturating_sub(1));
                self.calc_state.select(if self.vars.is_empty() {
                    None
                } else {
                    Some(sel)
                });
            }
            Msg::Info(i) => {
                self.info = Some(i);
                self.mode = Mode::Info;
            }
            Msg::Screen(s) => {
                self.screen = Some(s);
                self.screen_gen += 1;
                if matches!(self.mode, Mode::Normal) {
                    self.mode = Mode::Screen;
                }
            }
            Msg::LocalChanged => self.reload_local(),
        }
    }

    fn selected_vars(&self) -> Vec<CalcVar> {
        if !self.calc_sel.is_empty() {
            return self
                .vars
                .iter()
                .filter(|v| self.calc_sel.contains(&(v.name.clone(), v.type_id)))
                .cloned()
                .collect();
        }
        self.calc_state
            .selected()
            .and_then(|i| self.vars.get(i))
            .cloned()
            .into_iter()
            .collect()
    }

    fn selected_files(&self) -> Vec<PathBuf> {
        if !self.local_sel.is_empty() {
            return self.local_sel.iter().cloned().collect();
        }
        self.local_state
            .selected()
            .and_then(|i| self.local.get(i))
            .filter(|e| !e.is_dir)
            .map(|e| e.path.clone())
            .into_iter()
            .collect()
    }

    fn move_cursor(&mut self, delta: isize) {
        let (state, len) = match self.focus {
            Pane::Local => (&mut self.local_state, self.local.len()),
            Pane::Calc => (&mut self.calc_state, self.vars.len()),
        };
        if len == 0 {
            return;
        }
        let cur = state.selected().unwrap_or(0) as isize;
        state.select(Some((cur + delta).clamp(0, len as isize - 1) as usize));
        if self.focus == Pane::Local {
            self.update_local_detail();
        }
    }

    fn send_files(&mut self, archive: Option<bool>) {
        let files = self.selected_files();
        if files.is_empty() {
            self.push_log(
                LogKind::Error,
                "nothing to send: pick a file in the left pane".into(),
            );
            return;
        }
        self.local_sel.clear();
        self.job(Job::Send(files, archive));
    }

    fn get_vars(&mut self) {
        let vars = self.selected_vars();
        if vars.is_empty() {
            return;
        }
        self.calc_sel.clear();
        self.job(Job::Get(vars, self.local_dir.clone()));
    }

    fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.on_key_normal(k),
            Mode::Confirm(c) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => match c {
                    Confirm::Delete(v) => {
                        self.calc_sel.clear();
                        self.job(Job::Delete(v));
                    }
                    Confirm::SyncClock => self.job(Job::SyncClock),
                },
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.mode = Mode::Confirm(c),
            },
            Mode::Rename { var, mut input } => match k.code {
                KeyCode::Enter if !input.is_empty() => self.job(Job::Rename(var, input)),
                KeyCode::Esc => {}
                KeyCode::Backspace => {
                    input.pop();
                    self.mode = Mode::Rename { var, input };
                }
                KeyCode::Char(c) if input.chars().count() < 8 => {
                    input.push(c);
                    self.mode = Mode::Rename { var, input };
                }
                _ => self.mode = Mode::Rename { var, input },
            },
            Mode::Screen => match k.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('p') => {}
                KeyCode::Char('r') => {
                    self.job(Job::Screenshot);
                    self.mode = Mode::Screen;
                }
                KeyCode::Char('k') => self.mode = Mode::Remote,
                KeyCode::Char('s') | KeyCode::Char('w') => {
                    self.save_screen();
                    self.mode = Mode::Screen;
                }
                _ => self.mode = Mode::Screen,
            },
            Mode::Remote => {
                if k.code == KeyCode::Esc {
                    self.mode = Mode::Screen;
                    return;
                }
                self.mode = Mode::Remote;
                if let Some(code) = remote_key(k) {
                    self.job(Job::Key(code, true));
                }
            }
            Mode::Info | Mode::Help => {}
        }
    }

    fn save_screen(&mut self) {
        let Some(s) = &self.screen else { return };
        let name = format!(
            "screenshot-{}.png",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let path = self.local_dir.join(name);
        match s.write_png(&path) {
            Ok(()) => {
                self.push_log(LogKind::Info, format!("saved {}", path.display()));
                if self.show_all_files {
                    self.reload_local();
                }
            }
            Err(e) => self.push_log(LogKind::Error, format!("saving screenshot: {e}")),
        }
    }

    fn on_key_normal(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.mode = Mode::Help,
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Pane::Local => Pane::Calc,
                    Pane::Calc => Pane::Local,
                }
            }
            KeyCode::Left => self.focus = Pane::Local,
            KeyCode::Right => self.focus = Pane::Calc,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-15),
            KeyCode::PageDown => self.move_cursor(15),
            KeyCode::Home => self.move_cursor(isize::MIN / 2),
            KeyCode::End => self.move_cursor(isize::MAX / 2),
            KeyCode::Char(' ') => {
                match self.focus {
                    Pane::Local => {
                        if let Some(e) = self.local_state.selected().and_then(|i| self.local.get(i))
                            && !e.is_dir
                            && !self.local_sel.remove(&e.path)
                        {
                            self.local_sel.insert(e.path.clone());
                        }
                    }
                    Pane::Calc => {
                        if let Some(v) = self.calc_state.selected().and_then(|i| self.vars.get(i)) {
                            let key = (v.name.clone(), v.type_id);
                            if !self.calc_sel.remove(&key) {
                                self.calc_sel.insert(key);
                            }
                        }
                    }
                }
                self.move_cursor(1);
            }
            KeyCode::Enter => match self.focus {
                Pane::Local => {
                    let entry = self
                        .local_state
                        .selected()
                        .and_then(|i| self.local.get(i))
                        .cloned();
                    match entry {
                        Some(e) if e.is_dir => {
                            self.local_dir = e.path;
                            self.local_state.select(Some(0));
                            self.local_sel.clear();
                            self.reload_local();
                        }
                        Some(_) => self.send_files(None),
                        None => {}
                    }
                }
                Pane::Calc => self.get_vars(),
            },
            KeyCode::Backspace => {
                if let Some(p) = self.local_dir.parent() {
                    self.local_dir = p.to_path_buf();
                    self.local_state.select(Some(0));
                    self.reload_local();
                }
            }
            KeyCode::Char('s') => self.send_files(None),
            KeyCode::Char('S') => self.send_files(Some(true)),
            KeyCode::Char('g') => self.get_vars(),
            KeyCode::Char('d') | KeyCode::Delete if self.focus == Pane::Calc => {
                let v = self.selected_vars();
                if !v.is_empty() {
                    self.mode = Mode::Confirm(Confirm::Delete(v));
                }
            }
            KeyCode::Char('a') if self.focus == Pane::Calc => {
                let v = self.selected_vars();
                if let Some(first) = v.first() {
                    // Toggle based on the first one, like a file manager would.
                    let to = !first.archived;
                    let v: Vec<_> = v.into_iter().filter(|x| !x.is_app()).collect();
                    self.calc_sel.clear();
                    self.job(Job::SetArchived(v, to));
                }
            }
            KeyCode::Char('r') if self.focus == Pane::Calc => {
                if let Some(var) = self
                    .calc_state
                    .selected()
                    .and_then(|i| self.vars.get(i))
                    .cloned()
                {
                    self.mode = Mode::Rename {
                        input: var.name.clone(),
                        var,
                    };
                }
            }
            KeyCode::Char('x') if self.focus == Pane::Calc => {
                if let Some(var) = self
                    .calc_state
                    .selected()
                    .and_then(|i| self.vars.get(i))
                    .cloned()
                {
                    self.job(Job::Run(var));
                }
            }
            KeyCode::Char('R') | KeyCode::F(5) => {
                self.reload_local();
                self.job(Job::Refresh);
            }
            KeyCode::Char('c') => self.job(Job::Connect),
            KeyCode::Char('i') => self.job(Job::Info),
            KeyCode::Char('p') => {
                self.mode = Mode::Screen;
                self.job(Job::Screenshot);
            }
            KeyCode::Char('K') => {
                self.mode = Mode::Remote;
                self.job(Job::Screenshot);
            }
            KeyCode::Char('t') => self.mode = Mode::Confirm(Confirm::SyncClock),
            KeyCode::Char('l') => self.show_packets = !self.show_packets,
            KeyCode::Char('.') => {
                self.show_all_files = !self.show_all_files;
                self.reload_local();
            }
            _ => {}
        }
    }
}

/// Terminal key -> calculator key in remote control mode.
fn remote_key(k: KeyEvent) -> Option<u16> {
    Some(match k.code {
        KeyCode::Up => keys::UP,
        KeyCode::Down => keys::DOWN,
        KeyCode::Left => keys::LEFT,
        KeyCode::Right => keys::RIGHT,
        KeyCode::Enter => keys::ENTER,
        KeyCode::Backspace => keys::DEL,
        KeyCode::Delete => keys::CLEAR,
        KeyCode::Insert => keys::INS,
        KeyCode::F(1) => keys::YEQU,
        KeyCode::F(2) => keys::WINDOW,
        KeyCode::F(3) => keys::ZOOM,
        KeyCode::F(4) => keys::TRACE,
        KeyCode::F(5) => keys::GRAPH,
        KeyCode::F(6) => keys::MODE,
        KeyCode::F(7) => keys::QUIT,
        KeyCode::F(8) => keys::PRGM,
        KeyCode::F(9) => keys::APPS,
        KeyCode::F(10) => keys::VARS,
        KeyCode::F(11) => keys::MATH,
        KeyCode::F(12) => keys::CATALOG,
        KeyCode::Char(c) => keys::for_char(c)?,
        _ => return None,
    })
}

pub fn run(opts: Options) -> anyhow::Result<()> {
    let mut app = App::new(opts);
    let mut terminal = ratatui::init();
    let result = (|| -> anyhow::Result<()> {
        while !app.quit {
            while let Ok(m) = app.msgs.try_recv() {
                app.handle_msg(m);
            }
            terminal.draw(|f| ui::draw(f, &mut app))?;
            if let Some(k) = &mut app.kitty {
                let want = app
                    .image_area
                    .zip(app.screen.as_ref())
                    .map(|(a, s)| (a, s, app.screen_gen));
                k.sync(&mut std::io::stdout(), want)?;
            }
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => app.on_key(k),
                    _ => {}
                }
            }
        }
        Ok(())
    })();
    if let Some(k) = &mut app.kitty {
        let _ = k.sync(&mut std::io::stdout(), None);
    }
    ratatui::restore();
    result
}
