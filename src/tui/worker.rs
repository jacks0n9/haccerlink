//! Background thread that owns the calculator connection. The UI sends it
//! jobs and gets messages back, so a slow transfer never freezes the screen.

use super::Options;
use haccerlink::calc::{Calc, CalcInfo, CalcVar, Clock, Screenshot};
use haccerlink::dusb::dissect;
use haccerlink::error::Error;
use haccerlink::logging::hex_log_observer;
use haccerlink::tifiles::{self, Piece, TiFile, VarFile};
use haccerlink::transport::Transport;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

type DynCalc = Calc<Box<dyn Transport>>;

pub enum Job {
    Connect,
    Refresh,
    Info,
    Send(Vec<PathBuf>, Option<bool>),
    Get(Vec<CalcVar>, PathBuf),
    Delete(Vec<CalcVar>),
    Rename(CalcVar, String),
    SetArchived(Vec<CalcVar>, bool),
    Screenshot,
    Key(u16, bool),
    Run(CalcVar),
    SyncClock,
}

pub enum Msg {
    Log(String),
    Error(String),
    Packet(String),
    Progress(Option<(usize, usize)>),
    Busy(Option<String>),
    Connected(String, CalcInfo),
    Disconnected(String),
    Vars(Vec<CalcVar>, Option<(u64, u64)>),
    Info(CalcInfo),
    Screen(Screenshot),
    LocalChanged,
}

struct Worker {
    opts: Options,
    tx: Sender<Msg>,
    calc: Option<DynCalc>,
}

pub fn spawn(opts: Options, tx: Sender<Msg>, rx: Receiver<Job>) {
    std::thread::spawn(move || {
        let mut w = Worker {
            opts,
            tx,
            calc: None,
        };
        while let Ok(job) = rx.recv() {
            w.handle(job);
        }
    });
}

