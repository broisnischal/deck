//! A look at the bytes on their way to the emulator, for the escape codes
//! alacritty's parser drops but deck acts on: desktop notifications (OSC 9 as
//! iTerm2 has it, OSC 777 as Ghostty and rxvt do) and progress (OSC 9;4, as
//! ConEmu and Windows Terminal have it). That is how an agent CLI says it
//! needs you, or that it is working.
//!
//! The PTY is wrapped, not replaced: alacritty still owns it and polls its
//! descriptor; the reader it is handed reads through a duplicate of that
//! descriptor (the same open file, so the same bytes) and scans them first.

use crate::{Shared, TermEvent};
use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use polling::{Event, PollMode, Poller};
use std::fs::File;
use std::io::{self, Read};
use std::sync::Arc;

/// What a program said that deck acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    Notify {
        title: Option<String>,
        body: String,
    },
    /// OSC 9;4: `Some(state)` while working (1 a percentage, 2 an error,
    /// 3 indeterminate, 4 paused), `None` once it clears.
    Progress(Option<u8>),
}

/// An OSC longer than this is not one we want, and is not kept.
const MAX_OSC: usize = 4096;

#[derive(Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Ground,
    Esc,
    Osc,
    /// An ESC inside an OSC: `\` ends it, anything else abandons it.
    OscEsc,
}

/// A byte-at-a-time recogniser that survives a sequence split across reads.
#[derive(Default)]
pub struct Scanner {
    state: State,
    buf: Vec<u8>,
    too_long: bool,
}

impl Scanner {
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Signal>) {
        let mut i = 0;
        while i < bytes.len() {
            // Plain output is nearly everything: skip to the next ESC at
            // memchr speed rather than looking at every byte.
            if self.state == State::Ground {
                match memchr::memchr(0x1b, &bytes[i..]) {
                    Some(at) => {
                        i += at + 1;
                        self.state = State::Esc;
                        continue;
                    }
                    None => return,
                }
            }
            let b = bytes[i];
            i += 1;
            self.state = match self.state {
                State::Ground if b == 0x1b => State::Esc,
                State::Ground => State::Ground,
                State::Esc if b == b']' => self.begin(),
                State::Esc if b == 0x1b => State::Esc,
                State::Esc => State::Ground,
                State::Osc => match b {
                    0x07 => self.end(out),
                    0x1b => State::OscEsc,
                    // CAN and SUB abandon a sequence.
                    0x18 | 0x1a => State::Ground,
                    _ => {
                        if self.buf.len() < MAX_OSC {
                            self.buf.push(b);
                        } else {
                            self.too_long = true;
                        }
                        State::Osc
                    }
                },
                State::OscEsc if b == b'\\' => self.end(out),
                State::OscEsc if b == b']' => self.begin(),
                State::OscEsc if b == 0x1b => State::Esc,
                State::OscEsc => State::Ground,
            };
        }
    }

    fn begin(&mut self) -> State {
        self.buf.clear();
        self.too_long = false;
        State::Osc
    }

    fn end(&mut self, out: &mut Vec<Signal>) -> State {
        if !self.too_long {
            if let Some(signal) = parse(&String::from_utf8_lossy(&self.buf)) {
                out.push(signal);
            }
        }
        self.buf.clear();
        State::Ground
    }
}

fn parse(osc: &str) -> Option<Signal> {
    if let Some(rest) = osc.strip_prefix("9;") {
        if let Some(progress) = rest.strip_prefix("4;") {
            let state = progress.split(';').next()?.parse::<u8>().ok()?;
            return Some(Signal::Progress((state != 0).then_some(state)));
        }
        let body = rest.trim();
        return (!body.is_empty()).then(|| Signal::Notify {
            title: None,
            body: body.to_string(),
        });
    }
    if let Some(rest) = osc.strip_prefix("777;notify;") {
        let (title, body) = rest.split_once(';').unwrap_or(("", rest));
        let (title, body) = (title.trim(), body.trim());
        if body.is_empty() && title.is_empty() {
            return None;
        }
        return Some(Signal::Notify {
            title: (!title.is_empty()).then(|| title.to_string()),
            body: body.to_string(),
        });
    }
    None
}

