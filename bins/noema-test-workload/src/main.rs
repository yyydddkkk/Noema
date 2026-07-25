use std::{
    env,
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use signal_hook::consts::signal::{SIGINT, SIGTERM};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Normal,
    Crash,
    StartupTimeout,
    Unhealthy,
}

impl Mode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "normal" => Ok(Self::Normal),
            "crash" => Ok(Self::Crash),
            "startup-timeout" => Ok(Self::StartupTimeout),
            "unhealthy" => Ok(Self::Unhealthy),
            _ => Err(format!("unsupported mode '{value}'")),
        }
    }
}

struct Configuration {
    mode: Mode,
    port: Option<u16>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("noema-test-workload: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let configuration = parse_arguments()?;
    if configuration.mode == Mode::Crash {
        return Err("intentional startup crash".to_owned());
    }

    let terminate = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGTERM, Arc::clone(&terminate))
        .map_err(|error| format!("register SIGTERM handler: {error}"))?;
    signal_hook::flag::register(SIGINT, Arc::clone(&terminate))
        .map_err(|error| format!("register SIGINT handler: {error}"))?;

    if configuration.mode == Mode::StartupTimeout || configuration.port.is_none() {
        wait_for_termination(&terminate);
        return Ok(());
    }

    let port = configuration.port.expect("port checked above");
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|error| format!("bind HTTP health endpoint on port {port}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("make health listener nonblocking: {error}"))?;
    println!("READY http://127.0.0.1:{port}/health");

    while !terminate.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => respond(stream, configuration.mode)?,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("accept health request: {error}")),
        }
    }
    Ok(())
}

fn wait_for_termination(terminate: &AtomicBool) {
    while !terminate.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(20));
    }
}

fn respond(mut stream: TcpStream, mode: Mode) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|error| format!("set request timeout: {error}"))?;
    let mut request = [0_u8; 1024];
    let bytes = stream
        .read(&mut request)
        .map_err(|error| format!("read health request: {error}"))?;
    let request = String::from_utf8_lossy(&request[..bytes]);
    let healthy = mode == Mode::Normal && request.starts_with("GET /health ");
    let (status, body) = if healthy {
        ("200 OK", "ok\n")
    } else {
        ("503 Service Unavailable", "unhealthy\n")
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("write health response: {error}"))
}

fn parse_arguments() -> Result<Configuration, String> {
    let mut arguments = env::args().skip(1);
    let mut mode = Mode::Normal;
    let mut port = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--mode" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--mode requires a value".to_owned())?;
                mode = Mode::parse(&value)?;
            }
            "--port" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--port requires a value".to_owned())?;
                port = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid port '{value}': {error}"))?,
                );
            }
            _ => return Err(format!("unknown argument '{argument}'")),
        }
    }
    Ok(Configuration { mode, port })
}
