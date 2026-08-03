#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! CheIME Engine Host.
//!
//! The user-level x64 process that hosts all CheIME engine logic.
//! Listens on a named pipe for TIP client connections, loads dictionaries
//! from a configured directory, and runs per-client input sessions.

mod pipe_handle;
mod server;
mod session_runner;

use cheime_config::schema::{EmojiTranslatorConfig, SchemaConfig, TranslatorConfig};
use cheime_dictionary::{CompiledIndex, DictCache, DictColumn};
use cheime_pipeline::learning::LearningService;
use cheime_user_data::UserStore;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut dict_dir: Option<PathBuf> = None;
    let mut pipe_name = server::DEFAULT_PIPE_NAME.to_owned();
    let mut stdin_mode = false;
    let mut config_path: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--pipe" => {
                i += 1;
                if i < args.len() {
                    pipe_name = args[i].clone();
                }
            }
            "--dict-dir" => {
                i += 1;
                if i < args.len() {
                    dict_dir = Some(PathBuf::from(&args[i]));
                }
            }
            "--config" => {
                i += 1;
                if i < args.len() {
                    config_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--stdin" => stdin_mode = true,
            "--help" => {
                print_usage();
                return;
            }
            _ => {}
        }
        i += 1;
    }

    eprintln!("CheIME Engine Host v{}", env!("CARGO_PKG_VERSION"));
    eprintln!("Protocol version: {}", cheime_model::CORE_PROTOCOL_VERSION);

    // ── Load dict (shared between pipe server and stdin mode) ─────────
    let dict_dir = dict_dir.unwrap_or_else(|| data_dir().join("data").join("dicts"));
    eprintln!("Loading dictionaries from: {}", dict_dir.display());
    let index = load_index(&dict_dir);

    if stdin_mode {
        run_stdin_mode(index);
        return;
    }

    // ── Pipe server mode ──────────────────────────────────────────────
    let config_path = config_path.unwrap_or_else(|| {
        data_dir()
            .join("config")
            .join("schemas")
            .join("quanpin.yaml")
    });
    let mut config = load_config(&config_path);
    configure_emoji_translator(
        &mut config,
        &config_path,
        &data_dir().join("data").join("emoji.txt"),
    );

    let db_path = data_dir().join("user_data.db");
    let user_store =
        UserStore::open("engine-host", &db_path).unwrap_or_else(|_| UserStore::new("engine-host"));
    let store = Arc::new(Mutex::new(user_store));
    let learning = Arc::new(LearningService::production(store));

    eprintln!("Starting named pipe server...");
    if let Err(e) = server::run_server(&config, index, learning, &pipe_name) {
        eprintln!("Server error: {e}");
    }
}
// ── Loaders ─────────────────────────────────────────────────────────

