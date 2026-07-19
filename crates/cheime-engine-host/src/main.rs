//! CheIME Engine Host.
//!
//! The user-level x64 process that hosts all CheIME engine logic.
//! Listens on a named pipe for TIP client connections, loads dictionaries
//! from a configured directory, and runs per-client input sessions.

mod pipe_handle;
mod server;
mod session_runner;

use cheime_pipeline::DictPipeline;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut dict_dir: Option<PathBuf> = None;
    let mut pipe_name = server::DEFAULT_PIPE_NAME.to_owned();
    let mut stdin_mode = false;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--dict-dir" => {
                i += 1;
                if i < args.len() {
                    dict_dir = Some(PathBuf::from(&args[i]));
                } else {
                    eprintln!("error: --dict-dir requires a path argument");
                    std::process::exit(1);
                }
            }
            "--pipe-name" => {
                i += 1;
                if i < args.len() {
                    pipe_name = args[i].clone();
                } else {
                    eprintln!("error: --pipe-name requires a name argument");
                    std::process::exit(1);
                }
            }
            "--stdin" => {
                stdin_mode = true;
            }
            "--help" | "-h" => {
                print_usage();
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_usage();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    eprintln!("CheIME Engine Host v{}", env!("CARGO_PKG_VERSION"));
    eprintln!("Protocol version: {}", cheime_model::CORE_PROTOCOL_VERSION);

    if stdin_mode {
        run_stdin_mode();
        return;
    }

    // Load dictionaries
    let dict_dir = dict_dir.unwrap_or_else(|| {
        // Default: look for data/dicts relative to current dir or exe dir
        PathBuf::from("data").join("dicts")
    });

    eprintln!("Loading dictionaries from: {}", dict_dir.display());

    let deployment_gen = 1u64;
    let dict_handle = match cheime_dictionary::load_dict_dir(&dict_dir, deployment_gen) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error loading dictionaries: {e}");
            eprintln!();
            eprintln!("Expected .dict.yaml files in: {}", dict_dir.display());
            eprintln!("Format: YAML header followed by --- then tab-separated entries");
            eprintln!("Example:");
            eprintln!("  name: mydict");
            eprintln!("  columns: [text, code, weight]");
            eprintln!("  ---");
            eprintln!("  你\tn\t100");
            std::process::exit(1);
        }
    };

    let generation = dict_handle.generation();
    let total = dict_handle.index().total_entries;
    eprintln!("Loaded {total} dictionary entries (generation {})", generation.get());

    let index = Arc::new(dict_handle.index().clone());
    let pipeline = DictPipeline::new(index);

    eprintln!("Starting named pipe server...");
    if let Err(e) = server::run_server(pipeline, generation, &pipe_name) {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}

/// Run in stdin/stdout mode for testing without named pipes.
/// Reads JSON-encoded `FrontendMessage` lines from stdin,
/// writes JSON-encoded `EngineMessage` lines to stdout.
fn run_stdin_mode() {
    use std::io::{BufRead, BufReader, Write};

    eprintln!("Running in stdin test mode — send JSON FrontendMessage per line");

    // Load a minimal in-memory pipeline for testing
    let entries: Vec<(String, String, i64)> = vec![
        ("ni".into(), "你".into(), 100),
        ("ni".into(), "呢".into(), 50),
        ("hao".into(), "好".into(), 80),
        ("nihao".into(), "你好".into(), 120),
    ];
    let pipeline = cheime_pipeline::BuiltinPipeline::new(entries);

    let identity = cheime_protocol::MessageHeader {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        client: cheime_model::ClientInstanceId::new(1),
        session: cheime_model::SessionId::new(1),
        epoch: cheime_model::SessionEpoch::new(1),
        sequence: cheime_model::Sequence::new(0),
        revision: cheime_model::Revision::new(0),
        deployment: cheime_model::DeploymentGeneration::new(1),
    };

    let mut session = cheime_session::Session::new(identity, pipeline);
    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout().lock();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let msg: cheime_protocol::FrontendMessage = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                let _ = writeln!(stdout, r#"{{"error":"parse: {e}"}}"#);
                let _ = stdout.flush();
                continue;
            }
        };

        match session.handle(msg) {
            Ok(outputs) => {
                for out in outputs {
                    let serialized = serde_json::to_string(&out).unwrap();
                    let _ = writeln!(stdout, "{serialized}");
                }
            }
            Err(e) => {
                let _ = writeln!(stdout, r#"{{"error":"session: {e}"}}"#);
            }
        }
        let _ = stdout.flush();
    }
}

fn print_usage() {
    eprintln!("Usage: cheime-engine [OPTIONS]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --dict-dir <PATH>   Directory containing .dict.yaml files");
    eprintln!("  --pipe-name <NAME>  Named pipe path (default: \\\\.\\pipe\\cheime-engine)");
    eprintln!("  --stdin             Run in stdin/stdout JSON test mode");
    eprintln!("  --help, -h          Show this help");
}
