//! Throughput and per-frame cost of the emulator, without any UI.
//!
//!     cargo run --release -p term --example bench
//!
//! 1. Pushes ~27 MB of output through a real PTY and the parser, timing how
//!    long until the child has exited and everything is processed.
//! 2. Times `snapshot()` (the grid → runs conversion the UI does on every
//!    frame that has new output) on a full, colourful 200×60 screen.

use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let palette = term::Palette {
        fg: 0xffffff,
        bg: 0x000000,
        selection: 0x333333,
        search: 0x444444,
        search_focus: 0x666666,
        ansi: [0x808080; 16],
    };

    // 20 MB of random bytes, base64'd: ~27 MB of printable output.
    let cmd = "head -c 20000000 /dev/urandom | base64 -b 200";
    let mut spawn = term::Spawn::command(cmd, &cwd);
    spawn.program = Some("/bin/sh".into());
    spawn.args = vec!["-c".into(), cmd.into()];
    let started = Instant::now();
    let mut t = term::Terminal::spawn(spawn, 200, 60)?;
    t.resize(200, 60, 8, 16);
    while !t.has_exited() && started.elapsed() < Duration::from_secs(60) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let secs = started.elapsed().as_secs_f64();
    println!("throughput: 27 MB in {secs:.2}s = {:.0} MB/s", 27.0 / secs);

    // A colourful full screen: every cell styled, so runs are short.
    let mut colours = String::new();
    for row in 0..60 {
        for col in 0..200 {
            colours.push_str(&format!(
                "\x1b[38;5;{}m{}",
                (row * 7 + col) % 256,
                (b'a' + (col % 26) as u8) as char
            ));
        }
        colours.push_str("\x1b[0m\r\n");
    }
    let path = std::env::temp_dir().join("deck-bench-colours.txt");
    std::fs::write(&path, colours)?;
    let cmd = format!("cat {}; sleep 30", path.display());
    let mut spawn = term::Spawn::command(&cmd, &cwd);
    spawn.program = Some("/bin/sh".into());
    spawn.args = vec!["-c".into(), cmd];
    let mut t = term::Terminal::spawn(spawn, 200, 60)?;
    t.resize(200, 60, 8, 16);
    std::thread::sleep(Duration::from_millis(800));
    let n = 500;
    let at = Instant::now();
    for _ in 0..n {
        t.snapshot(&palette, true);
    }
    let per = at.elapsed() / n;
    println!("snapshot of a 200x60 screen, every cell coloured: {per:?} per frame");
    t.shutdown();
    Ok(())
}
