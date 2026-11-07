use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use haccerlink::calc::{Calc, CalcVar, Clock};
use haccerlink::dusb::dissect::Dissector;
use haccerlink::logging::hex_log_observer;
use haccerlink::names;
use haccerlink::sim::{self, SimState};
use haccerlink::tifiles::{self, Piece, TiFile, VarFile};
use haccerlink::transport::{self, StdioTransport, Transport};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod tui;

#[derive(Parser)]
#[command(version, about = "Link program for the TI-84 Plus CE (DUSB over USB)")]
struct Cli {
    /// Talk to a simulated calculator instead of real hardware.
    #[arg(long, global = true)]
    sim: bool,
    /// Use the calculator at BUS:ADDRESS (see `haccerlink devices`).
    #[arg(long, global = true, value_name = "BUS:ADDR")]
    device: Option<String>,
    /// Write a TiLP-format hex log (same format as ~/.ticables/ticables-log.hex).
    #[arg(long, global = true, value_name = "FILE")]
    log_hex: Option<PathBuf>,
    /// Write a libticalcs-style packet dissection log.
    #[arg(long, global = true, value_name = "FILE")]
    dissect: Option<PathBuf>,
    /// How the TUI draws screenshots.
    #[arg(long, global = true, value_enum, default_value_t = tui::kitty::ImagePref::Auto)]
    images: tui::kitty::ImagePref,
    /// USB timeout in seconds.
    #[arg(long, global = true, default_value_t = 5.0)]
    timeout: f64,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive terminal UI (the default).
    Tui,
    /// List connected calculators.
    Devices,
    /// Show calculator version and memory information.
    Info,
    /// List variables and apps on the calculator.
    Ls,
    /// Send .8x?/.8ek files to the calculator.
    Send {
        files: Vec<PathBuf>,
        /// Store variables in the archive.
        #[arg(long, conflicts_with = "ram")]
        archive: bool,
        /// Store variables in RAM.
        #[arg(long)]
        ram: bool,
    },
    /// Receive variables (or apps) from the calculator.
    Get {
        /// Variable names; omit with --all to get everything.
        names: Vec<String>,
        /// Receive every variable.
        #[arg(long)]
        all: bool,
        /// Only match this type (e.g. PRGM, APPV, or a hex id like 05).
        #[arg(short = 't', long)]
        r#type: Option<String>,
        /// Output directory (or file name when getting a single variable).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Delete variables.
    Rm {
        names: Vec<String>,
        #[arg(short = 't', long)]
        r#type: Option<String>,
    },
    /// Rename a variable.
    Mv {
        from: String,
        to: String,
        #[arg(short = 't', long)]
        r#type: Option<String>,
    },
    /// Move variables to the archive.
    Archive {
        names: Vec<String>,
        #[arg(short = 't', long)]
        r#type: Option<String>,
    },
    /// Move variables to RAM.
    Unarchive {
        names: Vec<String>,
        #[arg(short = 't', long)]
        r#type: Option<String>,
    },
    /// Save a screenshot as PNG.
    Screenshot {
        #[arg(short, long, default_value = "screenshot.png")]
        output: PathBuf,
    },
    /// Press keys remotely (names like enter/clear/up, characters, or 0xNN codes).
    Key { keys: Vec<String> },
    /// Run a program.
    Run { name: String },
    /// Show the calculator's clock, or set it.
    Clock {
        /// Set the clock to this machine's local time.
        #[arg(long)]
        set_now: bool,
    },
    /// Serve a simulated calculator on stdin/stdout (used by the libticalcs oracle).
    SimServe,
    /// Parse a TI file and show what's inside.
    Inspect { file: PathBuf },
}

type DynCalc = Calc<Box<dyn Transport>>;

pub fn open_transport(
    sim: bool,
    device: Option<&str>,
    timeout: Duration,
) -> anyhow::Result<(Box<dyn Transport>, String)> {
    let mut t: Box<dyn Transport>;
    let desc;
    if sim {
        t = Box::new(sim::spawn(Arc::new(Mutex::new(SimState::default()))));
        desc = "simulated TI-84 Plus CE".to_string();
    } else {
        let select = match device {
            Some(d) => {
                let (bus, addr) = d
                    .split_once(':')
                    .context("--device must look like BUS:ADDR")?;
                Some((
                    bus.to_string(),
                    addr.parse::<u8>().context("bad device address")?,
                ))
            }
            None => None,
        };
        let usb = transport::UsbTransport::open(select.as_ref().map(|(b, a)| (b.as_str(), *a)))?;
        desc = usb.info.to_string();
        t = Box::new(usb);
    }
    t.set_timeout(timeout);
    Ok((t, desc))
}

fn connect(cli: &Cli) -> anyhow::Result<DynCalc> {
    let (t, desc) = open_transport(
        cli.sim,
        cli.device.as_deref(),
        Duration::from_secs_f64(cli.timeout),
    )?;
    eprintln!("using {desc}");
    let mut calc = Calc::new(t);
    attach_logs(&mut calc, cli)?;
    calc.is_ready()
        .context("calculator did not answer the ready check")?;
    Ok(calc)
}

fn attach_logs(calc: &mut DynCalc, cli: &Cli) -> anyhow::Result<()> {
    if let Some(p) = &cli.log_hex {
        calc.link().add_observer(hex_log_observer(p)?);
    }
    if let Some(p) = &cli.dissect {
        let mut f = std::fs::File::create(p)?;
        let mut d = Dissector::new();
        calc.link().add_observer(Box::new(move |dir, bytes| {
            if let Some(s) = d.dissect(dir, bytes) {
                let _ = f.write_all(s.as_bytes());
            }
        }));
    }
    Ok(())
}

fn parse_type(t: &str) -> anyhow::Result<u8> {
    let upper = t.to_ascii_uppercase();
    if let Some(id) = (0..=255u8).find(|&id| names::type_name(id) == upper) {
        return Ok(id);
    }
    u8::from_str_radix(t.trim_start_matches("0x"), 16).with_context(|| format!("unknown type {t}"))
}

fn find_vars(
    calc: &mut DynCalc,
    names: &[String],
    ty: Option<&str>,
) -> anyhow::Result<Vec<CalcVar>> {
    let ty = ty.map(parse_type).transpose()?;
    let list = calc.list()?;
    let mut out = Vec::new();
    for n in names {
        let matches: Vec<_> = list
            .iter()
            .filter(|v| &v.name == n && ty.is_none_or(|t| t == v.type_id))
            .collect();
        match matches.len() {
            0 => bail!("no variable named {n} on the calculator"),
            1 => out.push(matches[0].clone()),
            _ => bail!("{n} exists with several types; pick one with -t"),
        }
    }
    Ok(out)
}

fn print_list(vars: &[CalcVar]) {
    println!("{:<10} {:<6} {:>7}  WHERE", "NAME", "TYPE", "SIZE");
    for v in vars {
        println!(
            "{:<10} {:<6} {:>7}  {}",
            v.name,
            names::type_name(v.type_id),
            v.size,
            if v.archived { "archive" } else { "RAM" }
        );
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command.as_ref().unwrap_or(&Command::Tui) {
        Command::Tui => tui::run(tui::Options {
            sim: cli.sim,
            device: cli.device.clone(),
            timeout: Duration::from_secs_f64(cli.timeout),
            log_hex: cli.log_hex.clone(),
            images: cli.images,
        })?,
        Command::Devices => {
            let devs = transport::list_calculators()?;
            if devs.is_empty() {
                println!("no calculators found");
            }
            for d in devs {
                println!("{}:{}  {d}", d.bus, d.address);
            }
        }
        Command::Info => {
            let mut calc = connect(&cli)?;
            for (k, v) in calc.get_version()?.lines() {
                println!("{k:<17} {v}");
            }
        }
        Command::Ls => {
            let mut calc = connect(&cli)?;
            print_list(&calc.list()?);
        }
        Command::Send {
            files,
            archive,
            ram,
        } => {
            let loc = if *archive {
                Some(true)
            } else if *ram {
                Some(false)
            } else {
                None
            };
            let parsed = files
                .iter()
                .map(|f| Ok((f, TiFile::load(f).with_context(|| f.display().to_string())?)))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let mut calc = connect(&cli)?;
            let mut refused = 0;
            // The calculator refusing one variable (e.g. it exists in the
            // archive) shouldn't stop the rest; link failures still do.
            let mut check = |name: &str, r: haccerlink::Result<()>| -> anyhow::Result<()> {
                match r {
                    Err(haccerlink::Error::Calc(code)) => {
                        eprintln!("  {name} refused by the calculator: error {code}");
                        refused += 1;
                        Ok(())
                    }
                    other => Ok(other?),
                }
            };
            let mut skipped_os = 0;
            for (path, file) in parsed {
                let (pieces, warnings) = file.pieces();
                for w in warnings {
                    eprintln!("warning: {}: {w}", path.display());
                }
                for p in &pieces {
                    match p {
                        Piece::Var(e) => {
                            eprintln!("sending {}", tifiles::describe(p));
                            check(&e.wire_name(), calc.send_var(e, loc))?;
                        }
                        Piece::App(f) => {
                            eprintln!("sending {}", tifiles::describe(p));
                            check(&f.name, calc.send_app(f))?;
                        }
                        Piece::Os(_) => {
                            eprintln!(
                                "skipping {}: OS transfer isn't supported",
                                tifiles::describe(p)
                            );
                            skipped_os += 1;
                        }
                    }
                }
            }
            if skipped_os > 0 {
                eprintln!(
                    "note: {skipped_os} OS file(s) skipped; use TI Connect CE to upgrade the OS"
                );
            }
            if refused > 0 {
                bail!("{refused} variable(s) were refused");
            }
        }
        Command::Get {
            names: wanted,
            all,
            r#type,
            output,
        } => {
            let mut calc = connect(&cli)?;
            let vars = if *all {
                let ty = r#type.as_deref().map(parse_type).transpose()?;
                calc.list()?
                    .into_iter()
                    .filter(|v| ty.is_none_or(|t| t == v.type_id))
                    .collect()
            } else {
                find_vars(&mut calc, wanted, r#type.as_deref())?
            };
            let single_file =
                !*all && vars.len() == 1 && output.as_ref().is_some_and(|o| !o.is_dir());
            for v in &vars {
                let fname = tifiles::file_name_for(&v.name, v.type_id);
                let path = match output {
                    Some(o) if single_file => o.clone(),
                    Some(o) => {
                        std::fs::create_dir_all(o)?;
                        o.join(&fname)
                    }
                    None => PathBuf::from(&fname),
                };
                let bytes = if v.is_app() {
                    calc.recv_app(v)?.to_bytes()
                } else {
                    let e = calc.recv_var(v)?;
                    VarFile::single(e, tifiles::single_file_comment(chrono::Local::now()))
                        .to_bytes()
                };
                std::fs::write(&path, bytes)?;
                eprintln!("saved {}", path.display());
            }
        }
        Command::Rm { names: n, r#type } => {
            let mut calc = connect(&cli)?;
            for v in find_vars(&mut calc, n, r#type.as_deref())? {
                calc.delete(&v)?;
                eprintln!("deleted {}", v.name);
            }
        }
        Command::Mv { from, to, r#type } => {
            let mut calc = connect(&cli)?;
            let v = find_vars(&mut calc, std::slice::from_ref(from), r#type.as_deref())?;
            calc.rename(&v[0], to)?;
        }
        Command::Archive { names: n, r#type } | Command::Unarchive { names: n, r#type } => {
            let to_archive = matches!(cli.command, Some(Command::Archive { .. }));
            let mut calc = connect(&cli)?;
            for v in find_vars(&mut calc, n, r#type.as_deref())? {
                calc.set_archived(&v, to_archive)?;
            }
        }
        Command::Screenshot { output } => {
            let mut calc = connect(&cli)?;
            calc.screenshot()?.write_png(output)?;
            eprintln!("saved {}", output.display());
        }
        Command::Key { keys } => {
            let codes = keys
                .iter()
                .map(|k| haccerlink::keys::by_name(k).with_context(|| format!("unknown key {k}")))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let mut calc = connect(&cli)?;
            for c in codes {
                calc.send_key(c)?;
            }
        }
        Command::Run { name } => {
            let mut calc = connect(&cli)?;
            let v = find_vars(&mut calc, std::slice::from_ref(name), None)?;
            calc.execute(&v[0])?;
        }
        Command::Clock { set_now } => {
            let mut calc = connect(&cli)?;
            if *set_now {
                use chrono::{Datelike, Timelike};
                let now = chrono::Local::now();
                let old = calc.get_clock().ok();
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
            }
            let c = calc.get_clock()?;
            println!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02} ({}h, date format {}, clock {})",
                c.year,
                c.month,
                c.day,
                c.hours,
                c.minutes,
                c.seconds,
                c.time_format,
                c.date_format,
                if c.on { "on" } else { "off" }
            );
        }
        Command::SimServe => {
            let state = Arc::new(Mutex::new(SimState::default()));
            sim::Simulator::new(StdioTransport::new(), state).run()?;
        }
        Command::Inspect { file } => print_inspect(&TiFile::load(file)?, ""),
    }
    Ok(())
}

fn print_inspect(file: &TiFile, indent: &str) {
    match file {
        TiFile::Vars(v) => {
            let comment: String = v
                .comment
                .iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as char)
                .collect();
            println!(
                "{indent}variable file, comment {comment:?}, checksum {}",
                if v.checksum_ok { "ok" } else { "BAD" }
            );
            for e in &v.entries {
                println!(
                    "{indent}  {:<10} {:<6} {:>6} bytes  version {}  {}  raw name {:02X?}",
                    e.wire_name(),
                    names::type_name(e.type_id),
                    e.data.len(),
                    e.version,
                    if e.archived { "archived" } else { "RAM" },
                    e.raw_name
                );
            }
        }
        TiFile::Flash(f) => println!(
            "{indent}flash file: {} type {:02X} ({}), {} bytes, hw id {:02X}",
            f.name,
            f.data_type,
            names::type_name(f.data_type),
            f.data.len(),
            f.hw_id
        ),
        TiFile::Bundle(b) => {
            let check = match b.checksum_ok {
                Some(true) => "ok",
                Some(false) => "BAD",
                None => "none",
            };
            println!(
                "{indent}bundle for {}, {} files, checksum {check}",
                b.target_device().unwrap_or("?"),
                b.files.len()
            );
            for (name, f) in &b.files {
                println!("{indent}  {name}:");
                print_inspect(f, &format!("{indent}    "));
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
