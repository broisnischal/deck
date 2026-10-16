//! What a pane is doing, read from the OS: its process tree, working
//! directory, listening ports, and the git state of the project it sits in.
//!
//! Pure data, no gpui. The inspector refreshes on a timer while the user
//! works, so processes and ports are read straight from the kernel through
//! libproc (microseconds) rather than by forking `ps` and `lsof` (tens of
//! milliseconds each). Only git still runs a child process.

use std::collections::{HashMap, HashSet};
use std::ffi::{c_int, c_void};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    /// Percent of one core, as `ps %cpu` reports it.
    pub cpu: f32,
    /// Resident set size in bytes.
    pub rss: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub port: u16,
    pub pid: u32,
    pub process: String,
    /// `127.0.0.1`, `::1` (brackets stripped) or `*` for every interface.
    pub addr: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GitFile {
    pub path: String,
    /// `M`, `A`, `D`, `R`, `C`, `U` or `??`.
    pub status: String,
    /// Has changes in the index (staged), and in the working tree.
    pub staged: bool,
    pub unstaged: bool,
}

/// A checkout of the repository: the main one or one added with
/// `git worktree add`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub current: bool,
}

/// A commit in the recent history.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    /// Unix seconds.
    pub date: i64,
    pub refs: Vec<String>,
    /// More than one parent.
    pub merge: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GitInfo {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<GitFile>,
    pub worktrees: Vec<Worktree>,
    pub commits: Vec<Commit>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

const MAX_GIT_FILES: usize = 500;
const MAX_ENTRIES: usize = 1000;

// ---------------------------------------------------------------------------
// Processes
// ---------------------------------------------------------------------------

/// `root` and every descendant, parent first (depth-first, root at index 0).
/// Empty when `root` is not running.
///
/// `cpu` is the share of one core used since the previous call that saw the
/// process (0 on the first sighting), so it reads like Activity Monitor's
/// live figure rather than `ps`'s lifetime average. Processes owned by
/// another user, such as the setuid `login` that wraps each shell, report 0
/// for `cpu` and `rss`: the kernel only discloses those to root.
pub fn descendants(root: u32) -> Vec<Proc> {
    let Some(root_ppid) = short_info(root).map(|i| i.pbsi_ppid) else {
        return Vec::new();
    };
    // Collect the subtree (one listpids call per process), then order it.
    let mut procs = Vec::new();
    let mut pending = vec![(root, root_ppid)];
    let mut seen = HashSet::new();
    while let Some((pid, ppid)) = pending.pop() {
        if !seen.insert(pid) {
            continue;
        }
        let (cpu_ns, rss) = task_usage(pid).unwrap_or((None, 0));
        procs.push((
            Proc {
                pid,
                ppid,
                name: name_of(pid),
                cpu: 0.,
                rss,
            },
            cpu_ns,
        ));
        pending.extend(child_pids(pid).into_iter().map(|c| (c, pid)));
    }
    rate_cpu(&mut procs, Instant::now());
    tree_order(&procs.into_iter().map(|(p, _)| p).collect::<Vec<_>>(), root)
}

/// Turn cumulative CPU time into a percentage against the last sample.
fn rate_cpu(procs: &mut [(Proc, Option<u64>)], now: Instant) {
    static LAST: OnceLock<Mutex<HashMap<u32, (u64, Instant)>>> = OnceLock::new();
    let Ok(mut last) = LAST.get_or_init(Default::default).lock() else {
        return;
    };
    for (p, cpu_ns) in procs.iter_mut() {
        let Some(ns) = *cpu_ns else { continue };
        if let Some(&(prev_ns, at)) = last.get(&p.pid) {
            let wall = now.saturating_duration_since(at).as_nanos() as f64;
            // A reused pid starts below the old counter; that reads as 0.
            if wall > 0. && ns >= prev_ns {
                p.cpu = ((ns - prev_ns) as f64 / wall * 100.) as f32;
            }
        }
        last.insert(p.pid, (ns, now));
    }
    // Several panes share this table, so a pid one caller did not visit is
    // not necessarily dead; drop only what nobody has sampled for a while.
    last.retain(|_, &mut (_, at)| now.saturating_duration_since(at) < Duration::from_secs(60));
}

fn tree_order(procs: &[Proc], root: u32) -> Vec<Proc> {
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    if !by_pid.contains_key(&root) {
        return Vec::new();
    }
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in procs {
        // launchd (pid 0/1) is its own parent; an edge to itself would loop.
        if p.pid != p.ppid {
            children.entry(p.ppid).or_default().push(p.pid);
        }
    }
    for kids in children.values_mut() {
        // Stack pops from the end, so reverse order yields ascending pids.
        kids.sort_unstable_by(|a, b| b.cmp(a));
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(p) = by_pid.get(&pid) {
            out.push((*p).clone());
        }
        if let Some(kids) = children.get(&pid) {
            stack.extend(kids.iter().copied());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Working directory
// ---------------------------------------------------------------------------

const PROC_PIDVNODEPATHINFO: c_int = 9;
const MAXPATHLEN: usize = 1024;

/// `struct vnode_info_path` from <sys/proc_info.h>: a 152-byte `vnode_info`
/// (kept opaque, u64-aligned like the real one) followed by the path.
#[repr(C)]
struct VnodeInfoPath {
    _vip_vi: [u64; 19],
    vip_path: [u8; MAXPATHLEN],
}

#[repr(C)]
struct ProcVnodePathInfo {
    pvi_cdir: VnodeInfoPath,
    _pvi_rdir: VnodeInfoPath,
}

extern "C" {
    // libproc and mach_time live in libSystem, which every binary links.
    fn proc_pidinfo(
        pid: c_int,
        flavor: c_int,
        arg: u64,
        buffer: *mut c_void,
        buffersize: c_int,
    ) -> c_int;
    fn proc_pidfdinfo(
        pid: c_int,
        fd: c_int,
        flavor: c_int,
        buffer: *mut c_void,
        buffersize: c_int,
    ) -> c_int;
    fn proc_listpids(kind: u32, typeinfo: u32, buffer: *mut c_void, buffersize: c_int) -> c_int;
    fn proc_name(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;
    fn sysctl(
        name: *mut c_int,
        namelen: u32,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> c_int;
}

const PROC_PIDLISTFDS: c_int = 1;
const PROC_PIDTBSDINFO: c_int = 3;
const PROC_PIDFDSOCKETINFO: c_int = 3;
const PROC_PIDTASKINFO: c_int = 4;
const PROC_PIDT_SHORTBSDINFO: c_int = 13;
const PROC_PPID_ONLY: u32 = 6;
const PROX_FDTYPE_SOCKET: u32 = 2;
const SOCKINFO_TCP: i32 = 2;
const TSI_S_LISTEN: i32 = 1;
const INI_IPV6: u8 = 2;

/// `struct proc_taskinfo` (96 bytes). Times are in mach absolute units, which
/// are not nanoseconds on Apple silicon (the timebase there is 125/3).
#[repr(C)]
#[derive(Default)]
struct ProcTaskInfo {
    pti_virtual_size: u64,
    pti_resident_size: u64,
    pti_total_user: u64,
    pti_total_system: u64,
    pti_threads_user: u64,
    pti_threads_system: u64,
    _counters: [i32; 12],
}

/// `struct proc_bsdshortinfo` (64 bytes): readable for any process, unlike
/// the task info, which is why it is the liveness and parent check.
#[repr(C)]
#[derive(Default)]
struct ProcBsdShortInfo {
    pbsi_pid: u32,
    pbsi_ppid: u32,
    pbsi_pgid: u32,
    pbsi_status: u32,
    pbsi_comm: [u8; 16],
    pbsi_flags: u32,
    _ids: [u32; 6],
    _rfu: u32,
}

/// `struct proc_fdinfo` (8 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ProcFdInfo {
    proc_fd: i32,
    proc_fdtype: u32,
}

/// `struct socket_fdinfo` (792 bytes), kept as bytes and read at fixed
/// offsets: the real type is a deep tree of unions, and only five fields of
/// it matter. Offsets are from <sys/proc_info.h> and pinned by a test.
#[repr(C, align(8))]
struct SocketFdInfo([u8; SOCKET_FDINFO_SIZE]);

const SOCKET_FDINFO_SIZE: usize = 792;
/// `psi` (socket_info) starts at 24; inside it `soi_kind` is at 232 and the
/// protocol union at 240, whose TCP arm is `tcp_sockinfo`: an `in_sockinfo`
/// at 0 (local port at 4, `insi_vflag` at 24, local address at 48) and
/// `tcpsi_state` at 80.
const SOI_KIND: usize = 24 + 232;
const INSI: usize = 24 + 240;
const INSI_LPORT: usize = INSI + 4;
const INSI_VFLAG: usize = INSI + 24;
const INSI_LADDR: usize = INSI + 48;
const TCPSI_STATE: usize = INSI + 80;

#[repr(C)]
#[derive(Default)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

fn pidinfo<T: Default>(pid: u32, flavor: c_int) -> Option<T> {
    let mut v = T::default();
    let size = std::mem::size_of::<T>() as c_int;
    // SAFETY: `v` is a plain-integer struct of exactly `size` bytes, and the
    // kernel writes at most `size` bytes into it.
    let n = unsafe { proc_pidinfo(pid as c_int, flavor, 0, (&mut v as *mut T).cast(), size) };
    (n == size).then_some(v)
}

fn short_info(pid: u32) -> Option<ProcBsdShortInfo> {
    pidinfo::<ProcBsdShortInfo>(pid, PROC_PIDT_SHORTBSDINFO).filter(|i| i.pbsi_pid == pid)
}

/// Cumulative CPU time in nanoseconds, and resident bytes.
fn task_usage(pid: u32) -> Option<(Option<u64>, u64)> {
    let ti = pidinfo::<ProcTaskInfo>(pid, PROC_PIDTASKINFO)?;
    let ticks = ti.pti_total_user.saturating_add(ti.pti_total_system);
    Some((Some(ticks_to_ns(ticks)), ti.pti_resident_size))
}

fn ticks_to_ns(ticks: u64) -> u64 {
    static TB: OnceLock<(u64, u64)> = OnceLock::new();
    let &(numer, denom) = TB.get_or_init(|| {
        let mut tb = MachTimebaseInfo::default();
        // SAFETY: writes one small struct we own.
        let ok = unsafe { mach_timebase_info(&mut tb) } == 0 && tb.denom != 0;
        if ok {
            (tb.numer as u64, tb.denom as u64)
        } else {
            (1, 1)
        }
    });
    (ticks as u128 * numer as u128 / denom as u128) as u64
}

/// `struct proc_bsdinfo` (136 bytes); only the terminal fields are read.
#[repr(C)]
struct ProcBsdInfo {
    _ids: [u32; 12],
    _comm: [u8; 16],
    _name: [u8; 32],
    _nfiles: u32,
    _pgid: u32,
    _pjobc: u32,
    _tdev: u32,
    /// The foreground process group of the process's terminal.
    tpgid: u32,
    _nice: i32,
    _start_sec: u64,
    _start_usec: u64,
}

/// The process in front of `shell`'s terminal: the leader of its
/// foreground process group (`claude` while Claude Code runs, the shell
/// itself at a prompt). Exact, unlike guessing from the process tree, where
/// an agent's helpers sit below it.
pub fn foreground(shell: u32) -> Option<u32> {
    // SAFETY: an all-zero proc_bsdinfo is valid; the kernel fills it.
    let mut info: ProcBsdInfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<ProcBsdInfo>() as c_int;
    // SAFETY: `info` is exactly the size passed.
    let n = unsafe {
        proc_pidinfo(
            shell as c_int,
            PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut ProcBsdInfo).cast(),
            size,
        )
    };
    (n == size && info.tpgid != 0).then_some(info.tpgid)
}

/// A process's name, as the inspector lists it.
pub fn process_name(pid: u32) -> String {
    name_of(pid)
}

/// `2.1.300`: a file named after a release rather than a program.
fn is_version(name: &str) -> bool {
    name.contains('.') && name.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// A process's arguments, `argv[0]` first. Empty for one we may not read
/// (another user's) or that has exited.
pub fn argv(pid: u32) -> Vec<String> {
    const CTL_KERN: c_int = 1;
    const KERN_PROCARGS2: c_int = 49;
    let mut mib = [CTL_KERN, KERN_PROCARGS2, pid as c_int];
    let mut size = 0usize;
    let null = std::ptr::null_mut();
    // SAFETY: a size query; the kernel writes only `size`.
    if unsafe { sysctl(mib.as_mut_ptr(), 3, null, &mut size, null, 0) } != 0 || size < 4 {
        return Vec::new();
    }
    let mut buf = vec![0u8; size];
    // SAFETY: the kernel writes at most `size` bytes and updates `size`.
    if unsafe {
        sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut size,
            null,
            0,
        )
    } != 0
    {
        return Vec::new();
    }
    buf.truncate(size);
    parse_procargs(&buf)
}

/// KERN_PROCARGS2: `argc`, the executable's path, NUL padding, then the
/// arguments (and the environment after them).
fn parse_procargs(buf: &[u8]) -> Vec<String> {
    let Some(argc) = buf
        .get(..4)
        .and_then(|b| b.try_into().ok())
        .map(i32::from_ne_bytes)
    else {
        return Vec::new();
    };
    let rest = &buf[4..];
    let Some(path_end) = rest.iter().position(|&b| b == 0) else {
        return Vec::new();
    };
    let rest = &rest[path_end..];
    let start = rest.iter().position(|&b| b != 0).unwrap_or(rest.len());
    rest[start..]
        .split(|&b| b == 0)
        .take(argc.max(0) as usize)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect()
}

fn name_of(pid: u32) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the kernel writes at most `buf.len()` bytes and returns the length.
    let n = unsafe { proc_name(pid as c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n > 0 {
        let name = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
        // Claude Code's binary is its version (`…/claude/versions/2.1.300`);
        // the name it was started by says what it is.
        if is_version(&name) {
            if let Some(cmd) = argv(pid).first().and_then(|a| a.rsplit('/').next()) {
                if !cmd.is_empty() {
                    return cmd.to_string();
                }
            }
        }
        return name;
    }
    // Another user's process (setuid `login`): the 16-byte `comm` is public.
    short_info(pid)
        .map(|i| {
            let comm = &i.pbsi_comm;
            let len = comm.iter().position(|&b| b == 0).unwrap_or(comm.len());
            String::from_utf8_lossy(&comm[..len]).into_owned()
        })
        .unwrap_or_default()
}

fn child_pids(pid: u32) -> Vec<u32> {
    let mut buf = vec![0i32; 64];
    loop {
        let bytes = (buf.len() * 4) as c_int;
        // SAFETY: the buffer holds `bytes` bytes; the call returns how many it
        // filled (bytes, not a count).
        let n = unsafe { proc_listpids(PROC_PPID_ONLY, pid, buf.as_mut_ptr().cast(), bytes) };
        if n <= 0 {
            return Vec::new();
        }
        // A full buffer may have been truncated; grow and ask again.
        if n >= bytes && buf.len() < 1 << 16 {
            buf.resize(buf.len() * 4, 0);
            continue;
        }
        return buf[..n as usize / 4]
            .iter()
            .filter(|&&p| p > 0)
            .map(|&p| p as u32)
            .collect();
    }
}

fn fds(pid: u32) -> Vec<ProcFdInfo> {
    // SAFETY: a null buffer asks for the size needed.
    let need = unsafe { proc_pidinfo(pid as c_int, PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if need <= 0 {
        return Vec::new();
    }
    // Slack for descriptors opened between the two calls.
    let mut buf = vec![ProcFdInfo::default(); need as usize / 8 + 16];
    let bytes = (buf.len() * 8) as c_int;
    // SAFETY: `buf` holds `bytes` bytes of plain-integer structs.
    let n = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDLISTFDS,
            0,
            buf.as_mut_ptr().cast(),
            bytes,
        )
    };
    if n <= 0 {
        return Vec::new();
    }
    buf.truncate(n as usize / 8);
    buf
}

/// The local port and address of `fd` if it is a TCP socket in LISTEN.
fn tcp_listener(pid: u32, fd: i32) -> Option<(u16, String)> {
    let mut info = SocketFdInfo([0; SOCKET_FDINFO_SIZE]);
    // SAFETY: the buffer is exactly the struct's size; the kernel writes at
    // most that many bytes.
    let n = unsafe {
        proc_pidfdinfo(
            pid as c_int,
            fd,
            PROC_PIDFDSOCKETINFO,
            info.0.as_mut_ptr().cast(),
            SOCKET_FDINFO_SIZE as c_int,
        )
    };
    if n != SOCKET_FDINFO_SIZE as c_int {
        return None;
    }
    let b = &info.0;
    let i32_at = |o: usize| i32::from_ne_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    if i32_at(SOI_KIND) != SOCKINFO_TCP || i32_at(TCPSI_STATE) != TSI_S_LISTEN {
        return None;
    }
    // The port is stored in network order in the low 16 bits of an int.
    let port = u16::from_be(i32_at(INSI_LPORT) as u16);
    let raw: [u8; 16] = b[INSI_LADDR..INSI_LADDR + 16].try_into().ok()?;
    let addr = if b[INSI_VFLAG] & INI_IPV6 != 0 {
        let a = std::net::Ipv6Addr::from(raw);
        if a.is_unspecified() {
            "*".to_string()
        } else {
            a.to_string()
        }
    } else {
        // `in4in6_addr`: 12 bytes of padding, then the IPv4 address.
        let a = std::net::Ipv4Addr::new(raw[12], raw[13], raw[14], raw[15]);
        if a.is_unspecified() {
            "*".to_string()
        } else {
            a.to_string()
        }
    };
    Some((port, addr))
}

/// The current directory of `pid`, as the kernel tracks it: this follows a
/// shell's `cd` without asking the shell anything.
pub fn cwd(pid: u32) -> Option<PathBuf> {
    let mut info = std::mem::MaybeUninit::<ProcVnodePathInfo>::zeroed();
    let size = std::mem::size_of::<ProcVnodePathInfo>() as c_int;
    // SAFETY: the buffer is exactly `size` bytes and the kernel writes at most
    // that many; zeroed memory is a valid value of this all-integer struct.
    let n = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if n != size {
        return None;
    }
    // SAFETY: fully initialised: zeroed, then filled by a successful call.
    let info = unsafe { info.assume_init() };
    let raw = &info.pvi_cdir.vip_path;
    let len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    if len == 0 {
        return None;
    }
    use std::os::unix::ffi::OsStrExt;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&raw[..len])))
}

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

/// TCP sockets in LISTEN owned by any of `pids`, sorted by port. An IPv4
/// and an IPv6 listener on one port are one service and appear once.
pub fn listening_ports(pids: &[u32]) -> Vec<Port> {
    let mut ports = Vec::new();
    let mut seen = HashSet::new();
    for &pid in pids {
        let mut name = None;
        for fd in fds(pid) {
            if fd.proc_fdtype != PROX_FDTYPE_SOCKET {
                continue;
            }
            let Some((port, addr)) = tcp_listener(pid, fd.proc_fd) else {
                continue;
            };
            if !seen.insert((port, pid)) {
                continue;
            }
            let process = name.get_or_insert_with(|| name_of(pid)).clone();
            ports.push(Port {
                port,
                pid,
                process,
                addr,
            });
        }
    }
    ports.sort_by_key(|p| (p.port, p.pid));
    ports
}

// ---------------------------------------------------------------------------
// Project and git
// ---------------------------------------------------------------------------

/// The nearest ancestor of `dir` (itself included) holding `.git`; `dir`
/// when there is none.
pub fn project_root(dir: &Path) -> PathBuf {
    dir.ancestors()
        .find(|d| d.join(".git").exists())
        .unwrap_or(dir)
        .to_path_buf()
}

/// Branch, ahead/behind and changed files. `None` outside a repository or
/// without git.
pub fn git(dir: &Path) -> Option<GitInfo> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain=v2", "--branch", "-z"])
        // A status poll must never take `index.lock` out from under a commit
        // the user is running at the same moment.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut info = parse_git(&String::from_utf8_lossy(&out.stdout));
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    info.worktrees = parse_worktrees(&git(&["worktree", "list", "--porcelain"]), dir);
    info.commits = parse_log(&git(&["log", "-n", "12", "--format=%H%x1f%h%x1f%P%x1f%s%x1f%an%x1f%at%x1f%D%x1e"]));
    Some(info)
}

fn parse_worktrees(out: &str, dir: &Path) -> Vec<Worktree> {
    let mut list: Vec<Worktree> = out
        .split("\n\n")
        .filter_map(|block| {
            let mut w = Worktree::default();
            for line in block.lines() {
                let (k, v) = line.split_once(' ').unwrap_or((line, ""));
                match k {
                    "worktree" => w.path = PathBuf::from(v),
                    "branch" => w.branch = Some(v.strip_prefix("refs/heads/").unwrap_or(v).to_string()),
                    _ => {}
                }
            }
            (!w.path.as_os_str().is_empty()).then_some(w)
        })
        .collect();
    // The current one is the deepest worktree this folder is inside (one
    // worktree can sit inside another's folder).
    let here = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let current = list
        .iter()
        .enumerate()
        .filter_map(|(i, w)| {
            let p = std::fs::canonicalize(&w.path).unwrap_or_else(|_| w.path.clone());
            here.starts_with(&p).then_some((i, p.components().count()))
        })
        .max_by_key(|&(_, depth)| depth)
        .map(|(i, _)| i);
    if let Some(i) = current {
        list[i].current = true;
    }
    list
}

fn parse_log(out: &str) -> Vec<Commit> {
    out.split('\x1e')
        .map(|r| r.trim_start_matches('\n'))
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let f: Vec<&str> = r.split('\x1f').collect();
            Some(Commit {
                hash: f.first()?.to_string(),
                short: f.get(1)?.to_string(),
                merge: f.get(2)?.split_whitespace().count() > 1,
                subject: f.get(3)?.to_string(),
                author: f.get(4)?.to_string(),
                date: f.get(5)?.parse().unwrap_or(0),
                refs: f
                    .get(6)
                    .map(|d| d.split(", ").filter(|x| !x.is_empty()).map(str::to_string).collect())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Porcelain v2 with `-z`: NUL-terminated records, so paths need no
/// unquoting; a rename record is followed by its original path as a record
/// of its own.
fn parse_git(out: &str) -> GitInfo {
    let mut info = GitInfo::default();
    let mut oid = String::new();
    let mut records = out.split('\0').filter(|r| !r.is_empty());
    while let Some(rec) = records.next() {
        if let Some(header) = rec.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" => oid = value.to_string(),
                "branch.head" => info.branch = value.to_string(),
                "branch.upstream" => info.upstream = Some(value.to_string()),
                "branch.ab" => {
                    for part in value.split_whitespace() {
                        if let Some(n) = part.strip_prefix('+') {
                            info.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            info.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let xy = rec.get(2..4).unwrap_or("..");
        let (staged, unstaged) = match rec.as_bytes().first() {
            Some(b'1' | b'2') => (xy.as_bytes()[0] != b'.', xy.as_bytes().get(1) != Some(&b'.')),
            Some(b'u') => (false, true),
            _ => (false, true),
        };
        let file = match rec.as_bytes().first() {
            Some(b'1') => rec
                .splitn(9, ' ')
                .nth(8)
                .map(|path| (path, xy_code(rec.get(2..4).unwrap_or("")))),
            Some(b'2') => {
                // The original path is the next record; consume it.
                records.next();
                rec.splitn(10, ' ')
                    .nth(9)
                    .map(|path| (path, "R".to_string()))
            }
            Some(b'u') => rec
                .splitn(11, ' ')
                .nth(10)
                .map(|path| (path, "U".to_string())),
            Some(b'?') => rec.get(2..).map(|path| (path, "??".to_string())),
            _ => None,
        };
        if let Some((path, status)) = file {
            if info.files.len() < MAX_GIT_FILES {
                info.files.push(GitFile {
                    path: path.to_string(),
                    status,
                    staged,
                    unstaged,
                });
            }
        }
    }
    if info.branch == "(detached)" && oid.len() >= 7 {
        info.branch = format!("detached@{}", &oid[..7]);
    }
    info
}

/// One letter for an `XY` pair (`.` = unchanged): the staged side when it has
/// a change, else the worktree side. A type change reads as a modification.
fn xy_code(xy: &str) -> String {
    let c = xy.chars().find(|&c| c != '.').unwrap_or('M');
    match c {
        'T' => "M".to_string(),
        c => c.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// Directories first, then files, each alphabetical ignoring case.
pub fn list_dir(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == ".DS_Store" {
                return None;
            }
            let path = e.path();
            // `file_type` comes free with the directory read; only symlinks
            // cost a stat, to learn what they point at.
            let is_dir = match e.file_type() {
                Ok(t) if t.is_symlink() => path.is_dir(),
                Ok(t) => t.is_dir(),
                Err(_) => false,
            };
            Some(Entry { name, path, is_dir })
        })
        .collect();
    entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));
    entries.truncate(MAX_ENTRIES);
    entries
}

/// `812 KB`, `3.4 MB`, `1.2 GB` — binary units, as Activity Monitor uses.
pub fn human_bytes(n: u64) -> String {
    const K: f64 = 1024.;
    let f = n as f64;
    if f < K {
        format!("{n} B")
    } else if f < K * K {
        format!("{:.0} KB", f / K)
    } else if f < K * K * K {
        format!("{:.1} MB", f / (K * K))
    } else {
        format!("{:.1} GB", f / (K * K * K))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, ppid: u32) -> Proc {
        Proc {
            pid,
            ppid,
            name: format!("p{pid}"),
            cpu: 0.,
            rss: 0,
        }
    }

    #[test]
    fn tree_orders_parent_first_with_ascending_siblings() {
        let procs = [
            p(1, 0),
            p(100, 1),
            p(300, 100),
            p(200, 100),
            p(201, 200),
            p(202, 201),
            p(999, 1),
        ];
        let order: Vec<u32> = tree_order(&procs, 100).iter().map(|p| p.pid).collect();
        assert_eq!(order, vec![100, 200, 201, 202, 300]);
        assert!(tree_order(&procs, 4242).is_empty());
        // launchd parents itself through pid 0 without looping.
        assert_eq!(tree_order(&[p(1, 1), p(5, 1)], 1).len(), 2);
    }

    #[test]
    fn libproc_struct_sizes_match_the_sdk() {
        // From `sizeof` in a C program built against <sys/proc_info.h>.
        assert_eq!(std::mem::size_of::<ProcTaskInfo>(), 96);
        assert_eq!(std::mem::size_of::<ProcBsdShortInfo>(), 64);
        assert_eq!(std::mem::size_of::<ProcFdInfo>(), 8);
        assert_eq!(std::mem::size_of::<SocketFdInfo>(), 792);
    }

    #[test]
    fn cpu_is_a_rate_between_samples() {
        let t0 = Instant::now();
        let mut a = [(p(4_000_001, 1), Some(1_000_000_000u64))];
        rate_cpu(&mut a, t0);
        assert_eq!(a[0].0.cpu, 0.);
        // 0.5s of CPU over 1s of wall time is half a core.
        let mut b = [(p(4_000_001, 1), Some(1_500_000_000u64))];
        rate_cpu(&mut b, t0 + Duration::from_secs(1));
        assert!((b[0].0.cpu - 50.).abs() < 0.01, "{}", b[0].0.cpu);
        // A counter that went backwards (pid reuse) reads as idle, not huge.
        let mut c = [(p(4_000_001, 1), Some(10u64))];
        rate_cpu(&mut c, t0 + Duration::from_secs(2));
        assert_eq!(c[0].0.cpu, 0.);
    }

    #[test]
    fn live_busy_child_shows_cpu_and_rss() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "while :; do :; done"])
            .spawn()
            .unwrap();
        let me = std::process::id();
        let first = descendants(me);
        assert!(
            first.iter().any(|p| p.pid == child.id()),
            "child missing: {first:?}"
        );
        assert!(first[0].rss > 0);
        std::thread::sleep(Duration::from_millis(300));
        let second = descendants(me);
        let busy = second.iter().find(|p| p.pid == child.id()).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(
            busy.cpu > 50.,
            "a spinning shell should use most of a core: {busy:?}"
        );
        assert_eq!(busy.ppid, me);
        // `/bin/sh` is bash on macOS; what matters is that a name came back.
        assert!(!busy.name.is_empty());
    }

    #[test]
    fn live_listeners_are_found_on_v4_and_v6() {
        let v4 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let v6 = std::net::TcpListener::bind("[::1]:0").unwrap();
        let any = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let ports = listening_ports(&[std::process::id()]);
        let find = |port: u16| ports.iter().find(|p| p.port == port).cloned();
        let a = find(v4.local_addr().unwrap().port()).expect("v4 listener");
        assert_eq!((a.addr.as_str(), a.pid), ("127.0.0.1", std::process::id()));
        assert_eq!(
            find(v6.local_addr().unwrap().port())
                .expect("v6 listener")
                .addr,
            "::1"
        );
        assert_eq!(
            find(any.local_addr().unwrap().port())
                .expect("any listener")
                .addr,
            "*"
        );
        assert!(!a.process.is_empty());
        // A connected (non-listening) socket must not show up.
        let _conn = std::net::TcpStream::connect(v4.local_addr().unwrap()).unwrap();
        assert_eq!(listening_ports(&[std::process::id()]).len(), 3);
        assert!(listening_ports(&[]).is_empty());
    }

    #[test]
    fn git_porcelain_v2_parses_branch_counts_and_files() {
        let out = [
            "# branch.oid 0123456789abcdef0123456789abcdef01234567",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 .M N... 100644 100644 100644 aaaa bbbb src/main.rs",
            "1 A. N... 000000 100644 100644 0000 cccc new file.rs",
            "1 D. N... 100644 000000 000000 dddd 0000 gone.rs",
            "2 R. N... 100644 100644 100644 eeee ffff R100 renamed.rs",
            "old.rs",
            "u UU N... 100644 100644 100644 100644 1111 2222 3333 conflict.rs",
            "? notes.txt",
            "",
        ]
        .join("\0");
        let g = parse_git(&out);
        assert_eq!(g.branch, "main");
        assert_eq!((g.ahead, g.behind), (2, 1));
        let files: Vec<(&str, &str)> = g
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.status.as_str()))
            .collect();
        assert_eq!(
            files,
            vec![
                ("src/main.rs", "M"),
                ("new file.rs", "A"),
                ("gone.rs", "D"),
                ("renamed.rs", "R"),
                ("conflict.rs", "U"),
                ("notes.txt", "??"),
            ]
        );

        let detached =
            "# branch.oid 89abcdef0123456789abcdef0123456789abcdef\0# branch.head (detached)\0";
        assert_eq!(parse_git(detached).branch, "detached@89abcde");
    }

    #[test]
    fn bytes_read_like_activity_monitor() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(812 * 1024), "812 KB");
        assert_eq!(human_bytes(3_565_158), "3.4 MB");
        assert_eq!(human_bytes(548_510_105), "523.1 MB");
        assert_eq!(human_bytes(1_288_490_189), "1.2 GB");
    }

    #[test]
    fn live_process_tree_and_cwd_of_this_test() {
        let me = std::process::id();
        let tree = descendants(me);
        assert_eq!(tree.first().map(|p| p.pid), Some(me));
        assert!(tree.iter().all(|p| p.pid == me || p.ppid != 0));
        assert_eq!(cwd(me), std::env::current_dir().ok());
        assert_eq!(cwd(u32::MAX >> 1), None);
    }

    #[test]
    fn project_root_and_listing() {
        let base = std::env::temp_dir().join(format!("deck-inspect-{}", std::process::id()));
        let nested = base.join("repo/src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(base.join("repo/.git")).unwrap();
        for f in ["b.txt", "A.txt", ".env", ".DS_Store"] {
            std::fs::write(base.join("repo").join(f), "").unwrap();
        }
        std::fs::create_dir_all(base.join("repo/Zdir")).unwrap();

        assert_eq!(project_root(&nested), base.join("repo"));
        assert_eq!(project_root(&base), base);

        let names: Vec<String> = list_dir(&base.join("repo"))
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["src", "Zdir", ".env", "A.txt", "b.txt"]);
        assert!(list_dir(&base.join("missing")).is_empty());

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn procargs_parse_and_our_own_argv_reads() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/opt/claude/versions/2.1.300\0\0\0claude\0--resume\0HOME=/x\0");
        assert_eq!(parse_procargs(&buf), vec!["claude", "--resume"]);
        assert!(parse_procargs(b"\x01").is_empty());
        let me = argv(std::process::id());
        assert!(!me.is_empty() && me[0].contains("deck"), "{me:?}");
        assert!(is_version("2.1.300") && !is_version("zsh") && !is_version("42"));
        assert_eq!(std::mem::size_of::<ProcBsdInfo>(), 136);
        // Our own row reads (the test may have no terminal, so no tpgid).
        let _ = foreground(std::process::id());
    }
}
