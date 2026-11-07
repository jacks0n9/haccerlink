//! Byte-for-byte comparison against libticalcs2 (what TiLP uses).
//!
//! `tests/golden/*.txt` are transcripts recorded by `oracle/build/oracle`,
//! which runs the real libticalcs2 against `haccerlink sim-serve`. Each test
//! case runs the same operation through haccerlink against the same simulator
//! and requires the exact same packets in both directions.
//!
//! Regenerate goldens (needs the oracle, see oracle/README):
//!   nix develop .#oracle -c ./oracle/build.sh
//!   cargo test --test oracle -- --ignored regenerate_goldens

use haccerlink::calc::{Calc, CalcVar, Clock};
use haccerlink::dusb::Dir;
use haccerlink::dusb::dissect::Dissector;
use haccerlink::logging::Transcript;
use haccerlink::names::types;
use haccerlink::sim::{self, SimState};
use haccerlink::tifiles::{FlashFile, VarEntry, VarFile};
use haccerlink::transport::ChannelTransport;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type C = Calc<ChannelTransport>;

fn var(name: &str, type_id: u8) -> CalcVar {
    CalcVar {
        name: name.into(),
        type_id,
        size: 0,
        archived: false,
    }
}

fn program(name: &str, body_len: usize) -> VarEntry {
    // Token data: length prefix, then `:` tokens, padded.
    let mut data = (body_len as u16).to_le_bytes().to_vec();
    data.extend((0..body_len).map(|i| {
        if i % 2 == 0 {
            0x3E
        } else {
            b'A' + (i % 26) as u8
        }
    }));
    VarEntry {
        type_id: types::PROGRAM,
        raw_name: name.as_bytes().to_vec(),
        version: 0,
        archived: false,
        data,
    }
}

/// Files the send cases use. Sizes are picked to hit the interesting
/// fragmentation cases with a 1018-byte raw packet limit.
fn write_fixtures(dir: &Path) {
    let w = |name: &str, f: VarFile| std::fs::write(dir.join(name), f.to_bytes()).unwrap();
    // 53 data bytes: single raw packet of 59 bytes + 5 header = 64 -> ZLP.
    w("SMALL.8xp", VarFile::single(program("SMALL", 51), [0; 42]));
    w("BIG.8xp", VarFile::single(program("BIG", 3000), [0; 42]));
    // Last raw packet of 59 bytes -> ZLP after a multi-packet transfer.
    w(
        "ZLPBIG.8xp",
        VarFile::single(program("ZLPBIG", 1012 + 1018 + 59 - 2), [0; 42]),
    );
    // Exactly fills the first + one continuation packet: empty final packet.
    w(
        "EXACT.8xp",
        VarFile::single(program("EXACT", 1012 + 1018 - 2), [0; 42]),
    );
    let list = VarEntry {
        type_id: types::LIST,
        raw_name: vec![0x5D, 0x01],
        version: 0,
        archived: false,
        data: vec![0x01, 0x00, 0x00, 0x80, 0x30, 0, 0, 0, 0, 0, 0],
    };
    w("L2.8xl", VarFile::single(list.clone(), [0; 42]));
    let mut arch = program("ARCHD", 10);
    arch.archived = true;
    w(
        "GROUP.8xg",
        VarFile {
            comment: [0; 42],
            entries: vec![program("GRPA", 4), arch],
            checksum_ok: true,
        },
    );
    let mut app = vec![
        0x81, 0x0F, 0x00, 0x00, 0x08, 0x00, 0x81, 0x45, b'H', b'E', b'L', b'L', b'O',
    ];
    app.resize(0x800 + 6, 0xA5);
    let mut f = FlashFile::from_received("HELLO", types::APP, app);
    f.revision_major = 1;
    std::fs::write(dir.join("HELLO.8ek"), f.to_bytes()).unwrap();
    // A non-ASCII app name, like TI's Español app ("Espa" B5 "ol").
    let mut app = vec![0x81, 0x0F, 0x00, 0x00, 0x01, 0x00, 0x81, 0x47];
    app.extend_from_slice(b"Espa\xB5ol");
    app.resize(0x100 + 6, 0x5A);
    let f = FlashFile::from_received("x", types::APP, app);
    std::fs::write(dir.join("ESPANOL.8ek"), f.to_bytes()).unwrap();
    let chunk = (sim::SIM_OS_CHUNK - 4) as usize;
    // Three full OS Data chunks and a partial one...
    std::fs::write(dir.join("OS.8eu"), fake_os(3 * chunk + 100).to_bytes()).unwrap();
    // ...and exactly three, after which libticalcs sends an empty chunk.
    std::fs::write(dir.join("OSEXACT.8eu"), fake_os(3 * chunk).to_bytes()).unwrap();
}