fn load_index(dict_dir: &PathBuf) -> Arc<CompiledIndex> {
    if !dict_dir.exists() {
        eprintln!("Dictionary directory not found, using empty index");
        return Arc::new(CompiledIndex::build(
            vec![],
            cheime_model::DeploymentGeneration::new(1),
        ));
    }
    let files: Vec<PathBuf> = std::fs::read_dir(dict_dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    let cache = DictCache::new(data_dir().join("cache"));
    match cache.load_or_build(
        &files,
        "dictionaries",
        &[DictColumn::Text, DictColumn::Code, DictColumn::Weight],
        cheime_model::DeploymentGeneration::new(1),
    ) {
        Ok(idx) => {
            eprintln!("Loaded {} entries", idx.total_entries());
            Arc::new(idx)
        }
        Err(e) => {
            eprintln!("warning: dict cache error: {e}, using empty index");
            Arc::new(CompiledIndex::build(
                vec![],
                cheime_model::DeploymentGeneration::new(1),
            ))
        }
    }
}

fn load_config(config_path: &PathBuf) -> SchemaConfig {
    let fallback = r#"schema_version: 1
engine:
  segmentors:
    - type: pinyin_syllable
  translators:
    - type: table
      dictionary: test_dict
  filters:
    - type: uniquifier
"#;
    let yaml = std::fs::read_to_string(config_path).unwrap_or_else(|_| fallback.to_string());
    let parent_dir = config_path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let loader = cheime_config::ConfigLoader::new().with_base_dir(parent_dir);
    loader.load(&yaml).unwrap_or_else(|e| {
        eprintln!("warning: config load failed ({e}), using minimal config");
        serde_yaml::from_str(fallback).unwrap()
    })
}

fn configure_emoji_translator(
    config: &mut SchemaConfig,
    config_path: &Path,
    default_emoji_path: &Path,
) {
    let mut found = false;
    for translator in &mut config.engine.translators {
        let TranslatorConfig::Emoji(emoji) = translator else {
            continue;
        };
        found = true;
        let configured = PathBuf::from(&emoji.emoji_data);
        if configured.is_absolute() {
            continue;
        }

        let config_relative = config_path
            .parent()
            .map_or_else(|| configured.clone(), |parent| parent.join(&configured));
        let runtime_relative = data_dir().join(&configured);
        let resolved = if config_relative.is_file() {
            config_relative
        } else if runtime_relative.is_file() {
            runtime_relative
        } else if default_emoji_path.is_file() {
            default_emoji_path.to_path_buf()
        } else {
            config_relative
        };
        emoji.emoji_data = resolved.to_string_lossy().into_owned();
    }

    if !found {
        config
            .engine
            .translators
            .push(TranslatorConfig::Emoji(EmojiTranslatorConfig {
                emoji_data: default_emoji_path.to_string_lossy().into_owned(),
            }));
    }
}

// ── stdin mode ─────────────────────────────────────────────────────

fn run_stdin_mode(index: Arc<CompiledIndex>) {
    use cheime_model::{
        CORE_PROTOCOL_VERSION, ClientInstanceId, DeploymentGeneration, Key, KeyEvent, KeyState,
        Revision, Sequence, SessionEpoch, SessionId,
    };
    use cheime_pipeline::factory::PipelineFactory;
    use cheime_protocol::{FrontendMessage, MessageHeader};
    use cheime_session::Session;
    use std::io::{self, BufRead, Write};

    let config: SchemaConfig = serde_yaml::from_str(
        r#"schema_version: 1
engine:
  segmentors:
    - type: pinyin_syllable
  translators:
    - type: table
      dictionary: test_dict
    - type: emoji
      emoji_data: data/emoji.txt
  filters:
    - type: uniquifier
"#,
    )
    .unwrap();

    let store = Arc::new(Mutex::new(UserStore::new("stdin")));
    let pipeline = PipelineFactory::build(&config, Some(store), Some(index), None).unwrap();

    let init_header = MessageHeader {
        protocol_version: CORE_PROTOCOL_VERSION,
        client: ClientInstanceId::new(1),
        session: SessionId::new(1),
        epoch: SessionEpoch::new(1),
        sequence: Sequence::new(0),
        revision: Revision::new(0),
        deployment: DeploymentGeneration::new(1),
    };
    let mut session = Session::new(init_header, pipeline);
    let mut next_seq: u64 = 1;

    eprintln!("stdin mode ready. Type pinyin (one letter per line) and press Enter.");
    let stdin = io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line == "quit" || line == "exit" {
            break;
        }

        let msg: FrontendMessage = match serde_json::from_str(&line) {
            Ok(msg) => msg,
            Err(_) => {
                let event = KeyEvent {
                    key: Key::Character(line.chars().next().unwrap_or('a')),
                    state: KeyState::default(),
                };
                FrontendMessage::KeyCommand {
                    header: MessageHeader {
                        protocol_version: CORE_PROTOCOL_VERSION,
                        client: ClientInstanceId::new(1),
                        session: SessionId::new(1),
                        epoch: SessionEpoch::new(1),
                        sequence: Sequence::new(next_seq),
                        revision: Revision::new(0),
                        deployment: DeploymentGeneration::new(1),
                    },
                    event,
                }
            }
        };
        next_seq = next_seq.saturating_add(1);

        match session.handle(msg) {
            Ok(responses) => {
                for resp in responses {
                    let json = serde_json::to_string(&resp).unwrap();
                    println!("{}", json);
                    let _ = io::stdout().flush();
                }
            }
            Err(e) => eprintln!("session error: {e}"),
        }
    }
}

