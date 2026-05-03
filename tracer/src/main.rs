// chartered-trace: standalone syscall-trace tool that intercepts an agent
// process's syscalls and writes each one to a log. Passthrough only —
// observe and continue, no policy, no daemon, no protobuf receipts.
// Companion to CharteredOS, not the architectural Runtime; the Runtime is
// the per-deployment process that hosts the Actor loop, runs the Gate,
// dispatches Tools, and writes Receipts (spec §The Runtime).
//
// Usage: chartered-trace <cmd> [args...]
//   CHARTERED_LOG=path overrides the default log path (./chartered.log).

mod decode;

use std::env;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{IoSlice, IoSliceMut, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

use libseccomp::{
    ScmpAction, ScmpArch, ScmpFilterContext, ScmpNotifReq, ScmpNotifResp, ScmpNotifRespFlags,
    ScmpSyscall,
};
use nix::cmsg_space;
use nix::sys::socket::{
    recvmsg, sendmsg, socketpair, AddressFamily, ControlMessage, ControlMessageOwned,
    MsgFlags, SockFlag, SockType,
};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{execvp, fork, ForkResult, Pid};

const INTERCEPT: &[&str] = &[
    "execve", "execveat", "openat", "connect", "unlinkat", "renameat2",
];

fn main() {
    let argv: Vec<String> = env::args().collect();
    if argv.len() < 2 {
        eprintln!("usage: chartered-trace <cmd> [args...]");
        exit(64);
    }
    let cmd_argv: Vec<CString> = argv[1..]
        .iter()
        .map(|s| CString::new(s.as_str()).expect("argv contains NUL"))
        .collect();

    let log_path = env::var("CHARTERED_LOG").unwrap_or_else(|_| "chartered.log".to_string());

    let (parent_sock, child_sock) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .expect("socketpair");

    match unsafe { fork() }.expect("fork") {
        ForkResult::Parent { child } => {
            drop(child_sock);
            run_supervisor(parent_sock, child, &log_path);
        }
        ForkResult::Child => {
            drop(parent_sock);
            run_child(child_sock, &cmd_argv);
        }
    }
}

fn run_child(sock: OwnedFd, argv: &[CString]) -> ! {
    // PR_SET_NO_NEW_PRIVS is required for an unprivileged process to install
    // a seccomp filter, and prevents descendants from gaining privileges
    // through suid that would let them shed the filter.
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            eprintln!("chartered-trace: PR_SET_NO_NEW_PRIVS failed");
            exit(1);
        }
    }

    let mut filter = ScmpFilterContext::new(ScmpAction::Allow).expect("seccomp filter");
    filter.add_arch(ScmpArch::Native).ok();
    for name in INTERCEPT {
        if let Ok(sc) = ScmpSyscall::from_name(name) {
            filter
                .add_rule(ScmpAction::Notify, sc)
                .expect("add_rule");
        }
    }
    filter.load().expect("seccomp load");

    let raw_fd: RawFd = filter.get_notify_fd().expect("notify_fd");
    let notify_fd: OwnedFd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

    // Send the notify_fd to the supervisor over the socketpair.
    let buf = [0u8];
    let iov = [IoSlice::new(&buf)];
    let cmsg = [ControlMessage::ScmRights(&[notify_fd.as_raw_fd()])];
    sendmsg::<()>(sock.as_raw_fd(), &iov, &cmsg, MsgFlags::empty(), None).expect("sendmsg");

    drop(notify_fd);
    drop(sock);

    let prog = &argv[0];
    let _ = execvp(prog, argv);
    eprintln!("chartered-trace: execvp({:?}) failed", prog);
    exit(127);
}

fn run_supervisor(sock: OwnedFd, child: Pid, log_path: &str) -> ! {
    let mut buf = [0u8; 1];
    let mut iov = [IoSliceMut::new(&mut buf)];
    let mut cmsg_buf = cmsg_space!([RawFd; 1]);
    let msg = recvmsg::<()>(
        sock.as_raw_fd(),
        &mut iov,
        Some(&mut cmsg_buf),
        MsgFlags::empty(),
    )
    .expect("recvmsg");

    let mut notify_fd: Option<RawFd> = None;
    for cm in msg.cmsgs().expect("cmsgs") {
        if let ControlMessageOwned::ScmRights(fds) = cm
            && let Some(&fd) = fds.first()
        {
            notify_fd = Some(fd);
        }
    }
    let notify_fd = notify_fd.expect("supervisor did not receive notify_fd");

    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("open log");

    // Drain notifications on a thread; the main thread waits for the child.
    let log_thread = std::thread::spawn(move || {
        let mut log = log;
        loop {
            if poll_one(notify_fd, &mut log).is_err() {
                break;
            }
        }
    });

    let status = waitpid(child, None).expect("waitpid");
    // The child exiting closes the kernel's listener; the next ScmpNotifReq::receive
    // returns an error and the polling thread exits.
    let _ = log_thread.join();

    match status {
        WaitStatus::Exited(_, code) => exit(code),
        WaitStatus::Signaled(_, sig, _) => exit(128 + sig as i32),
        _ => exit(1),
    }
}

fn poll_one(notify_fd: RawFd, log: &mut File) -> Result<(), ()> {
    let req = ScmpNotifReq::receive(notify_fd).map_err(|_| ())?;
    let syscall_name = req
        .data
        .syscall
        .get_name()
        .unwrap_or_else(|_| format!("syscall_{}", i32::from(req.data.syscall)));
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let args_json = decode::decode(req.pid, &syscall_name, &req.data.args);
    let line = format!(
        "{{\"ts_ms\":{ts},\"pid\":{pid},\"syscall\":\"{syscall_name}\",\"args\":{args_json}}}\n",
        pid = req.pid,
    );
    let _ = log.write_all(line.as_bytes());
    let _ = log.flush();

    // CONTINUE: kernel re-runs the syscall normally. Acceptable for
    // passthrough logging; not safe for enforcement (TOCTOU on argv).
    let resp = ScmpNotifResp::new(req.id, 0, 0, ScmpNotifRespFlags::CONTINUE.bits());
    resp.respond(notify_fd).map_err(|_| ())?;
    Ok(())
}