/// A stand-in OS image laid out like TI's: 0x80 0x0F header fields up to
/// the first FF FF FF FF, then the code.
fn fake_os(len: usize) -> FlashFile {
    let mut d = vec![0x80, 0x0F, 0x00, 0x00, 0x01, 0x00];
    d.extend((0..250u32).map(|i| (i * 7 % 251) as u8));
    d.extend_from_slice(&[0xFF; 4]);
    d.extend((0..).map(|i: u32| (i * 13 % 256) as u8).take(len - d.len()));
    let mut f = FlashFile::from_received("basecode", types::OS, d);
    f.revision_major = 5;
    f.revision_minor = 8;
    f
}

fn load_flash(dir: &Path, f: &str) -> FlashFile {
    FlashFile::parse(&std::fs::read(dir.join(f)).unwrap()).unwrap()
}

struct Case {
    name: &'static str,
    oracle: &'static [&'static str],
    run: fn(&mut C, &Path),
    /// File both sides write, compared after the transcript.
    output: Option<&'static str>,
}

/// Places where we intentionally don't copy TiLP: a fix applied to its
/// saved file before comparing. Everything else must still match.
fn known_divergence(case: &str, tilp_file: Vec<u8>) -> Vec<u8> {
    match case {
        // libticonv tokenizes a received "L₁" as a *named* list "L₁"
        // (5D 4C 81) instead of the real L₁ (5D 00), so TiLP writes a file
        // that would create a bogus custom list when sent back.
        "get_list" => {
            let mut f = VarFile::parse(&tilp_file).unwrap();
            assert_eq!(
                f.entries[0].raw_name,
                [0x5D, 0x4C, 0x81],
                "TiLP's L₁ bug changed?"
            );
            f.entries[0].raw_name = vec![0x5D, 0x00];
            normalize_output(f.to_bytes())
        }
        // Same libticonv problem for matrices: '[' is mapped to 0xC1 before
        // the tokenizer looks for it, so "[A]" is saved as C1 41 5D, not 5C 00.
        "get_matrix" => {
            let mut f = VarFile::parse(&tilp_file).unwrap();
            assert_eq!(
                f.entries[0].raw_name,
                [0xC1, b'A', b']'],
                "TiLP's [A] bug changed?"
            );
            f.entries[0].raw_name = vec![0x5C, 0x00];
            normalize_output(f.to_bytes())
        }
        _ => tilp_file,
    }
}

fn load_vars(dir: &Path, f: &str) -> VarFile {
    VarFile::parse(&std::fs::read(dir.join(f)).unwrap()).unwrap()
}

fn save_var(c: &mut C, dir: &Path, v: CalcVar, file: &str) {
    let e = c.recv_var(&v).unwrap();
    std::fs::write(dir.join(file), VarFile::single(e, [0; 42]).to_bytes()).unwrap();
}