/// Errors that mean the link itself is gone (vs. the calculator saying no).
fn is_link_error(e: &Error) -> bool {
    matches!(
        e,
        Error::Usb(_)
            | Error::Timeout
            | Error::Disconnected
            | Error::Io(_)
            | Error::InvalidPacket(_)
    )
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (1..)
        .map(|i| dir.join(format!("{stem}_{i}.{ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

impl Worker {
    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }

    fn log(&self, s: impl Into<String>) {
        self.send(Msg::Log(s.into()));
    }

    fn connect(&mut self) -> Result<(), Error> {
        self.calc = None;
        let (t, desc) = crate::open_transport(
            self.opts.sim,
            self.opts.device.as_deref(),
            self.opts.timeout,
        )
        .map_err(|e| Error::Other(format!("{e:#}")))?;
        let mut calc = Calc::new(t);
        let tx = self.tx.clone();
        let mut first = true;
        calc.link().add_observer(Box::new(move |dir, bytes| {
            let _ = tx.send(Msg::Packet(dissect::summarize(dir, bytes, &mut first)));
        }));
        if let Some(p) = &self.opts.log_hex {
            calc.link().add_observer(hex_log_observer(p)?);
        }
        let tx = self.tx.clone();
        let mut last = 0usize;
        calc.link().set_progress(Some(Box::new(move |done, total| {
            // Don't flood the UI: about 100 updates per transfer.
            if done == total || done.saturating_sub(last) * 100 >= total.max(1) {
                last = done;
                let _ = tx.send(Msg::Progress(Some((done, total))));
            }
        })));
        calc.is_ready()?;
        let info = calc.get_version()?;
        self.calc = Some(calc);
        self.send(Msg::Connected(desc, info));
        self.refresh()
    }

    fn calc(&mut self) -> Result<&mut DynCalc, Error> {
        if self.calc.is_none() {
            self.connect()?;
        }
        Ok(self.calc.as_mut().unwrap())
    }

    fn refresh(&mut self) -> Result<(), Error> {
        let calc = self.calc()?;
        let mut vars = calc.list()?;
        let mem = calc.get_memfree().ok();
        vars.sort_by(|a, b| {
            (a.is_app(), a.type_id, &a.name).cmp(&(b.is_app(), b.type_id, &b.name))
        });
        self.send(Msg::Vars(vars, mem));
        Ok(())
    }

    fn handle(&mut self, job: Job) {
        let label = match &job {
            Job::Connect => "connecting".to_string(),
            Job::Refresh => "listing variables".into(),
            Job::Info => "reading calculator info".into(),
            Job::Send(f, _) => format!("sending {} file(s)", f.len()),
            Job::Get(v, _) => format!("receiving {} variable(s)", v.len()),
            Job::Delete(v) => format!("deleting {} variable(s)", v.len()),
            Job::Rename(v, n) => format!("renaming {} to {n}", v.name),
            Job::SetArchived(v, a) => {
                format!(
                    "moving {} variable(s) to {}",
                    v.len(),
                    if *a { "archive" } else { "RAM" }
                )
            }
            Job::Screenshot => "taking screenshot".into(),
            Job::Key(..) => "pressing key".into(),
            Job::Run(v) => format!("running {}", v.name),
            Job::SyncClock => "setting clock".into(),
        };
        self.send(Msg::Busy(Some(label)));
        let result = self.run_job(job);
        self.send(Msg::Progress(None));
        self.send(Msg::Busy(None));
        if let Err(e) = result {
            if is_link_error(&e) || self.calc.is_none() {
                self.calc = None;
                self.send(Msg::Disconnected(e.to_string()));
            } else {
                self.send(Msg::Error(e.to_string()));
            }
        }
    }

    fn run_job(&mut self, job: Job) -> Result<(), Error> {
        match job {
            Job::Connect => self.connect(),
            Job::Refresh => self.refresh(),
            Job::Info => {
                let info = self.calc()?.get_version()?;
                self.send(Msg::Info(info));
                Ok(())
            }
            Job::Send(files, archive) => {
                let (mut sent, mut failed, mut skipped_os) = (0, 0, 0);
                for path in files {
                    let file = match TiFile::load(&path) {
                        Ok(f) => f,
                        Err(e) => {
                            self.send(Msg::Error(format!("{}: {e}", path.display())));
                            failed += 1;
                            continue;
                        }
                    };
                    let (pieces, warnings) = file.pieces();
                    for w in warnings {
                        self.log(format!("warning: {}: {w}", path.display()));
                    }
                    for p in &pieces {
                        let what = tifiles::describe(p);
                        let r = match p {
                            Piece::Var(e) => {
                                self.log(format!("sending {what}"));
                                self.calc()?.send_var(e, archive)
                            }
                            Piece::App(f) => {
                                self.log(format!("sending {what}"));
                                self.calc()?.send_app(f)
                            }
                            Piece::Os(_) => {
                                self.log(format!("skipping {what}: OS transfer isn't supported"));
                                skipped_os += 1;
                                continue;
                            }
                        };
                        match r {
                            Ok(()) => sent += 1,
                            // The calculator said no to this one; keep going.
                            Err(e) if !is_link_error(&e) => {
                                self.send(Msg::Error(format!("{what}: {e}")));
                                failed += 1;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
                let mut summary = format!("sent {sent}");
                if failed > 0 {
                    summary += &format!(", {failed} failed");
                }
                if skipped_os > 0 {
                    summary += &format!(
                        ", {skipped_os} OS file(s) skipped (use TI Connect CE for OS upgrades)"
                    );
                }
                self.log(summary);
                self.refresh()
            }
            Job::Get(vars, dir) => {
                for v in vars {
                    let bytes = if v.is_app() {
                        self.calc()?.recv_app(&v)?.to_bytes()
                    } else {
                        let e = self.calc()?.recv_var(&v)?;
                        VarFile::single(e, tifiles::single_file_comment(chrono::Local::now()))
                            .to_bytes()
                    };
                    let path = unique_path(&dir, &tifiles::file_name_for(&v.name, v.type_id));
                    std::fs::write(&path, bytes)?;
                    self.log(format!("saved {}", path.display()));
                }
                self.send(Msg::LocalChanged);
                Ok(())
            }
            Job::Delete(vars) => {
                for v in vars {
                    self.calc()?.delete(&v)?;
                    self.log(format!("deleted {}", v.name));
                }
                self.refresh()
            }
            Job::Rename(v, new) => {
                self.calc()?.rename(&v, &new)?;
                self.log(format!("renamed {} to {new}", v.name));
                self.refresh()
            }
            Job::SetArchived(vars, archived) => {
                for v in vars {
                    self.calc()?.set_archived(&v, archived)?;
                }
                self.refresh()
            }
            Job::Screenshot => {
                let s = self.calc()?.screenshot()?;
                self.send(Msg::Screen(s));
                Ok(())
            }
            Job::Key(code, then_screen) => {
                self.calc()?.send_key(code)?;
                if then_screen {
                    let s = self.calc()?.screenshot()?;
                    self.send(Msg::Screen(s));
                }
                Ok(())
            }
            Job::Run(v) => {
                self.calc()?.execute(&v)?;
                self.log(format!("started {}", v.name));
                Ok(())
            }
            Job::SyncClock => {
                use chrono::{Datelike, Timelike};
                let calc = self.calc()?;
                let old = calc.get_clock().ok();
                let now = chrono::Local::now();
                calc.set_clock(&Clock {
                    year: now.year() as u16,
                    month: now.month() as u8,
                    day: now.day() as u8,
                    hours: now.hour() as u8,
                    minutes: now.minute() as u8,
                    seconds: now.second() as u8,
                    date_format: old.map_or(1, |c| c.date_format),
                    time_format: old.map_or(12, |c| c.time_format),
                    on: true,
                })?;
                self.log(format!("clock set to {}", now.format("%Y-%m-%d %H:%M:%S")));
                Ok(())
            }
        }
    }
}
