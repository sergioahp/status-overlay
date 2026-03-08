use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::mpsc::Sender,
};

#[derive(Debug, Clone, Copy)]
pub enum Command {
    Show,
    Hide,
    Toggle,
    Quit,
}

pub fn sock_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    PathBuf::from(runtime_dir).join("status-overlay.sock")
}

/// Client: send a command to a running daemon. Returns the response line.
pub fn send(cmd: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(sock_path())?;
    writeln!(stream, "{cmd}")?;
    let mut resp = String::new();
    BufReader::new(stream).read_line(&mut resp)?;
    Ok(resp.trim().to_string())
}

/// Daemon: listen on the socket and forward commands to the GTK thread via `tx`.
pub fn listen(tx: Sender<Command>) {
    let path = sock_path();
    let _ = std::fs::remove_file(&path); // clean up stale socket

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("IPC: failed to bind socket: {e}");
            return;
        }
    };

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle(stream, &tx),
            Err(e) => eprintln!("IPC: accept error: {e}"),
        }
    }
}

fn handle(mut stream: UnixStream, tx: &Sender<Command>) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }

    let (cmd, response) = match line.trim() {
        "show"   => (Some(Command::Show),   "OK shown"),
        "hide"   => (Some(Command::Hide),   "OK hidden"),
        "toggle" => (Some(Command::Toggle), "OK toggled"),
        "quit"   => (Some(Command::Quit),   "OK quitting"),
        other    => {
            let _ = writeln!(stream, "ERR unknown: {other}");
            return;
        }
    };

    let _ = writeln!(stream, "{response}");

    if let Some(cmd) = cmd {
        let _ = tx.send(cmd);
    }
}
