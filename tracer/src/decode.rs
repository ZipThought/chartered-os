// Decode syscall arguments by reading the agent process's memory via
// process_vm_readv. Returns a JSON object body (including braces) ready
// to drop into the log line as the value of "args".
//
// Decoding is best-effort. Unreadable pointers (unmapped, raced, denied)
// produce `null`. Unknown syscalls fall back to raw register values.

use std::convert::TryInto;

const PATH_MAX: usize = 4096;
const MAX_ARGV_ENTRIES: usize = 256;
const MAX_SOCKADDR: usize = 128;

pub fn decode(pid: u32, syscall: &str, args: &[u64; 6]) -> String {
    let pid = pid as i32;
    match syscall {
        "execve" => decode_execve(pid, args),
        "execveat" => decode_execveat(pid, args),
        "openat" => decode_openat(pid, args),
        "connect" => decode_connect(pid, args),
        "unlinkat" => decode_unlinkat(pid, args),
        "renameat2" => decode_renameat2(pid, args),
        _ => decode_raw(args),
    }
}

fn decode_execve(pid: i32, args: &[u64; 6]) -> String {
    let pathname = json_string_or_null(read_cstr(pid, args[0], PATH_MAX));
    let argv = match read_argv(pid, args[1], MAX_ARGV_ENTRIES) {
        Some(v) => json_array(&v),
        None => "null".to_string(),
    };
    let envc = match count_pointers(pid, args[2], MAX_ARGV_ENTRIES) {
        Some(n) => n.to_string(),
        None => "null".to_string(),
    };
    format!("{{\"pathname\":{pathname},\"argv\":{argv},\"envc\":{envc}}}")
}

fn decode_execveat(pid: i32, args: &[u64; 6]) -> String {
    let dirfd = format_dirfd(args[0]);
    let pathname = json_string_or_null(read_cstr(pid, args[1], PATH_MAX));
    let argv = match read_argv(pid, args[2], MAX_ARGV_ENTRIES) {
        Some(v) => json_array(&v),
        None => "null".to_string(),
    };
    let envc = match count_pointers(pid, args[3], MAX_ARGV_ENTRIES) {
        Some(n) => n.to_string(),
        None => "null".to_string(),
    };
    let flags = args[4] as i32;
    format!(
        "{{\"dirfd\":{dirfd},\"pathname\":{pathname},\"argv\":{argv},\"envc\":{envc},\"flags\":{flags}}}"
    )
}

fn decode_openat(pid: i32, args: &[u64; 6]) -> String {
    let dirfd = format_dirfd(args[0]);
    let pathname = json_string_or_null(read_cstr(pid, args[1], PATH_MAX));
    let flags = args[2] as i32;
    let mode = args[3] as u32;
    format!("{{\"dirfd\":{dirfd},\"pathname\":{pathname},\"flags\":{flags},\"mode\":{mode}}}")
}

fn decode_connect(pid: i32, args: &[u64; 6]) -> String {
    let fd = args[0] as i32;
    let addrlen = args[2];
    let addr = json_string_or_null(decode_sockaddr(pid, args[1], addrlen));
    format!("{{\"fd\":{fd},\"addr\":{addr},\"addrlen\":{addrlen}}}")
}

fn decode_unlinkat(pid: i32, args: &[u64; 6]) -> String {
    let dirfd = format_dirfd(args[0]);
    let pathname = json_string_or_null(read_cstr(pid, args[1], PATH_MAX));
    let flags = args[2] as i32;
    format!("{{\"dirfd\":{dirfd},\"pathname\":{pathname},\"flags\":{flags}}}")
}

fn decode_renameat2(pid: i32, args: &[u64; 6]) -> String {
    let olddirfd = format_dirfd(args[0]);
    let oldpath = json_string_or_null(read_cstr(pid, args[1], PATH_MAX));
    let newdirfd = format_dirfd(args[2]);
    let newpath = json_string_or_null(read_cstr(pid, args[3], PATH_MAX));
    let flags = args[4] as u32;
    format!(
        "{{\"olddirfd\":{olddirfd},\"oldpath\":{oldpath},\"newdirfd\":{newdirfd},\"newpath\":{newpath},\"flags\":{flags}}}"
    )
}

fn decode_raw(args: &[u64; 6]) -> String {
    format!(
        "{{\"a0\":{},\"a1\":{},\"a2\":{},\"a3\":{},\"a4\":{},\"a5\":{}}}",
        args[0], args[1], args[2], args[3], args[4], args[5]
    )
}