fn data_dir() -> PathBuf {
    std::env::var("CHEIME_DATA_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
            PathBuf::from(local).join("cheime")
        })
}

fn print_usage() {
    eprintln!("Usage: cheime-engine [OPTIONS]");
    eprintln!("  --pipe NAME   Named pipe name (default: \\\\.\\pipe\\cheime-engine)");
    eprintln!("  --dict-dir DIR Dictionary directory");
    eprintln!("  --config PATH Schema config file");
    eprintln!("  --stdin       Run in stdin/stdout JSON mode");
    eprintln!("  --help        Show this help");
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheime_dictionary::DictEntry;
    use cheime_model::DeploymentGeneration;
    use cheime_pipeline::InputPipeline;
    use cheime_pipeline::factory::PipelineFactory;

    #[test]
    fn missing_schema_translator_gets_runtime_emoji_data() {
        let mut config = SchemaConfig::default();
        let emoji_path = PathBuf::from(r"C:\CheIME\data\emoji.txt");

        configure_emoji_translator(
            &mut config,
            &PathBuf::from(r"C:\CheIME\config\schema.yaml"),
            &emoji_path,
        );

        assert!(config.engine.translators.iter().any(|translator| {
            matches!(
                translator,
                TranslatorConfig::Emoji(emoji) if emoji.emoji_data == emoji_path.to_string_lossy()
            )
        }));
    }

    #[test]
    fn absolute_custom_emoji_path_is_preserved() {
        let custom_path = PathBuf::from(r"D:\custom\emoji.txt");
        let mut config = SchemaConfig::default();
        config
            .engine
            .translators
            .push(TranslatorConfig::Emoji(EmojiTranslatorConfig {
                emoji_data: custom_path.to_string_lossy().into_owned(),
            }));

        configure_emoji_translator(
            &mut config,
            &PathBuf::from(r"C:\CheIME\config\schema.yaml"),
            &PathBuf::from(r"C:\CheIME\data\emoji.txt"),
        );

        assert!(matches!(
            &config.engine.translators[0],
            TranslatorConfig::Emoji(emoji) if emoji.emoji_data == custom_path.to_string_lossy()
        ));
    }

    #[test]
    fn repository_emoji_data_is_loaded_and_ranked_fifth() {
        let emoji_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("data")
            .join("emoji.txt");
        assert!(emoji_path.is_file());
        let yaml_path = emoji_path.to_string_lossy().replace('\\', "/");
        let config: SchemaConfig = serde_yaml::from_str(&format!(
            r#"schema_version: 1
engine:
  segmentors:
    - type: pinyin_syllable
  translators:
    - type: table
      dictionary: fixture
    - type: emoji
      emoji_data: '{yaml_path}'
"#
        ))
        .unwrap();
        let entries = (1..=8)
            .map(|index| DictEntry {
                text: format!("word-{index}"),
                code: String::from("xiao"),
                weight: Some(100 - index),
                stem: None,
            })
            .collect();
        let dictionary = Arc::new(CompiledIndex::build(entries, DeploymentGeneration::new(1)));
        let pipeline = PipelineFactory::build(&config, None, Some(dictionary), None).unwrap();

        let candidates = pipeline.refresh("xiao").unwrap();

        assert!(candidates[4].is_emoji);
        assert_eq!(candidates[4].text, "😀");
    }
}
