//! Background thread that owns the calculator connection. The UI sends it
//! jobs and gets messages back, so a slow transfer never freezes the screen.

use super::Options;
use haccerlink::calc::{Calc, CalcInfo, CalcVar, Clock, Screenshot};
use haccerlink::dusb::dissect;
use haccerlink::error::Error;
use haccerlink::logging::hex_log_observer;
use haccerlink::tifiles::{self, Piece, TiFile, VarFile};
use haccerlink::transport::{Transport, UsbDeviceInfo};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

type DynCalc = Calc<Box<dyn Transport>>;

/// What a send job sends: whole files, or items picked out of a bundle.
pub enum SendWhat {
    Files(Vec<PathBuf>),
    Pieces(Vec<Piece>),
}

impl SendWhat {
    pub fn describe(&self) -> String {
        match self {
            SendWhat::Files(f) => format!("{} file(s)", f.len()),
            SendWhat::Pieces(p) => format!("{} item(s)", p.len()),
        }
    }
}

pub enum Job {
    Connect,
    Refresh,
    Info,
    /// What to send, archive choice, whether to install an OS among it.
    Send(SendWhat, Option<bool>, bool),
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
    /// The USB device we're connected to (None for the simulator).
    usb: Option<UsbDeviceInfo>,
}

pub fn spawn(opts: Options, tx: Sender<Msg>, rx: Receiver<Job>) {
    std::thread::spawn(move || {
        let mut w = Worker {
            opts,
            tx,
            calc: None,
            usb: None,
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
        let (t, desc, usb) = crate::open_transport(
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
        match calc.is_ready() {
            Ok(()) => {}
            // A CE without an OS (e.g. after an interrupted OS transfer)
            // refuses normal mode but still takes an OS transfer: stay
            // connected so one can be sent.
            Err(Error::Calc(code)) => {
                self.calc = Some(calc);
                self.usb = usb;
                self.send(Msg::Connected(desc, CalcInfo::default()));
                self.send(Msg::Error(format!(
                    "calculator refused normal mode (error {code}); it may have no OS. \
                     To reinstall one, send an .8eu file"
                )));
                return Ok(());
            }
            Err(e) => return Err(e),
        }
        let info = calc.get_version()?;
        self.calc = Some(calc);
        self.usb = usb;
        self.send(Msg::Connected(desc, info));
        self.refresh()
    }

    /// Send an OS, then follow the calculator through its restart.
    fn install_os(&mut self, os: &tifiles::FlashFile) -> Result<(), Error> {
        let what = tifiles::describe(&Piece::Os(os.clone()));
        self.log(format!("installing {what}; don't unplug the calculator"));
        self.send(Msg::Busy(Some(format!("installing {what}"))));
        if let Err(e) = self.calc()?.send_os(os) {
            self.send(Msg::Error(
                "OS transfer failed. If the calculator shows the boot code's OS-transfer \
                 screen, send the OS again"
                    .into(),
            ));
            return Err(e);
        }
        self.log("OS sent");
        let Some(old) = self.usb.clone() else {
            return Ok(()); // the simulator doesn't restart
        };
        self.send(Msg::Busy(Some(
            "waiting for the calculator to restart".into(),
        )));
        let tx = self.tx.clone();
        let calc = self.calc.as_mut().expect("connected above");
        let new = match crate::reconnect_after_os(calc, &old, self.opts.timeout, |s| {
            let _ = tx.send(Msg::Log(s.to_string()));
        }) {
            Ok(new) => new,
            Err(e) => {
                // Its transport is dead; show the calculator as disconnected.
                self.calc = None;
                return Err(Error::Other(format!("{e:#}")));
            }
        };
        // Later reconnects should find it at its new address.
        if self.opts.device.is_some() {
            self.opts.device = Some(format!("{}:{}", new.bus, new.address));
        }
        let info = self.calc()?.get_version()?;
        self.send(Msg::Connected(new.to_string(), info));
        self.usb = Some(new);
        Ok(())
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
            Job::Send(what, _, _) => format!("sending {}", what.describe()),
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
            Job::Send(what, archive, install_os) => {
                let (mut sent, mut failed) = (0, 0);
                let mut pieces = Vec::new();
                let files = match what {
                    SendWhat::Files(f) => f,
                    SendWhat::Pieces(p) => {
                        pieces = p;
                        Vec::new()
                    }
                };
                for path in files {
                    match TiFile::load(&path) {
                        Ok(file) => {
                            let (p, warnings) = file.pieces();
                            for w in warnings {
                                self.log(format!("warning: {}: {w}", path.display()));
                            }
                            pieces.extend(p);
                        }
                        Err(e) => {
                            self.send(Msg::Error(format!("{}: {e}", path.display())));
                            failed += 1;
                        }
                    }
                }
                let (os, pieces) = match tifiles::split_os(pieces) {
                    Ok(x) => x,
                    Err(e) => {
                        self.send(Msg::Error(e.to_string()));
                        return Ok(());
                    }
                };
                let mut skipped_os = false;
                if let Some(os) = os {
                    if install_os {
                        self.install_os(&os)?;
                        sent += 1;
                    } else {
                        skipped_os = true;
                    }
                }
                for p in &pieces {
                    let what = tifiles::describe(p);
                    self.log(format!("sending {what}"));
                    let r = match p {
                        Piece::Var(e) => self.calc()?.send_var(e, archive),
                        Piece::App(f) => self.calc()?.send_app(f),
                        Piece::Os(_) => unreachable!("split_os took it out"),
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
                let mut summary = format!("sent {sent}");
                if failed > 0 {
                    summary += &format!(", {failed} failed");
                }
                if skipped_os {
                    summary += ", OS skipped";
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