const CASES: &[Case] = &[
    Case {
        name: "ready",
        oracle: &["ready"],
        run: |_, _| {},
        output: None,
    },
    Case {
        name: "info",
        oracle: &["info"],
        run: |c, _| drop(c.get_version().unwrap()),
        output: None,
    },
    Case {
        name: "memfree",
        oracle: &["memfree"],
        run: |c, _| {
            c.get_memfree().unwrap();
        },
        output: None,
    },
    Case {
        name: "ls",
        oracle: &["ls"],
        run: |c, _| drop(c.list().unwrap()),
        output: None,
    },
    Case {
        name: "send_small",
        oracle: &["send", "$/SMALL.8xp"],
        run: |c, d| c.send_file(&load_vars(d, "SMALL.8xp"), None).unwrap(),
        output: None,
    },
    Case {
        name: "send_big",
        oracle: &["send", "$/BIG.8xp"],
        run: |c, d| c.send_file(&load_vars(d, "BIG.8xp"), None).unwrap(),
        output: None,
    },
    Case {
        name: "send_zlp_multi",
        oracle: &["send", "$/ZLPBIG.8xp"],
        run: |c, d| c.send_file(&load_vars(d, "ZLPBIG.8xp"), None).unwrap(),
        output: None,
    },
    Case {
        name: "send_exact",
        oracle: &["send", "$/EXACT.8xp"],
        run: |c, d| c.send_file(&load_vars(d, "EXACT.8xp"), None).unwrap(),
        output: None,
    },
    Case {
        name: "send_list",
        oracle: &["send", "$/L2.8xl"],
        run: |c, d| c.send_file(&load_vars(d, "L2.8xl"), None).unwrap(),
        output: None,
    },
    Case {
        name: "send_group",
        oracle: &["send", "$/GROUP.8xg"],
        run: |c, d| c.send_file(&load_vars(d, "GROUP.8xg"), None).unwrap(),
        output: None,
    },
    Case {
        name: "send_app",
        oracle: &["sendapp", "$/HELLO.8ek"],
        run: |c, d| {
            let f = FlashFile::parse(&std::fs::read(d.join("HELLO.8ek")).unwrap()).unwrap();
            c.send_app(&f).unwrap()
        },
        output: None,
    },
    Case {
        name: "send_app_accented",
        oracle: &["sendapp", "$/ESPANOL.8ek"],
        run: |c, d| c.send_app(&load_flash(d, "ESPANOL.8ek")).unwrap(),
        output: None,
    },
    Case {
        name: "send_os",
        oracle: &["sendos", "$/OS.8eu"],
        run: |c, d| c.send_os(&load_flash(d, "OS.8eu")).unwrap(),
        output: None,
    },
    Case {
        name: "send_os_exact",
        oracle: &["sendos", "$/OSEXACT.8eu"],
        run: |c, d| c.send_os(&load_flash(d, "OSEXACT.8eu")).unwrap(),
        output: None,
    },
    Case {
        name: "get_program",
        oracle: &["get", "HELLO", "05", "$/out.8xp"],
        run: |c, d| save_var(c, d, var("HELLO", 5), "out.8xp"),
        output: Some("out.8xp"),
    },
    Case {
        name: "get_list",
        oracle: &["get", "L₁", "01", "$/out.8xl"],
        run: |c, d| save_var(c, d, var("L₁", 1), "out.8xl"),
        output: Some("out.8xl"),
    },
    Case {
        name: "get_string",
        oracle: &["get", "Str1", "04", "$/out.8xs"],
        run: |c, d| save_var(c, d, var("Str1", 4), "out.8xs"),
        output: Some("out.8xs"),
    },
    Case {
        name: "get_matrix",
        oracle: &["get", "[A]", "02", "$/out.8xm"],
        run: |c, d| save_var(c, d, var("[A]", 2), "out.8xm"),
        output: Some("out.8xm"),
    },
    Case {
        name: "get_archived_appvar",
        oracle: &["get", "SAVEDAT", "15", "$/out.8xv"],
        run: |c, d| save_var(c, d, var("SAVEDAT", 0x15), "out.8xv"),
        output: Some("out.8xv"),
    },
    Case {
        name: "get_app",
        oracle: &["getapp", "DEMO", "$/out.8ek"],
        run: |c, d| {
            let f = c.recv_app(&var("DEMO", 0x24)).unwrap();
            std::fs::write(d.join("out.8ek"), f.to_bytes()).unwrap()
        },
        output: Some("out.8ek"),
    },
    Case {
        name: "delete",
        oracle: &["rm", "A", "00"],
        run: |c, _| c.delete(&var("A", 0)).unwrap(),
        output: None,
    },
    Case {
        name: "rename",
        oracle: &["mv", "HELLO", "05", "HI"],
        run: |c, _| c.rename(&var("HELLO", 5), "HI").unwrap(),
        output: None,
    },
    Case {
        name: "archive",
        oracle: &["archive", "HELLO", "05"],
        run: |c, _| c.set_archived(&var("HELLO", 5), true).unwrap(),
        output: None,
    },
    Case {
        name: "unarchive",
        oracle: &["unarchive", "SAVEDAT", "15"],
        run: |c, _| c.set_archived(&var("SAVEDAT", 0x15), false).unwrap(),
        output: None,
    },
    Case {
        name: "screenshot",
        oracle: &["screenshot"],
        run: |c, _| drop(c.screenshot().unwrap()),
        output: None,
    },
    Case {
        name: "key_clear",
        oracle: &["key", "09"],
        run: |c, _| c.send_key(0x09).unwrap(),
        output: None,
    },
    Case {
        name: "key_two_byte",
        oracle: &["key", "FE0A"],
        run: |c, _| c.send_key(0xFE0A).unwrap(),
        output: None,
    },
    Case {
        name: "run",
        oracle: &["run", "HELLO", "05"],
        run: |c, _| c.execute(&var("HELLO", 5)).unwrap(),
        output: None,
    },
    Case {
        name: "get_clock",
        oracle: &["getclock"],
        run: |c, _| {
            c.get_clock().unwrap();
        },
        output: None,
    },
    Case {
        name: "set_clock",
        oracle: &["setclock", "2026", "11", "6", "13", "14", "15", "1", "24"],
        run: |c, _| {
            c.set_clock(&Clock {
                year: 2026,
                month: 11,
                day: 6,
                hours: 13,
                minutes: 14,
                seconds: 15,
                date_format: 1,
                time_format: 24,
                on: true,
            })
            .unwrap()
        },
        output: None,
    },
];

