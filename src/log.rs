use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Local;

static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Initialize the log file path. Call once at startup.
pub fn init(path: PathBuf) {
    if let Ok(mut guard) = LOG_PATH.lock() {
        *guard = Some(path);
    }
}

fn write_entry(level: &str, msg: &str) {
    let path = match LOG_PATH.lock().ok().and_then(|g| g.clone()) {
        Some(p) => p,
        None => return,
    };
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("[{}] {} {}\n", timestamp, level, msg);
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

pub fn info(msg: &str) {
    write_entry("INFO ", msg);
}

pub fn warn(msg: &str) {
    write_entry("WARN ", msg);
}

pub fn error(msg: &str) {
    write_entry("ERROR", msg);
}

/// Log a panic. Called from the panic hook.
pub fn panic(info: &std::panic::PanicHookInfo<'_>) {
    let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    };
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown location".to_string());
    write_entry("PANIC", &format!("{} at {}", msg, location));
}
