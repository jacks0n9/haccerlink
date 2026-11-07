# haccerlink

[![CI](https://github.com/jacks0n9/haccerlink/actions/workflows/ci.yml/badge.svg)](https://github.com/jacks0n9/haccerlink/actions/workflows/ci.yml)

A terminal link program for the **TI-84 Plus CE**, in Rust. It speaks DUSB,
the USB protocol the CE uses (not the older DBUS "silent link" protocol of the
pre-USB calculators), directly through Linux's usbfs via [`nusb`](https://docs.rs/nusb), so there are no
C dependencies and no libusb.

```
 haccerlink  TI-84 Plus CE · OS 5.8.2.0020 · RAM 150K free · Archive 2.9M free · bus 1 addr 7
┌ ~/calc/progs ──────────────────────┐┌ Calculator: 5 variables · 1 app ────────────────────┐
│  ../                               ││  Name          Type   Size    Where                 │
│  HELLO.8xp                   90    ││  A             REAL   9       RAM                   │
│● TETRIS.8xp                4123    ││  HELLO         PRGM   16      RAM                   │
│  SAVEDAT.8xv                 92    ││  SAVEDAT       APPV   18      archive               │
```

## Status

The protocol code is a port of libticalcs2 (what TiLP uses). Every operation
below produces **byte-identical packets to libticalcs2**: the test suite runs
the real libticalcs2 and haccerlink against the same simulated calculator and
diffs the traffic. It has been used on a real TI-84 Plus CE (OS 5.4) for
info, listing, screenshots, clock, sending, receiving and deleting. Linux only
for now.

| | |
|---|---|
| send variables (.8xp .8xv .8xl .8xs .8xn .8xm .8ci .8ca .8xg group files …) | ✓ |
| send apps (.8ek) | ✓ |
| send bundles (.b84 .b83 .tig) | ✓ |
| receive variables and apps | ✓ |
| list, delete, rename, archive/unarchive | ✓ |
| screenshot (TUI preview + PNG) | ✓ |
| remote control (keypresses), run program | ✓ |
| get/set clock, version info, free memory | ✓ |
| OS upgrades (.8eu, or the OS in a bundle) | ✓ |

## Building

```sh
nix develop -c cargo build --release     # or plain `cargo build --release`
nix build                                 # package via the flake
```

## USB permissions

Opening the calculator needs write access to its usbfs node. Without it you'll
get a "could not open … permissions" error. On NixOS:

```nix
services.udev.extraRules = ''
  SUBSYSTEM=="usb", ATTR{idVendor}=="0451", ATTR{idProduct}=="e003|e008", TAG+="uaccess"
'';
```

On other distros, put the same rule in `/etc/udev/rules.d/69-ti-calc.rules` and
run `udevadm control --reload`. Replug the calculator afterwards.

## Usage

`haccerlink` with no arguments opens the TUI. Add `--sim` to anything to use a
simulated calculator instead of hardware.

### TUI keys

| key | |
|---|---|
| Tab / ← → | switch between computer and calculator pane |
| Space | mark/unmark (works in both panes) |
| Enter | open folder / send file (left), receive into current folder (right) |
| Enter on a bundle | pick which items to send: Space marks, `a` all/none, `A` all but the OS, Enter/`s` sends, `S` sends to the archive |
| s on a bundle | send the whole bundle |
| s / S | send marked files / send them to the archive |
| g | receive marked variables |
| d, a, r, x | delete, archive⇄RAM, rename, run |
| p | screenshot (`s` saves PNG, `r` refreshes, `k` switches to remote) |
| K | remote control: typing goes to the calculator, Esc stops. F1–F5 = Y= WINDOW ZOOM TRACE GRAPH, F6 MODE, F7 QUIT, Backspace DEL, Delete CLEAR |
| i, t | calculator info, set clock to PC time |
| l | toggle message log / live packet log |
| R, c, ?, q | refresh, reconnect, help, quit |

Screenshots are drawn with the kitty graphics protocol (pixel-exact, scaled
by whole multiples) in kitty, WezTerm and Ghostty, and with colored half
blocks elsewhere, including inside tmux. Override with
`--images kitty|blocks|auto`.

### CLI

```sh
haccerlink devices                      # find calculators
haccerlink info                         # model, OS, memory
haccerlink ls
haccerlink send game.8xp data.8xv --archive
haccerlink send --os TI84CEBundle-5.8.3.48.b84   # OS upgrade + TI's apps
haccerlink send TI84CEBundle-5.8.3.48.b84 --only Python,CabriJr   # just these
haccerlink get HELLO -o hello.8xp
haccerlink get --all -o backup/
haccerlink rm A -t REAL
haccerlink mv HELLO HI
haccerlink archive HELLO
haccerlink screenshot -o screen.png
haccerlink key clear 2 + 2 enter
haccerlink clock --set-now
haccerlink inspect file.8xp             # show what's in a TI file
```

### Logs (for comparing against TiLP)

- `--log-hex FILE` writes the same format as libticables'
  `~/.ticables/ticables-log.hex`, so a haccerlink log and a TiLP log of the
  same operation can be diffed directly.
- `--dissect FILE` writes libticalcs' `dusb_dissect()` format: every packet
  decoded, with direction.

### OS upgrades

`haccerlink send --os FILE` installs the OS from an `.8eu` file or from a
bundle that contains one. Without `--os`, OS files are skipped. In the TUI,
sending anything that contains an OS asks first, and `s` in that dialog sends
everything except the OS.

- The OS is sent before anything else. Once it's installed, the calculator
  checks it, restarts and clears its RAM. haccerlink waits for it to come
  back on USB (up to a few minutes) and then sends the rest, so RAM
  variables in the same send aren't lost.
- The calculator erases its old OS before receiving the new one. If the
  transfer is interrupted, it stays in the boot code's "waiting for OS"
  screen until an OS is sent again, which haccerlink can do. It may refuse
  OS versions it doesn't allow, and haccerlink reports its error code.
- The packets match libticalcs2's `send_os_834pce` byte for byte. On real
  hardware it has reinstalled OS 5.6.0.0020 on a CE left without an OS by an
  interrupted transfer (boot code 5.6.1.0006, which asks for 1004-byte
  chunks). The reconnect after the restart hasn't been tried on hardware
  yet.
- A CE without an OS refuses normal mode (error 0x0009). `send --os` and
  the TUI carry on anyway, so an OS can be sent to recover it.

## How it's tested without a calculator

- `src/sim.rs` is a simulated CE that implements the calculator side of DUSB
  (same `Link` code, device role). `haccerlink sim-serve` exposes it on
  stdin/stdout.
- `oracle/oracle.c` links the **real libticalcs2** and defines
  `ticables_cable_send/recv` itself. Exported with `-rdynamic`, those take
  the place of libticables', so unmodified TiLP library code talks to the
  simulator over a pipe while every packet is recorded.
- `tests/oracle.rs` replays 30 operations through haccerlink and requires
  the same packets in both directions as libticalcs (`tests/golden/*.txt`).
  For 24 of them it also checks that `--dissect` output matches libticalcs'
  `dusb_dissect()`, and for receives that the saved files match libtifiles.
  The cases cover multi-packet transfers, the CE's zero-length-packet quirk,
  libticalcs' empty final packet, both key code byte orders, name
  conversion (`L₁`, `Str1`, accented app names like `Español`, …), and OS
  transfers (including the empty last OS Data packet libticalcs sends when
  the OS fills its chunks exactly).

```sh
nix develop -c cargo test                      # uses the checked-in goldens
nix develop .#oracle -c ./oracle/build.sh      # rebuild the oracle
nix develop .#oracle -c cargo test --test oracle -- --ignored regenerate_goldens
```

### Where haccerlink intentionally differs from TiLP

- Receiving `L₁`–`L₆`: libticonv turns the name into a *named* list "L₁"
  (`5D 4C 81`), so TiLP saves a file that creates a bogus custom list when
  sent back. haccerlink writes the real `5D 00`. Matrices (`[A]`) have the
  same kind of problem (libticonv maps `[` to 0xC1 before the tokenizer looks
  for it, so TiLP saves `C1 41 5D` instead of `5C 00`); also fixed here. Both
  divergences are asserted in the tests.
- OS transfers: libticalcs refuses nothing, while haccerlink refuses an
  OS file for other hardware and an image without the FF FF FF FF
  end-of-header marker (libticalcs would read past the end of the buffer
  looking for it). It also waits longer for the calculator's answers during
  the transfer. None of this changes the packets.
- `--dissect` copies a libticalcs dissector bug on purpose so logs diff
  cleanly: for a virtual packet split over several raw packets, it decodes
  the command fields from the wrong offset (CE OS Data addresses show up as
  e.g. `03000000` instead of `00030000`).

## Trying it on a new calculator or OS version

Turn the calculator on (home screen) and plug it in, then try, roughly in
order of risk:

```sh
haccerlink devices
haccerlink --log-hex info.hex --dissect info.txt info
haccerlink ls
haccerlink screenshot
haccerlink get <some program> -o test.8xp
haccerlink send test.8xp          # after renaming it, or to a calc without it
```

If anything fails, rerun it with `--log-hex` and `--dissect` and attach both
files to an issue: they show exactly where the calculator's answers differ
from what libticalcs expects.

## License

GPL-3.0-or-later (see [LICENSE](LICENSE)). The protocol code is ported from
[libticalcs2](https://github.com/debrouxl/tilibs) (GPL-2.0-or-later), part of
the TiLP project. TI-84 Plus CE and TI Connect are trademarks of Texas
Instruments; this project is not affiliated with TI.
