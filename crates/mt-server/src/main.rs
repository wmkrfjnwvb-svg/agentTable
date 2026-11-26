//! `agenttable` — starts the server.
//!
//! ```text
//! agenttable [--port 8080] [--host 127.0.0.1] [--data agenttable-data.json] [--fresh] [--demo] [--open]
//! ```
//!
//! The data file is created on first start: an empty workspace with one administrator, or
//! with `--demo` the purchasing sample. `--fresh` starts over; `--open` opens the web app in
//! the default browser.

use mt_server::api::Clock;
use std::net::SocketAddr;
use std::path::PathBuf;

struct Options {
    host: String,
    port: u16,
    data: PathBuf,
    fresh: bool,
    demo: bool,
    open: bool,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        host: "127.0.0.1".to_owned(),
        port: 8080,
        data: PathBuf::from("agenttable-data.json"),
        fresh: false,
        demo: false,
        open: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                let value = args.next().ok_or("--port needs a number")?;
                options.port = value
                    .parse()
                    .map_err(|_| format!("`{value}` is not a port"))?;
            }
            "--host" => options.host = args.next().ok_or("--host needs an address")?,
            "--data" => options.data = PathBuf::from(args.next().ok_or("--data needs a file")?),
            "--fresh" => options.fresh = true,
            "--demo" => options.demo = true,
            "--open" => options.open = true,
            "--help" | "-h" => {
                return Err("usage: agenttable [--port 8080] [--host 127.0.0.1] [--data agenttable-data.json] [--fresh] [--demo] [--open]".to_owned());
            }
            other => return Err(format!("unknown option `{other}` (try --help)")),
        }
    }
    Ok(options)
}

/// Opens the web app in the default browser; prints the address if that is not possible.
fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if result.is_err() {
        println!("Could not open a browser; please open {url} yourself.");
    }
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let options = match options() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::from(2);
        }
    };
    if options.fresh && options.data.exists() {
        if let Err(error) = std::fs::remove_file(&options.data) {
            eprintln!("cannot remove {}: {error}", options.data.display());
            return std::process::ExitCode::FAILURE;
        }
    }
    let app = match mt_server::open(
        Some(options.data.clone()),
        Clock::System,
        if options.demo {
            mt_server::Start::Demo
        } else {
            mt_server::Start::Empty
        },
    ) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("cannot open the workspace: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    {
        let ws = app.ws.lock().await;
        if let Err(error) = std::fs::write(
            &options.data,
            serde_json::to_vec_pretty(&ws.data).unwrap_or_default(),
        ) {
            eprintln!("cannot write {}: {error}", options.data.display());
            return std::process::ExitCode::FAILURE;
        }
    }
    // Overdue approval tasks are escalated or reported once a minute.
    {
        let app = app.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                every.tick().await;
                if let Err(error) = app.tick().await {
                    eprintln!("checking overdue approvals: {error}");
                }
            }
        });
    }
    let address: SocketAddr = match format!("{}:{}", options.host, options.port).parse() {
        Ok(address) => address,
        Err(_) => {
            eprintln!("`{}:{}` is not an address", options.host, options.port);
            return std::process::ExitCode::from(2);
        }
    };
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("cannot listen on {address}: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    println!("agenttable {} is running.", env!("CARGO_PKG_VERSION"));
    println!(
        "Open http://{address} in your browser. Data file: {}",
        options.data.display()
    );
    println!("Sign in as any of the demo people with the switcher at the top right.");
    println!("Press Ctrl+C to stop.");
    if options.open {
        open_browser(&format!("http://{address}"));
    }
    match axum::serve(listener, mt_server::router(app))
        .with_graceful_shutdown(shutdown())
        .await
    {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("server error: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