/// Long calculator->PC packets come from the simulator either way; shorten
/// them so goldens stay small. PC->TI packets are always kept in full.
fn compact(transcript: &str) -> String {
    let mut out = String::new();
    for line in transcript.lines() {
        let n = line.split(' ').count() - 1;
        if line.starts_with("TI>PC") && n > 48 {
            let bytes: Vec<u8> = line
                .split(' ')
                .skip(1)
                .map(|h| u8::from_str_radix(h, 16).unwrap())
                .collect();
            let sum = bytes
                .iter()
                .fold(0u32, |a, &b| a.wrapping_mul(31).wrapping_add(b as u32));
            let head: Vec<String> = line.split(' ').take(17).map(String::from).collect();
            out.push_str(&format!(
                "{} … ({n} bytes, hash {sum:08X})\n",
                head.join(" ")
            ));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Cases whose libticalcs `dusb_dissect()` output is also checked (the big
/// transfers are left out to keep the goldens small).
const DISSECT_CASES: &[&str] = &[
    "ready",
    "info",
    "memfree",
    "ls",
    "send_small",
    "send_zlp_multi",
    "send_exact",
    "send_list",
    "send_app",
    "send_app_accented",
    "send_os",
    "send_os_exact",
    "get_program",
    "get_matrix",
    "get_app",
    "delete",
    "rename",
    "archive",
    "unarchive",
    "key_clear",
    "key_two_byte",
    "run",
    "get_clock",
    "set_clock",
];

fn dissect_all(pkts: &[(Dir, Vec<u8>)]) -> String {
    let mut d = Dissector::new();
    pkts.iter()
        .filter_map(|(dir, b)| d.dissect(*dir, b))
        .collect()
}

fn run_rust(case: &Case, dir: &Path) -> (String, String) {
    let state = Arc::new(Mutex::new(SimState::default()));
    let mut calc = Calc::new(sim::spawn(state));
    let t = Transcript::new();
    calc.link().add_observer(t.observer());
    calc.is_ready().unwrap();
    (case.run)(&mut calc, dir);
    drop(calc);
    (compact(&t.render()), dissect_all(&t.packets()))
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Normalize files for comparison: blank the 42-byte comment (it has a date).
fn normalize_output(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.starts_with(b"**TI83F*") && bytes.len() > 53 {
        bytes[11..53].fill(0);
    }
    bytes
}

#[test]
fn matches_libticalcs_goldens() {
    let dir = tempfile::tempdir().unwrap();
    write_fixtures(dir.path());
    let mut failures = Vec::new();
    for case in CASES {
        let golden_path = golden_dir().join(format!("{}.txt", case.name));
        let Ok(golden) = std::fs::read_to_string(&golden_path) else {
            failures.push(format!(
                "{}: missing golden {}",
                case.name,
                golden_path.display()
            ));
            continue;
        };
        let (ours, our_dissect) = run_rust(case, dir.path());
        if DISSECT_CASES.contains(&case.name) {
            let p = golden_dir().join(format!("{}.dissect", case.name));
            match std::fs::read_to_string(&p) {
                Ok(g) if g == our_dissect => {}
                Ok(g) => {
                    let n = g
                        .lines()
                        .zip(our_dissect.lines())
                        .position(|(a, b)| a != b)
                        .unwrap_or(0);
                    failures.push(format!(
                        "{}: dissection differs at line {n}\n  libticalcs: {:?}\n  haccerlink: {:?}",
                        case.name,
                        g.lines().nth(n).unwrap_or("<end>"),
                        our_dissect.lines().nth(n).unwrap_or("<end>")
                    ));
                }
                Err(_) => failures.push(format!("{}: missing {}", case.name, p.display())),
            }
        }
        if ours != golden {
            let first_diff = ours
                .lines()
                .zip(golden.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(ours.lines().count().min(golden.lines().count()));
            failures.push(format!(
                "{}: transcript differs at packet {first_diff}\n  libticalcs: {}\n  haccerlink: {}",
                case.name,
                golden.lines().nth(first_diff).unwrap_or("<end>"),
                ours.lines().nth(first_diff).unwrap_or("<end>"),
            ));
        }
        if let Some(out) = case.output {
            let golden_file = golden_dir().join(format!("{}.out", case.name));
            let ours = normalize_output(std::fs::read(dir.path().join(out)).unwrap());
            match std::fs::read(&golden_file).map(|g| known_divergence(case.name, g)) {
                Ok(g) if g == ours => {}
                Ok(_) => failures.push(format!(
                    "{}: saved file differs from libtifiles'",
                    case.name
                )),
                Err(_) => {
                    failures.push(format!("{}: missing {}", case.name, golden_file.display()))
                }
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The simulator ends up with exactly the OS image that was sent.
#[test]
fn os_reaches_simulator() {
    let chunk = (sim::SIM_OS_CHUNK - 4) as usize;
    for len in [300, chunk, 3 * chunk, 3 * chunk + 7] {
        let os = fake_os(len);
        let state = Arc::new(Mutex::new(SimState::default()));
        let mut calc = Calc::new(sim::spawn(state.clone()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        calc.link()
            .set_progress(Some(Box::new(move |d, t| s2.lock().unwrap().push((d, t)))));
        calc.is_ready().unwrap();
        calc.send_os(&os).unwrap();
        assert_eq!(state.lock().unwrap().installed_os.as_ref(), Some(&os.data));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first(), Some(&(0, len)));
        assert_eq!(seen.last(), Some(&(len, len)));
        // The link's per-packet progress is restored afterwards.
        drop(seen);
        calc.list().unwrap();
    }
}

#[test]
fn send_os_rejects_bad_images() {
    let calc = || Calc::new(sim::spawn(Arc::new(Mutex::new(SimState::default()))));
    let mut no_marker = fake_os(600);
    no_marker.data.retain(|&b| b != 0xFF);
    assert!(calc().send_os(&no_marker).is_err());
    let mut other_hw = fake_os(600);
    other_hw.hw_id = 0x0A;
    assert!(calc().send_os(&other_hw).is_err());
    let app = FlashFile::from_received("X", types::APP, vec![0x81, 0x0F]);
    assert!(calc().send_os(&app).is_err());
}

/// Re-record goldens with the real libticalcs2. Needs `oracle/build/oracle`.
#[test]
#[ignore]
fn regenerate_goldens() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let oracle = root.join("oracle/build/oracle");
    assert!(
        oracle.exists(),
        "build the oracle first: nix develop .#oracle -c ./oracle/build.sh"
    );
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_haccerlink"));
    let dir = tempfile::tempdir().unwrap();
    write_fixtures(dir.path());
    std::fs::create_dir_all(golden_dir()).unwrap();
    for case in CASES {
        let tr = dir.path().join(format!("{}.raw", case.name));
        let args: Vec<String> = case
            .oracle
            .iter()
            .map(|a| a.replace('$', &dir.path().display().to_string()))
            .collect();
        let dissect = golden_dir().join(format!("{}.dissect", case.name));
        let mut cmd = std::process::Command::new(&oracle);
        if DISSECT_CASES.contains(&case.name) {
            cmd.env("ORACLE_DISSECT", &dissect);
        }
        let st = cmd.arg(&tr).arg(&bin).args(&args).status().unwrap();
        assert!(st.success(), "oracle failed on {}", case.name);
        let text = std::fs::read_to_string(&tr).unwrap();
        std::fs::write(
            golden_dir().join(format!("{}.txt", case.name)),
            compact(&text),
        )
        .unwrap();
        if let Some(out) = case.output {
            let bytes = normalize_output(std::fs::read(dir.path().join(out)).unwrap());
            std::fs::write(golden_dir().join(format!("{}.out", case.name)), bytes).unwrap();
        }
        eprintln!("recorded {}", case.name);
    }
}