/// The reader alacritty is handed.
pub struct Tap {
    file: File,
    scan: Scanner,
    found: Vec<Signal>,
    shared: Arc<Shared>,
}

impl Read for Tap {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read(buf)?;
        self.scan.feed(&buf[..n], &mut self.found);
        if !self.found.is_empty() {
            if let Ok(mut q) = self.shared.events.lock() {
                q.extend(self.found.drain(..).map(|s| match s {
                    Signal::Notify { title, body } => TermEvent::Notify { title, body },
                    Signal::Progress(p) => TermEvent::Progress(p),
                }));
            }
            self.found.clear();
            self.shared.mark_dirty();
        }
        Ok(n)
    }
}

/// alacritty's PTY with [`Tap`] as its reader.
pub struct Tapped {
    pty: Pty,
    reader: Tap,
}

impl Tapped {
    pub fn new(pty: Pty, shared: Arc<Shared>) -> io::Result<Tapped> {
        let file = pty.file().try_clone()?;
        Ok(Tapped {
            pty,
            reader: Tap {
                file,
                scan: Scanner::default(),
                found: Vec::new(),
                shared,
            },
        })
    }
}

impl EventedReadWrite for Tapped {
    type Reader = Tap;
    type Writer = File;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        event: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        // SAFETY: the PTY is owned here and outlives its registration, as
        // alacritty's own contract asks.
        unsafe { self.pty.register(poll, event, mode) }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, event: Event, mode: PollMode) -> io::Result<()> {
        self.pty.reregister(poll, event, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.pty.deregister(poll)
    }

    fn reader(&mut self) -> &mut Tap {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut File {
        self.pty.writer()
    }
}

impl EventedPty for Tapped {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.pty.next_child_event()
    }
}

impl OnResize for Tapped {
    fn on_resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<Signal> {
        let mut s = Scanner::default();
        let mut out = Vec::new();
        for c in chunks {
            s.feed(c, &mut out);
        }
        out
    }

    #[test]
    fn notifications_in_both_dialects_and_split_reads() {
        assert_eq!(
            scan(&[b"hi \x1b]9;Claude needs your permission to use Bash\x07 x"]),
            vec![Signal::Notify {
                title: None,
                body: "Claude needs your permission to use Bash".into()
            }]
        );
        assert_eq!(
            scan(&[
                b"\x1b]777;notify;Claude Code;",
                b"Waiting for your input\x1b",
                b"\\"
            ]),
            vec![Signal::Notify {
                title: Some("Claude Code".into()),
                body: "Waiting for your input".into()
            }]
        );
    }

    #[test]
    fn progress_sets_and_clears() {
        assert_eq!(
            scan(&[b"\x1b]9;4;3;\x07\x1b]9;4;1;40\x1b\\\x1b]9;4;0;\x07"]),
            vec![
                Signal::Progress(Some(3)),
                Signal::Progress(Some(1)),
                Signal::Progress(None)
            ]
        );
    }

    #[test]
    fn other_sequences_and_aborts_are_ignored() {
        // A title, a hyperlink, an OSC abandoned by CSI, and a lone ESC.
        assert!(scan(&[
            b"\x1b]0;\xe2\x9c\xb3 topic\x07\x1b]8;;http://a\x07x\x1b]8;;\x07",
            b"\x1b]9;half\x1b[31m red \x1b",
        ])
        .is_empty());
        let long = [b"\x1b]9;".as_slice(), &vec![b'a'; MAX_OSC + 10], b"\x07"].concat();
        assert!(scan(&[&long]).is_empty());
    }
}
