//! Run a command in the emulator headlessly and print the screen.
//!
//!     cargo run -p term --example dump -- 'opencode' 6
//!
//! For debugging a TUI that renders wrong (or not at all) in the app, with no
//! UI in the way.

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "echo hello".into());
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(4);
    let cwd = std::env::current_dir()?;

    let mut spawn = term::Spawn::command(&cmd, &cwd);
    spawn.program = Some("/bin/sh".into());
    spawn.args = vec!["-c".into(), cmd.clone()];
    let mut t = term::Terminal::spawn(spawn, 120, 40)?;
    t.resize(120, 40, 8, 16);
    std::thread::sleep(std::time::Duration::from_secs(secs));

    let palette = term::Palette {
        fg: 0xffffff,
        bg: 0x000000,
        selection: 0x333333,
        search: 0x555555,
        search_focus: 0xeeeeee,
        ansi: [0; 16],
    };
    t.snapshot(&palette, true);
    let snap = t.last();
    let mut painted = 0;
    for (i, row) in snap.rows.iter().enumerate() {
        let text: String = row
            .runs
            .iter()
            .flat_map(|r| r.text.chars())
            .filter(|&c| c != '\0')
            .collect();
        painted += row.runs.iter().filter(|r| r.style.bg.is_some()).count();
        println!("{i:>2}|{}", text.trim_end());
    }
    println!(
        "-- cursor {:?}, runs with a background: {painted}, exited: {}",
        snap.cursor,
        t.has_exited()
    );
    for ev in t.drain_events() {
        println!("-- event {ev:?}");
    }
    Ok(())
}
