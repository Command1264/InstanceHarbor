//! Test-only stand-in for the Java subprocess. Each instance has its own gate.
use std::io::{self, BufRead, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::path::Path;
use std::time::{Duration, Instant};

#[cfg(windows)]
fn console_grid() -> io::Result<(i16, i16)> {
    #[repr(C)]
    #[derive(Default)]
    struct BufferInfo {
        size: [i16; 2],
        cursor: [i16; 2],
        attributes: u16,
        window: [i16; 4],
        maximum: [i16; 2],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(kind: u32) -> *mut std::ffi::c_void;
        fn GetConsoleScreenBufferInfo(handle: *mut std::ffi::c_void, info: *mut BufferInfo) -> i32;
    }

    let mut info = BufferInfo::default();
    if unsafe { GetConsoleScreenBufferInfo(GetStdHandle(-11i32 as u32), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let [left, top, right, bottom] = info.window;
    Ok((right - left + 1, bottom - top + 1))
}
fn main() -> io::Result<()> {
    if std::env::args().any(|argument| argument == "-version") {
        return Ok(());
    }
    let port: u16 = std::fs::read_to_string("server.properties")?
        .lines()
        .find_map(|line| line.strip_prefix("server-port=")?.parse().ok())
        .ok_or_else(|| io::Error::other("Fixture port missing"))?;
    let _listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    std::fs::write("fixture-started", std::process::id().to_string())?;
    let deadline = Instant::now() + Duration::from_secs(120);
    while !Path::new("fixture-ready").is_file() {
        if Instant::now() > deadline {
            return Err(io::Error::other("Fixture ready gate timed out"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if Path::new("fixture-native-console").is_file() {
        std::thread::spawn(|| {
            while !Path::new("fixture-console-output").is_file() {
                std::thread::sleep(Duration::from_millis(40));
            }
            println!("IH-HIDDEN-01");
            println!("IH-LONG-START{}IH-LONG-END", "0123456789abcdef".repeat(24));
            println!("IH-HIDDEN-02");
        });
    }
    println!("[Server thread/INFO]: Done (0.1s)! For help");
    io::stdout().flush()?;
    if let Ok(code) = std::fs::read_to_string("fixture-exit-code") {
        std::thread::sleep(Duration::from_millis(300));
        let code: i32 = code.trim().parse().map_err(io::Error::other)?;
        std::process::exit(code);
    }
    for line in io::stdin().lock().lines() {
        let line = line?;
        let command = line.trim_matches(|ch: char| ch.is_ascii_control());
        if command == "test-ping" {
            println!("IH-PONG");
        } else if command == "test-viewport" {
            #[cfg(windows)]
            if let Ok((cols, rows)) = console_grid() {
                println!("IH-VIEWPORT|{cols}x{rows}|END");
            }
        } else if command == "stop" {
            std::fs::write("fixture-stop-seen", std::process::id().to_string())?;
            while Path::new("fixture-hold-stop").is_file() {
                std::thread::sleep(Duration::from_millis(25));
            }
            return Ok(());
        }
    }
    Ok(())
}
