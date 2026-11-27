//! `agenttable` — starts the server.
//!
//! ```text
//! agenttable [--port 8080] [--host 127.0.0.1] [--database URL] [--import FILE.json]
//!            [--data FILE.json] [--fresh] [--demo] [--open]
//! ```
//!
//! The workspace is kept in a database: by default SQLite files in `agenttable-data/` next to
//! where the server starts; `--database mysql://…` or `--database postgres://…` for a server.
//! A new workspace is empty (one administrator), or with `--demo` the purchasing sample;
//! `--import` puts a workspace saved as JSON into a new database. `--data` keeps the workspace
//! in a JSON file instead (as the first builds did). `--fresh` starts over; `--open` opens
//! the web app in the default browser.

use mt_server::api::Clock;
use std::net::SocketAddr;
use std::path::PathBuf;

const USAGE: &str = "usage: agenttable [--port 8080] [--host 127.0.0.1] [--database sqlite://agenttable-data | mysql://… | postgres://…] [--import FILE.json] [--data FILE.json] [--fresh] [--demo] [--open]";

struct Options {
    host: String,
    port: u16,
    database: String,
    import: Option<PathBuf>,
    data: Option<PathBuf>,
    fresh: bool,
    demo: bool,
    open: bool,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        host: "127.0.0.1".to_owned(),
        port: 8080,
        database: "sqlite://agenttable-data".to_owned(),
        import: None,
        data: None,
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
            "--database" => options.database = args.next().ok_or("--database needs a URL")?,
            "--import" => {
                options.import = Some(PathBuf::from(args.next().ok_or("--import needs a file")?));
            }
            "--data" => {
                options.data = Some(PathBuf::from(args.next().ok_or("--data needs a file")?))
            }
            "--fresh" => options.fresh = true,
            "--demo" => options.demo = true,
            "--open" => options.open = true,
            "--help" | "-h" => return Err(USAGE.to_owned()),
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

/// A database URL without its password, for messages.
fn redact(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((credentials, host)) => {
                let user = credentials.split(':').next().unwrap_or_default();
                format!("{scheme}://{user}@{host}")
            }
            None => url.to_owned(),
        },
        None => url.to_owned(),
    }
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
    let start = if options.demo {
        mt_server::Start::Demo
    } else {
        mt_server::Start::Empty
    };
    let (app, kept_in) = match &options.data {
        Some(file) => {
            if options.fresh && file.exists() {
                if let Err(error) = std::fs::remove_file(file) {
                    eprintln!("cannot remove {}: {error}", file.display());
                    return std::process::ExitCode::FAILURE;
                }
            }
            match mt_server::open(Some(file.clone()), Clock::System, start) {
                Ok(app) => {
                    if let Err(error) = app.save(&*app.ws.lock().await).await {
                        eprintln!("cannot write {}: {error}", file.display());
                        return std::process::ExitCode::FAILURE;
                    }
                    (app, format!("the file {}", file.display()))
                }
                Err(error) => {
                    eprintln!("cannot open the workspace: {error}");
                    return std::process::ExitCode::FAILURE;
                }
            }
        }
        None => {
            let import = match &options.import {
                Some(file) => match std::fs::read(file)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
                {
                    Ok(data) => Some(data),
                    Err(error) => {
                        eprintln!("cannot read {}: {error}", file.display());
                        return std::process::ExitCode::FAILURE;
                    }
                },
                None => None,
            };
            // The data file of an earlier version, next to the program, moves into a new
            // database by itself.
            let earlier = PathBuf::from("agenttable-data.json");
            let carried = import.is_none() && !options.fresh && earlier.exists();
            let import = if carried {
                std::fs::read(&earlier)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            } else {
                import
            };
            let opening = mt_server::persist::Opening {
                config: mt_store::StoreConfig::new(options.database.clone()),
                fresh: options.fresh,
                import,
                import_if_new: carried,
            };
            match mt_server::open_database(opening, Clock::System, start).await {
                Ok(app) => {
                    let engine = app.persist.engine().unwrap_or("a database");
                    (app, format!("{engine} ({})", redact(&options.database)))
                }
                Err(error) => {
                    eprintln!(
                        "cannot open the workspace in {}: {error}",
                        redact(&options.database)
                    );
                    return std::process::ExitCode::FAILURE;
                }
            }
        }
    };
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
    println!("Open http://{address} in your browser. The workspace is kept in {kept_in}.");
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