fn format_dirfd(v: u64) -> String {
    let i = v as i32;
    if i == libc::AT_FDCWD {
        "\"AT_FDCWD\"".to_string()
    } else {
        i.to_string()
    }
}

// Read up to `len` bytes from the remote process. process_vm_readv may fail
// with EFAULT if the requested region crosses an unmapped page; on failure
// we retry with the prefix that fits in the current page.
fn read_remote(pid: i32, addr: u64, len: usize) -> Option<Vec<u8>> {
    if addr == 0 || len == 0 {
        return None;
    }
    let n = read_remote_once(pid, addr, len);
    if let Some(buf) = n {
        return Some(buf);
    }
    // Retry up to next page boundary.
    let page = 4096usize;
    let to_boundary = page - (addr as usize % page);
    if to_boundary == 0 || to_boundary >= len {
        return None;
    }
    read_remote_once(pid, addr, to_boundary)
}

fn read_remote_once(pid: i32, addr: u64, len: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; len];
    let local = libc::iovec {
        iov_base: buf.as_mut_ptr() as *mut libc::c_void,
        iov_len: len,
    };
    let remote = libc::iovec {
        iov_base: addr as *mut libc::c_void,
        iov_len: len,
    };
    let n = unsafe { libc::process_vm_readv(pid, &local, 1, &remote, 1, 0) };
    if n < 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(buf)
}

fn read_cstr(pid: i32, addr: u64, max: usize) -> Option<String> {
    let buf = read_remote(pid, addr, max)?;
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..end]).into_owned())
}

fn read_pointer(pid: i32, addr: u64) -> Option<u64> {
    let bytes = read_remote(pid, addr, 8)?;
    if bytes.len() < 8 {
        return None;
    }
    let arr: [u8; 8] = bytes.as_slice().try_into().ok()?;
    Some(u64::from_ne_bytes(arr))
}

fn read_argv(pid: i32, addr: u64, max_entries: usize) -> Option<Vec<String>> {
    if addr == 0 {
        return None;
    }
    let mut out = Vec::new();
    for i in 0..max_entries {
        let ptr = read_pointer(pid, addr + (i as u64) * 8)?;
        if ptr == 0 {
            return Some(out);
        }
        match read_cstr(pid, ptr, PATH_MAX) {
            Some(s) => out.push(s),
            None => out.push("<unreadable>".to_string()),
        }
    }
    Some(out)
}

fn count_pointers(pid: i32, addr: u64, max_entries: usize) -> Option<usize> {
    if addr == 0 {
        return Some(0);
    }
    for i in 0..max_entries {
        let ptr = read_pointer(pid, addr + (i as u64) * 8)?;
        if ptr == 0 {
            return Some(i);
        }
    }
    Some(max_entries)
}

fn decode_sockaddr(pid: i32, addr: u64, len: u64) -> Option<String> {
    let want = (len as usize).min(MAX_SOCKADDR);
    if want < 2 {
        return None;
    }
    let buf = read_remote(pid, addr, want)?;
    if buf.len() < 2 {
        return None;
    }
    let family = u16::from_ne_bytes([buf[0], buf[1]]) as i32;
    match family {
        libc::AF_INET => {
            if buf.len() < 8 {
                return None;
            }
            let port = u16::from_be_bytes([buf[2], buf[3]]);
            Some(format!(
                "{}.{}.{}.{}:{} (AF_INET)",
                buf[4], buf[5], buf[6], buf[7], port
            ))
        }
        libc::AF_INET6 => {
            if buf.len() < 28 {
                return None;
            }
            let port = u16::from_be_bytes([buf[2], buf[3]]);
            let groups: Vec<String> = (0..8)
                .map(|i| {
                    let h = u16::from_be_bytes([buf[8 + i * 2], buf[8 + i * 2 + 1]]);
                    format!("{:x}", h)
                })
                .collect();
            Some(format!("[{}]:{} (AF_INET6)", groups.join(":"), port))
        }
        libc::AF_UNIX => {
            let path_bytes = &buf[2..];
            let end = path_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(path_bytes.len());
            let path = String::from_utf8_lossy(&path_bytes[..end]).into_owned();
            Some(format!("{path} (AF_UNIX)"))
        }
        _ => Some(format!("family={family}")),
    }
}

fn json_string_or_null(s: Option<String>) -> String {
    match s {
        Some(s) => json_str(&s),
        None => "null".to_string(),
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_array(items: &[String]) -> String {
    let mut s = String::from("[");
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&json_str(it));
    }
    s.push(']');
    s
}
