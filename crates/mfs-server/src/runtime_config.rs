use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::http::AppConfig;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub app: AppConfig,
    pub profile: String,
    pub bind_addr: String,
    pub auth_mode: Option<String>,
    pub api_key: Option<String>,
    pub api_key_file: Option<PathBuf>,
    pub api_key_command: Option<String>,
    pub allow_insecure_bind: Option<bool>,
    pub shutdown_timeout_ms: Option<u64>,
    pub providers: ProviderSettings,
    pub http: HttpSettings,
    pub resilience: ResilienceSettings,
    pub print_config: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderSettings {
    pub summary_provider: Option<String>,
    pub embedding_provider: Option<String>,
    pub rerank_provider: Option<String>,
    pub openai_api_base: Option<String>,
    pub openai_api_key: Option<String>,
    pub summary_model: Option<String>,
    pub chat_model: Option<String>,
    pub embedding_model: Option<String>,
    pub summary_concurrency: Option<u64>,
    pub jina_api_key: Option<String>,
    pub jina_base_url: Option<String>,
    pub jina_embedding_model: Option<String>,
    pub jina_embedding_dimensions: Option<u64>,
    pub jina_rerank_model: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HttpSettings {
    pub cors_enabled: Option<bool>,
    pub cors_origins: Option<String>,
    pub rate_limit_enabled: Option<bool>,
    pub rate_limit_requests: Option<u64>,
    pub rate_limit_window_secs: Option<u64>,
    pub max_body_size_mb: Option<u64>,
    pub max_import_size_mb: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct ResilienceSettings {
    pub max_retries: Option<u64>,
    pub retry_base_delay_ms: Option<u64>,
    pub retry_max_delay_ms: Option<u64>,
    pub cb_failure_threshold: Option<u64>,
    pub cb_reset_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeOverrides {
    pub config_path: Option<PathBuf>,
    pub env_file: Option<PathBuf>,
    pub bind_addr: Option<String>,
    pub data_dir: Option<PathBuf>,
    pub print_config: bool,
    pub help: bool,
}

impl RuntimeOverrides {
    pub fn from_args<I>(args: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut overrides = Self::default();
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            let arg = arg.to_string_lossy();
            if let Some(value) = arg.strip_prefix("--config=") {
                overrides.config_path = Some(PathBuf::from(value));
            } else if arg == "--config" {
                let value = iter
                    .next()
                    .ok_or_else(|| ConfigError::InvalidCli("--config requires a path".into()))?;
                overrides.config_path = Some(PathBuf::from(value));
            } else if let Some(value) = arg.strip_prefix("--env-file=") {
                overrides.env_file = Some(PathBuf::from(value));
            } else if arg == "--env-file" {
                let value = iter
                    .next()
                    .ok_or_else(|| ConfigError::InvalidCli("--env-file requires a path".into()))?;
                overrides.env_file = Some(PathBuf::from(value));
            } else if let Some(value) = arg.strip_prefix("--bind-addr=") {
                overrides.bind_addr = Some(value.to_owned());
            } else if arg == "--bind-addr" {
                let value = iter.next().ok_or_else(|| {
                    ConfigError::InvalidCli("--bind-addr requires a value".into())
                })?;
                overrides.bind_addr = Some(value.to_string_lossy().into_owned());
            } else if let Some(value) = arg.strip_prefix("--data-dir=") {
                overrides.data_dir = Some(PathBuf::from(value));
            } else if arg == "--data-dir" {
                let value = iter
                    .next()
                    .ok_or_else(|| ConfigError::InvalidCli("--data-dir requires a path".into()))?;
                overrides.data_dir = Some(PathBuf::from(value));
            } else if arg == "--print-config" {
                overrides.print_config = true;
            } else if arg == "--help" || arg == "-h" {
                overrides.help = true;
            } else {
                return Err(ConfigError::InvalidCli(format!("unknown option: {arg}")));
            }
        }
        Ok(overrides)
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Toml {
        path: PathBuf,
        source: toml::de::Error,
    },
    InvalidCli(String),
    InvalidConfig(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "failed to read config file {}: {source}", path.display())
            }
            Self::Toml { path, source } => {
                write!(
                    f,
                    "failed to parse config file {}: {source}",
                    path.display()
                )
            }
            Self::InvalidCli(message) | Self::InvalidConfig(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ConfigError {}

impl RuntimeConfig {
    pub fn load(overrides: RuntimeOverrides) -> Result<Self, ConfigError> {
        let explicit_config_path = overrides
            .config_path
            .clone()
            .or_else(|| std::env::var_os("MEMFUSE_CONFIG").map(PathBuf::from));
        let mut settings = RawSettings::default();

        if let Some(path) = explicit_config_path {
            settings.apply_file(read_config_file(&path)?);
        } else {
            let default_path = default_config_path();
            if default_path.exists() {
                settings.apply_file(read_config_file(&default_path)?);
            }
        }

        settings.apply_env();

        if let Some(bind_addr) = overrides.bind_addr {
            settings.bind_addr = bind_addr;
        }
        if let Some(data_dir) = overrides.data_dir {
            settings.data_dir = data_dir;
        }

        settings.load_secrets()?;
        settings.validate()?;

        Ok(Self {
            app: AppConfig {
                workspace_root: settings.data_dir,
                source_kind: settings.source_kind,
                source_path: settings.source_path.unwrap_or_default(),
                target_uri: settings.target_uri,
                account_id: settings.account_id,
                user_id: settings.user_id,
                agent_id: settings.agent_id,
                canvas_separate_db: settings.canvas_separate_db,
            },
            profile: settings.profile,
            bind_addr: settings.bind_addr,
            auth_mode: settings.auth_mode,
            api_key: settings.api_key,
            api_key_file: settings.api_key_file,
            api_key_command: settings.api_key_command,
            allow_insecure_bind: settings.allow_insecure_bind,
            shutdown_timeout_ms: settings.shutdown_timeout_ms,
            providers: settings.providers,
            http: settings.http,
            resilience: settings.resilience,
            print_config: overrides.print_config,
        })
    }

    pub fn apply_to_process_env(&self) {
        set_env(
            "MEMFUSE_WORKSPACE_ROOT",
            self.app.workspace_root.to_string_lossy().as_ref(),
        );
        set_env("MEMFUSE_SOURCE_KIND", &self.app.source_kind);
        if !self.app.source_path.as_os_str().is_empty() {
            set_env(
                "MEMFUSE_SOURCE_PATH",
                self.app.source_path.to_string_lossy().as_ref(),
            );
        }
        set_env("MEMFUSE_TARGET_URI", &self.app.target_uri);
        set_env("MEMFUSE_ACCOUNT_ID", &self.app.account_id);
        set_env("MEMFUSE_USER_ID", &self.app.user_id);
        set_env("MEMFUSE_AGENT_ID", &self.app.agent_id);
        set_env("MEMFUSE_PROFILE", &self.profile);
        set_env("MEMFUSE_BIND_ADDR", &self.bind_addr);
        set_env(
            "MEMFUSE_CANVAS_SEPARATE_DB",
            if self.app.canvas_separate_db {
                "true"
            } else {
                "false"
            },
        );
        if let Some(auth_mode) = &self.auth_mode {
            set_env("MEMFUSE_AUTH_MODE", auth_mode);
        }
        if let Some(api_key) = &self.api_key {
            set_env("MEMFUSE_API_KEY", api_key);
        }
        if let Some(allow) = self.allow_insecure_bind {
            set_env(
                "MEMFUSE_ALLOW_INSECURE_BIND",
                if allow { "true" } else { "false" },
            );
        }
        if let Some(timeout) = self.shutdown_timeout_ms {
            set_env("MEMFUSE_SHUTDOWN_TIMEOUT_MS", &timeout.to_string());
        }
        self.providers.apply_to_process_env();
        self.http.apply_to_process_env();
        self.resilience.apply_to_process_env();
    }

    pub fn render_summary(&self) -> String {
        format!(
            "profile={}\nbind_addr={}\nworkspace_root={}\nsource_kind={}\naccount_id={}\nuser_id={}\nagent_id={}\ncanvas_separate_db={}",
            self.profile,
            self.bind_addr,
            self.app.workspace_root.display(),
            self.app.source_kind,
            self.app.account_id,
            self.app.user_id,
            self.app.agent_id,
            self.app.canvas_separate_db,
        )
    }
}

pub fn render_usage() -> &'static str {
    "Usage: mfs-server [OPTIONS]\n\nOptions:\n  --config <PATH>       Read runtime config.toml from PATH\n  --env-file <PATH>     Load a dotenv file before runtime config\n  --bind-addr <ADDR>    Override server bind address\n  --data-dir <PATH>     Override storage data directory\n  --print-config        Print resolved non-secret runtime config and exit\n  -h, --help            Show this help"
}

#[derive(Debug, Clone)]
struct RawSettings {
    profile: String,
    bind_addr: String,
    data_dir: PathBuf,
    source_kind: String,
    source_path: Option<PathBuf>,
    target_uri: String,
    account_id: String,
    user_id: String,
    agent_id: String,
    canvas_separate_db: bool,
    auth_mode: Option<String>,
    api_key: Option<String>,
    api_key_file: Option<PathBuf>,
    api_key_command: Option<String>,
    allow_insecure_bind: Option<bool>,
    shutdown_timeout_ms: Option<u64>,
    providers: ProviderSettings,
    http: HttpSettings,
    resilience: ResilienceSettings,
}

impl Default for RawSettings {
    fn default() -> Self {
        Self {
            profile: "development".to_owned(),
            bind_addr: "127.0.0.1:8720".to_owned(),
            data_dir: default_data_dir(),
            source_kind: "managed".to_owned(),
            source_path: None,
            target_uri: "mfs://resources/localfs/docs".to_owned(),
            account_id: "default".to_owned(),
            user_id: "default".to_owned(),
            agent_id: "default".to_owned(),
            canvas_separate_db: false,
            auth_mode: None,
            api_key: None,
            api_key_file: None,
            api_key_command: None,
            allow_insecure_bind: None,
            shutdown_timeout_ms: None,
            providers: ProviderSettings::default(),
            http: HttpSettings::default(),
            resilience: ResilienceSettings::default(),
        }
    }
}

impl RawSettings {
    fn apply_file(&mut self, file: FileConfig) {
        if let Some(server) = file.server {
            if let Some(value) = server.profile {
                self.profile = value;
            }
            if let Some(value) = server.bind_addr {
                self.bind_addr = value;
            }
            if let Some(value) = server.auth_mode {
                self.auth_mode = Some(value);
            }
            if let Some(value) = server.api_key {
                self.api_key = Some(value);
            }
            if let Some(value) = server.api_key_file {
                self.api_key_file = Some(mfs_types::expand_tilde(&value));
            }
            if let Some(value) = server.api_key_command {
                self.api_key_command = Some(value);
            }
            if let Some(value) = server.allow_insecure_bind {
                self.allow_insecure_bind = Some(value);
            }
            if let Some(value) = server.shutdown_timeout_ms {
                self.shutdown_timeout_ms = Some(value);
            }
        }
        if let Some(storage) = file.storage {
            if let Some(value) = storage.data_dir {
                self.data_dir = mfs_types::expand_tilde(&value);
            }
            if let Some(value) = storage.source_kind {
                self.source_kind = value;
            }
            if let Some(value) = storage.source_path {
                self.source_path = Some(mfs_types::expand_tilde(&value));
            }
            if let Some(value) = storage.target_uri {
                self.target_uri = value;
            }
            if let Some(value) = storage.canvas_separate_db {
                self.canvas_separate_db = value;
            }
        }
        if let Some(identity) = file.identity {
            if let Some(value) = identity.account_id {
                self.account_id = value;
            }
            if let Some(value) = identity.user_id {
                self.user_id = value;
            }
            if let Some(value) = identity.agent_id {
                self.agent_id = value;
            }
        }
        if let Some(providers) = file.providers {
            self.providers.apply_file(providers);
        }
        if let Some(http) = file.http {
            self.http.apply_file(http);
        }
        if let Some(resilience) = file.resilience {
            self.resilience.apply_file(resilience);
        }
    }

    fn apply_env(&mut self) {
        if let Ok(value) = std::env::var("MEMFUSE_PROFILE") {
            self.profile = value;
        }
        if let Ok(value) = std::env::var("MEMFUSE_BIND_ADDR") {
            self.bind_addr = value;
        }
        if let Ok(value) = std::env::var("MEMFUSE_WORKSPACE_ROOT") {
            self.data_dir = mfs_types::expand_tilde(&value);
        }
        if let Ok(value) = std::env::var("MEMFUSE_SOURCE_KIND") {
            self.source_kind = value;
        }
        if let Ok(value) = std::env::var("MEMFUSE_SOURCE_PATH") {
            self.source_path = Some(mfs_types::expand_tilde(&value));
        }
        if let Ok(value) = std::env::var("MEMFUSE_TARGET_URI") {
            self.target_uri = value;
        }
        if let Ok(value) = std::env::var("MEMFUSE_ACCOUNT_ID") {
            self.account_id = value;
        }
        if let Ok(value) = std::env::var("MEMFUSE_USER_ID") {
            self.user_id = value;
        }
        if let Ok(value) = std::env::var("MEMFUSE_AGENT_ID") {
            self.agent_id = value;
        }
        if let Ok(value) = std::env::var("MEMFUSE_CANVAS_SEPARATE_DB") {
            self.canvas_separate_db = parse_bool(&value);
        }
        if let Ok(value) = std::env::var("MEMFUSE_AUTH_MODE") {
            self.auth_mode = Some(value);
        }
        if let Ok(value) = std::env::var("MEMFUSE_API_KEY") {
            self.api_key = Some(value);
        } else {
            if let Ok(value) = std::env::var("MEMFUSE_API_KEY_FILE") {
                self.api_key = None;
                self.api_key_file = Some(mfs_types::expand_tilde(&value));
            }
            if let Ok(value) = std::env::var("MEMFUSE_API_KEY_COMMAND") {
                self.api_key = None;
                self.api_key_command = Some(value);
            }
        }
        if let Ok(value) = std::env::var("MEMFUSE_ALLOW_INSECURE_BIND") {
            self.allow_insecure_bind = Some(parse_bool(&value));
        }
        if let Ok(value) = std::env::var("MEMFUSE_SHUTDOWN_TIMEOUT_MS") {
            self.shutdown_timeout_ms = value.parse().ok();
        }
        self.providers.apply_env();
        self.http.apply_env();
        self.resilience.apply_env();
    }

    fn load_secrets(&mut self) -> Result<(), ConfigError> {
        if self.api_key.is_some() {
            return Ok(());
        }
        if let Some(path) = self.api_key_file.as_ref() {
            let secret = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
                path: path.clone(),
                source,
            })?;
            self.api_key = Some(secret.trim().to_owned());
            return Ok(());
        }
        if let Some(command) = self.api_key_command.as_ref() {
            let output = run_secret_command(command)?;
            self.api_key = Some(output);
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if matches!(self.source_kind.as_str(), "localfs" | "git")
            && self
                .source_path
                .as_ref()
                .is_none_or(|p| p.as_os_str().is_empty())
        {
            return Err(ConfigError::InvalidConfig(format!(
                "source_path is required when source_kind is '{}'",
                self.source_kind
            )));
        }
        let auth_mode = self.auth_mode.as_deref().unwrap_or("dev");
        let allow_insecure = self.allow_insecure_bind.unwrap_or(false);
        if self.profile == "production"
            && !is_local_bind_addr(&self.bind_addr)
            && auth_mode == "dev"
            && !allow_insecure
        {
            return Err(ConfigError::InvalidConfig(
                "production profile refuses non-local bind with dev auth".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    server: Option<ServerConfig>,
    storage: Option<StorageConfig>,
    identity: Option<IdentityConfig>,
    providers: Option<ProviderConfig>,
    http: Option<HttpConfig>,
    resilience: Option<ResilienceConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct ServerConfig {
    profile: Option<String>,
    bind_addr: Option<String>,
    auth_mode: Option<String>,
    api_key: Option<String>,
    api_key_file: Option<String>,
    api_key_command: Option<String>,
    allow_insecure_bind: Option<bool>,
    shutdown_timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct StorageConfig {
    data_dir: Option<String>,
    source_kind: Option<String>,
    source_path: Option<String>,
    target_uri: Option<String>,
    canvas_separate_db: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct IdentityConfig {
    account_id: Option<String>,
    user_id: Option<String>,
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ProviderConfig {
    summary_provider: Option<String>,
    embedding_provider: Option<String>,
    rerank_provider: Option<String>,
    openai_api_base: Option<String>,
    openai_api_key: Option<String>,
    summary_model: Option<String>,
    chat_model: Option<String>,
    embedding_model: Option<String>,
    summary_concurrency: Option<u64>,
    jina_api_key: Option<String>,
    jina_base_url: Option<String>,
    jina_embedding_model: Option<String>,
    jina_embedding_dimensions: Option<u64>,
    jina_rerank_model: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct HttpConfig {
    cors_enabled: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_string_or_list")]
    cors_origins: Option<String>,
    rate_limit_enabled: Option<bool>,
    rate_limit_requests: Option<u64>,
    rate_limit_window_secs: Option<u64>,
    max_body_size_mb: Option<u64>,
    max_import_size_mb: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct ResilienceConfig {
    max_retries: Option<u64>,
    retry_base_delay_ms: Option<u64>,
    retry_max_delay_ms: Option<u64>,
    cb_failure_threshold: Option<u64>,
    cb_reset_timeout_ms: Option<u64>,
}

impl ProviderSettings {
    fn apply_file(&mut self, providers: ProviderConfig) {
        assign_provider_opt(&mut self.summary_provider, providers.summary_provider);
        assign_provider_opt(&mut self.embedding_provider, providers.embedding_provider);
        assign_provider_opt(&mut self.rerank_provider, providers.rerank_provider);
        assign_opt(&mut self.openai_api_base, providers.openai_api_base);
        assign_opt(&mut self.openai_api_key, providers.openai_api_key);
        assign_opt(&mut self.summary_model, providers.summary_model);
        assign_opt(&mut self.chat_model, providers.chat_model);
        assign_opt(&mut self.embedding_model, providers.embedding_model);
        assign_opt(&mut self.summary_concurrency, providers.summary_concurrency);
        assign_opt(&mut self.jina_api_key, providers.jina_api_key);
        assign_opt(&mut self.jina_base_url, providers.jina_base_url);
        assign_opt(
            &mut self.jina_embedding_model,
            providers.jina_embedding_model,
        );
        assign_opt(
            &mut self.jina_embedding_dimensions,
            providers.jina_embedding_dimensions,
        );
        assign_opt(&mut self.jina_rerank_model, providers.jina_rerank_model);
    }

    fn apply_env(&mut self) {
        apply_env_provider(&mut self.summary_provider, "MEMFUSE_SUMMARY_PROVIDER");
        apply_env_provider(&mut self.embedding_provider, "MEMFUSE_EMBEDDING_PROVIDER");
        apply_env_provider(&mut self.rerank_provider, "MEMFUSE_RERANK_PROVIDER");
        apply_env_string(&mut self.openai_api_base, "MEMFUSE_OPENAI_API_BASE");
        apply_env_string(&mut self.openai_api_key, "MEMFUSE_OPENAI_API_KEY");
        apply_env_string(&mut self.summary_model, "MEMFUSE_SUMMARY_MODEL");
        apply_env_string(&mut self.chat_model, "MEMFUSE_CHAT_MODEL");
        apply_env_string(&mut self.embedding_model, "MEMFUSE_EMBEDDING_MODEL");
        apply_env_u64(&mut self.summary_concurrency, "MEMFUSE_SUMMARY_CONCURRENCY");
        apply_env_string(&mut self.jina_api_key, "MEMFUSE_JINA_API_KEY");
        apply_env_string(&mut self.jina_base_url, "MEMFUSE_JINA_BASE_URL");
        apply_env_string(
            &mut self.jina_embedding_model,
            "MEMFUSE_JINA_EMBEDDING_MODEL",
        );
        apply_env_u64(
            &mut self.jina_embedding_dimensions,
            "MEMFUSE_JINA_EMBEDDING_DIMENSIONS",
        );
        apply_env_string(&mut self.jina_rerank_model, "MEMFUSE_JINA_RERANK_MODEL");
    }

    fn apply_to_process_env(&self) {
        set_env_if_some("MEMFUSE_SUMMARY_PROVIDER", self.summary_provider.as_deref());
        set_env_if_some(
            "MEMFUSE_EMBEDDING_PROVIDER",
            self.embedding_provider.as_deref(),
        );
        set_env_if_some("MEMFUSE_RERANK_PROVIDER", self.rerank_provider.as_deref());
        set_env_if_some("MEMFUSE_OPENAI_API_BASE", self.openai_api_base.as_deref());
        set_env_if_some("MEMFUSE_OPENAI_API_KEY", self.openai_api_key.as_deref());
        set_env_if_some("MEMFUSE_SUMMARY_MODEL", self.summary_model.as_deref());
        set_env_if_some("MEMFUSE_CHAT_MODEL", self.chat_model.as_deref());
        set_env_if_some("MEMFUSE_EMBEDDING_MODEL", self.embedding_model.as_deref());
        set_env_u64_if_some("MEMFUSE_SUMMARY_CONCURRENCY", self.summary_concurrency);
        set_env_if_some("MEMFUSE_JINA_API_KEY", self.jina_api_key.as_deref());
        set_env_if_some("MEMFUSE_JINA_BASE_URL", self.jina_base_url.as_deref());
        set_env_if_some(
            "MEMFUSE_JINA_EMBEDDING_MODEL",
            self.jina_embedding_model.as_deref(),
        );
        set_env_u64_if_some(
            "MEMFUSE_JINA_EMBEDDING_DIMENSIONS",
            self.jina_embedding_dimensions,
        );
        set_env_if_some(
            "MEMFUSE_JINA_RERANK_MODEL",
            self.jina_rerank_model.as_deref(),
        );
    }
}

impl HttpSettings {
    fn apply_file(&mut self, http: HttpConfig) {
        assign_opt(&mut self.cors_enabled, http.cors_enabled);
        assign_opt(&mut self.cors_origins, http.cors_origins);
        assign_opt(&mut self.rate_limit_enabled, http.rate_limit_enabled);
        assign_opt(&mut self.rate_limit_requests, http.rate_limit_requests);
        assign_opt(
            &mut self.rate_limit_window_secs,
            http.rate_limit_window_secs,
        );
        assign_opt(&mut self.max_body_size_mb, http.max_body_size_mb);
        assign_opt(&mut self.max_import_size_mb, http.max_import_size_mb);
    }

    fn apply_env(&mut self) {
        apply_env_bool(&mut self.cors_enabled, "MEMFUSE_CORS_ENABLED");
        apply_env_string(&mut self.cors_origins, "MEMFUSE_CORS_ORIGINS");
        apply_env_bool(&mut self.rate_limit_enabled, "MEMFUSE_RATE_LIMIT_ENABLED");
        apply_env_u64(&mut self.rate_limit_requests, "MEMFUSE_RATE_LIMIT_REQUESTS");
        apply_env_u64(
            &mut self.rate_limit_window_secs,
            "MEMFUSE_RATE_LIMIT_WINDOW_SECS",
        );
        apply_env_u64(&mut self.max_body_size_mb, "MEMFUSE_MAX_BODY_SIZE_MB");
        apply_env_u64(&mut self.max_import_size_mb, "MEMFUSE_MAX_IMPORT_SIZE_MB");
    }

    fn apply_to_process_env(&self) {
        set_env_bool_if_some("MEMFUSE_CORS_ENABLED", self.cors_enabled);
        set_env_if_some("MEMFUSE_CORS_ORIGINS", self.cors_origins.as_deref());
        set_env_bool_if_some("MEMFUSE_RATE_LIMIT_ENABLED", self.rate_limit_enabled);
        set_env_u64_if_some("MEMFUSE_RATE_LIMIT_REQUESTS", self.rate_limit_requests);
        set_env_u64_if_some(
            "MEMFUSE_RATE_LIMIT_WINDOW_SECS",
            self.rate_limit_window_secs,
        );
        set_env_u64_if_some("MEMFUSE_MAX_BODY_SIZE_MB", self.max_body_size_mb);
        set_env_u64_if_some("MEMFUSE_MAX_IMPORT_SIZE_MB", self.max_import_size_mb);
    }
}

impl ResilienceSettings {
    fn apply_file(&mut self, resilience: ResilienceConfig) {
        assign_opt(&mut self.max_retries, resilience.max_retries);
        assign_opt(
            &mut self.retry_base_delay_ms,
            resilience.retry_base_delay_ms,
        );
        assign_opt(&mut self.retry_max_delay_ms, resilience.retry_max_delay_ms);
        assign_opt(
            &mut self.cb_failure_threshold,
            resilience.cb_failure_threshold,
        );
        assign_opt(
            &mut self.cb_reset_timeout_ms,
            resilience.cb_reset_timeout_ms,
        );
    }

    fn apply_env(&mut self) {
        apply_env_u64(&mut self.max_retries, "MEMFUSE_MAX_RETRIES");
        apply_env_u64(&mut self.retry_base_delay_ms, "MEMFUSE_RETRY_BASE_DELAY_MS");
        apply_env_u64(&mut self.retry_max_delay_ms, "MEMFUSE_RETRY_MAX_DELAY_MS");
        apply_env_u64(
            &mut self.cb_failure_threshold,
            "MEMFUSE_CB_FAILURE_THRESHOLD",
        );
        apply_env_u64(&mut self.cb_reset_timeout_ms, "MEMFUSE_CB_RESET_TIMEOUT_MS");
    }

    fn apply_to_process_env(&self) {
        set_env_u64_if_some("MEMFUSE_MAX_RETRIES", self.max_retries);
        set_env_u64_if_some("MEMFUSE_RETRY_BASE_DELAY_MS", self.retry_base_delay_ms);
        set_env_u64_if_some("MEMFUSE_RETRY_MAX_DELAY_MS", self.retry_max_delay_ms);
        set_env_u64_if_some("MEMFUSE_CB_FAILURE_THRESHOLD", self.cb_failure_threshold);
        set_env_u64_if_some("MEMFUSE_CB_RESET_TIMEOUT_MS", self.cb_reset_timeout_ms);
    }
}

fn read_config_file(path: &Path) -> Result<FileConfig, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&raw).map_err(|source| ConfigError::Toml {
        path: path.to_path_buf(),
        source,
    })
}

fn assign_opt<T>(target: &mut Option<T>, value: Option<T>) {
    if value.is_some() {
        *target = value;
    }
}

fn assign_provider_opt(target: &mut Option<String>, value: Option<String>) {
    if let Some(value) = value {
        *target = normalize_provider_value(value);
    }
}

fn apply_env_string(target: &mut Option<String>, key: &str) {
    if let Ok(value) = std::env::var(key) {
        *target = Some(value);
    }
}

fn apply_env_provider(target: &mut Option<String>, key: &str) {
    if let Ok(value) = std::env::var(key) {
        *target = normalize_provider_value(value);
    }
}

fn normalize_provider_value(value: String) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() || value == "auto" {
        None
    } else {
        Some(value)
    }
}

fn apply_env_bool(target: &mut Option<bool>, key: &str) {
    if let Ok(value) = std::env::var(key) {
        *target = Some(parse_bool(&value));
    }
}

fn apply_env_u64(target: &mut Option<u64>, key: &str) {
    if let Ok(value) = std::env::var(key) {
        *target = value.parse().ok();
    }
}

fn set_env_if_some(key: &str, value: Option<&str>) {
    if let Some(value) = value {
        set_env(key, value);
    }
}

fn set_env_bool_if_some(key: &str, value: Option<bool>) {
    if let Some(value) = value {
        set_env(key, if value { "true" } else { "false" });
    }
}

fn set_env_u64_if_some(key: &str, value: Option<u64>) {
    if let Some(value) = value {
        set_env(key, &value.to_string());
    }
}

fn deserialize_optional_string_or_list<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrList {
        String(String),
        List(Vec<String>),
    }

    Ok(
        Option::<StringOrList>::deserialize(deserializer)?.map(|value| match value {
            StringOrList::String(value) => value,
            StringOrList::List(values) => values.join(","),
        }),
    )
}

fn run_secret_command(command: &str) -> Result<String, ConfigError> {
    #[cfg(windows)]
    let output = std::process::Command::new("cmd")
        .args(["/C", command])
        .output()
        .map_err(|source| {
            ConfigError::InvalidConfig(format!("failed to run api_key_command: {source}"))
        })?;

    #[cfg(not(windows))]
    let output = std::process::Command::new("sh")
        .args(["-c", command])
        .output()
        .map_err(|source| {
            ConfigError::InvalidConfig(format!("failed to run api_key_command: {source}"))
        })?;

    if !output.status.success() {
        return Err(ConfigError::InvalidConfig(format!(
            "api_key_command exited with status {}",
            output.status
        )));
    }
    let secret = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if secret.is_empty() {
        return Err(ConfigError::InvalidConfig(
            "api_key_command produced an empty secret".to_owned(),
        ));
    }
    Ok(secret)
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn is_local_bind_addr(bind_addr: &str) -> bool {
    bind_addr.starts_with("127.")
        || bind_addr.starts_with("localhost:")
        || bind_addr == "localhost"
        || bind_addr.contains("[::1]")
}

fn memfuse_home() -> PathBuf {
    std::env::var_os("MEMFUSE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".memfuse"))
}

fn default_config_path() -> PathBuf {
    memfuse_home().join("config.toml")
}

fn default_data_dir() -> PathBuf {
    memfuse_home().join("data")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[allow(unsafe_code)]
fn set_env(key: &str, value: &str) {
    // SAFETY: Called during single-threaded process startup before the Axum
    // server or background tasks are spawned.
    unsafe { std::env::set_var(key, value) };
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{LazyLock, Mutex};

    use tempfile::tempdir;

    use super::{RuntimeConfig, RuntimeOverrides};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn environment_overrides_explicit_config_file_values() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let tmp = tempdir().unwrap();
        let data_dir = tmp.path().join("config-data");
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[server]
bind_addr = "127.0.0.1:9999"

[storage]
data_dir = "{}"
source_kind = "managed"

[identity]
user_id = "from-config"
"#,
                data_dir.display()
            ),
        )
        .unwrap();

        let env_data_dir = tmp.path().join("env-data");
        let _env = EnvGuard::set(&[
            (
                "MEMFUSE_WORKSPACE_ROOT",
                Some(env_data_dir.to_string_lossy().as_ref()),
            ),
            ("MEMFUSE_BIND_ADDR", Some("127.0.0.1:8888")),
            ("MEMFUSE_USER_ID", Some("from-env")),
            ("MEMFUSE_SOURCE_KIND", None),
            ("MEMFUSE_SOURCE_PATH", None),
        ]);

        let config = RuntimeConfig::load(RuntimeOverrides {
            config_path: Some(config_path),
            ..RuntimeOverrides::default()
        })
        .unwrap();

        assert_eq!(config.bind_addr, "127.0.0.1:8888");
        assert_eq!(config.app.workspace_root, env_data_dir);
        assert_eq!(config.app.user_id, "from-env");
    }

    #[test]
    fn cli_overrides_win_over_explicit_config() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let tmp = tempdir().unwrap();
        let config_data_dir = tmp.path().join("config-data");
        let cli_data_dir = tmp.path().join("cli-data");
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[server]
bind_addr = "127.0.0.1:9999"

[storage]
data_dir = "{}"
source_kind = "managed"
"#,
                config_data_dir.display()
            ),
        )
        .unwrap();
        let _env = EnvGuard::set(&[
            ("MEMFUSE_BIND_ADDR", Some("127.0.0.1:8888")),
            ("MEMFUSE_WORKSPACE_ROOT", None),
            ("MEMFUSE_SOURCE_KIND", None),
            ("MEMFUSE_SOURCE_PATH", None),
        ]);

        let config = RuntimeConfig::load(RuntimeOverrides {
            config_path: Some(config_path),
            bind_addr: Some("127.0.0.1:7777".to_owned()),
            data_dir: Some(cli_data_dir.clone()),
            ..RuntimeOverrides::default()
        })
        .unwrap();

        assert_eq!(config.bind_addr, "127.0.0.1:7777");
        assert_eq!(config.app.workspace_root, cli_data_dir);
    }

    #[test]
    fn cli_parser_supports_help() {
        let overrides = RuntimeOverrides::from_args([std::ffi::OsString::from("--help")]).unwrap();
        assert!(overrides.help);
    }

    #[test]
    fn localfs_requires_source_path_after_merging_config() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[storage]
data_dir = "/tmp/memfuse"
source_kind = "localfs"
"#,
        )
        .unwrap();
        let _env = EnvGuard::set(&[
            ("MEMFUSE_SOURCE_KIND", None),
            ("MEMFUSE_SOURCE_PATH", None),
            ("MEMFUSE_WORKSPACE_ROOT", None),
        ]);

        let err = RuntimeConfig::load(RuntimeOverrides {
            config_path: Some(config_path),
            ..RuntimeOverrides::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains("source_path"));
    }

    #[test]
    fn production_profile_rejects_public_dev_auth_bind() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[server]
profile = "production"
bind_addr = "0.0.0.0:8720"
auth_mode = "dev"

[storage]
data_dir = "{}"
source_kind = "managed"
"#,
                tmp.path().join("data").display()
            ),
        )
        .unwrap();
        let _env = EnvGuard::set(&[
            ("MEMFUSE_PROFILE", None),
            ("MEMFUSE_BIND_ADDR", None),
            ("MEMFUSE_AUTH_MODE", None),
            ("MEMFUSE_ALLOW_INSECURE_BIND", None),
        ]);

        let err = RuntimeConfig::load(RuntimeOverrides {
            config_path: Some(config_path),
            ..RuntimeOverrides::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains("production"));
        assert!(err.to_string().contains("dev auth"));
    }

    #[test]
    fn api_key_file_loads_secret_without_rendering_it() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let tmp = tempdir().unwrap();
        let secret_path = tmp.path().join("api-key");
        let config_path = tmp.path().join("config.toml");
        fs::write(&secret_path, "secret-from-file\n").unwrap();
        fs::write(
            &config_path,
            format!(
                r#"
[server]
auth_mode = "api_key"
api_key_file = "{}"

[storage]
data_dir = "{}"
source_kind = "managed"
"#,
                secret_path.display(),
                tmp.path().join("data").display()
            ),
        )
        .unwrap();
        let _env = EnvGuard::set(&[("MEMFUSE_API_KEY", None)]);

        let config = RuntimeConfig::load(RuntimeOverrides {
            config_path: Some(config_path),
            ..RuntimeOverrides::default()
        })
        .unwrap();

        assert_eq!(config.api_key.as_deref(), Some("secret-from-file"));
        assert!(!config.render_summary().contains("secret-from-file"));
    }

    #[test]
    fn api_key_command_loads_secret_without_rendering_it() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[server]
auth_mode = "api_key"
api_key_command = "printf secret-from-command"

[storage]
data_dir = "{}"
source_kind = "managed"
"#,
                tmp.path().join("data").display()
            ),
        )
        .unwrap();
        let _env = EnvGuard::set(&[
            ("MEMFUSE_API_KEY", None),
            ("MEMFUSE_API_KEY_FILE", None),
            ("MEMFUSE_API_KEY_COMMAND", None),
        ]);

        let config = RuntimeConfig::load(RuntimeOverrides {
            config_path: Some(config_path),
            ..RuntimeOverrides::default()
        })
        .unwrap();

        assert_eq!(config.api_key.as_deref(), Some("secret-from-command"));
        assert!(!config.render_summary().contains("secret-from-command"));
    }

    #[test]
    fn file_config_applies_provider_http_and_resilience_settings() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[storage]
data_dir = "{}"
source_kind = "managed"

[providers]
summary_provider = "deterministic"
embedding_provider = "deterministic"
rerank_provider = "deterministic"
openai_api_base = "https://api.example.test/v1"
summary_model = "summary-test"
chat_model = "chat-test"
jina_embedding_model = "jina-test"
summary_concurrency = 2

[resilience]
max_retries = 4
retry_base_delay_ms = 250
retry_max_delay_ms = 9000
cb_failure_threshold = 7
cb_reset_timeout_ms = 60000

[http]
cors_enabled = false
cors_origins = "http://localhost:3000"
rate_limit_enabled = false
max_body_size_mb = 12
"#,
                tmp.path().join("data").display()
            ),
        )
        .unwrap();
        let _env = EnvGuard::set(&[
            ("MEMFUSE_SUMMARY_PROVIDER", None),
            ("MEMFUSE_EMBEDDING_PROVIDER", None),
            ("MEMFUSE_RERANK_PROVIDER", None),
            ("MEMFUSE_OPENAI_API_BASE", None),
            ("MEMFUSE_SUMMARY_MODEL", None),
            ("MEMFUSE_CHAT_MODEL", None),
            ("MEMFUSE_JINA_EMBEDDING_MODEL", None),
            ("MEMFUSE_SUMMARY_CONCURRENCY", None),
            ("MEMFUSE_MAX_RETRIES", None),
            ("MEMFUSE_RETRY_BASE_DELAY_MS", None),
            ("MEMFUSE_RETRY_MAX_DELAY_MS", None),
            ("MEMFUSE_CB_FAILURE_THRESHOLD", None),
            ("MEMFUSE_CB_RESET_TIMEOUT_MS", None),
            ("MEMFUSE_CORS_ENABLED", None),
            ("MEMFUSE_CORS_ORIGINS", None),
            ("MEMFUSE_RATE_LIMIT_ENABLED", None),
            ("MEMFUSE_MAX_BODY_SIZE_MB", None),
        ]);

        let config = RuntimeConfig::load(RuntimeOverrides {
            config_path: Some(config_path),
            ..RuntimeOverrides::default()
        })
        .unwrap();
        config.apply_to_process_env();

        assert_eq!(
            std::env::var("MEMFUSE_SUMMARY_PROVIDER").unwrap(),
            "deterministic"
        );
        assert_eq!(
            std::env::var("MEMFUSE_EMBEDDING_PROVIDER").unwrap(),
            "deterministic"
        );
        assert_eq!(
            std::env::var("MEMFUSE_RERANK_PROVIDER").unwrap(),
            "deterministic"
        );
        assert_eq!(
            std::env::var("MEMFUSE_OPENAI_API_BASE").unwrap(),
            "https://api.example.test/v1"
        );
        assert_eq!(
            std::env::var("MEMFUSE_SUMMARY_MODEL").unwrap(),
            "summary-test"
        );
        assert_eq!(std::env::var("MEMFUSE_CHAT_MODEL").unwrap(), "chat-test");
        assert_eq!(
            std::env::var("MEMFUSE_JINA_EMBEDDING_MODEL").unwrap(),
            "jina-test"
        );
        assert_eq!(std::env::var("MEMFUSE_SUMMARY_CONCURRENCY").unwrap(), "2");
        assert_eq!(std::env::var("MEMFUSE_MAX_RETRIES").unwrap(), "4");
        assert_eq!(std::env::var("MEMFUSE_RETRY_BASE_DELAY_MS").unwrap(), "250");
        assert_eq!(std::env::var("MEMFUSE_RETRY_MAX_DELAY_MS").unwrap(), "9000");
        assert_eq!(std::env::var("MEMFUSE_CB_FAILURE_THRESHOLD").unwrap(), "7");
        assert_eq!(
            std::env::var("MEMFUSE_CB_RESET_TIMEOUT_MS").unwrap(),
            "60000"
        );
        assert_eq!(std::env::var("MEMFUSE_CORS_ENABLED").unwrap(), "false");
        assert_eq!(
            std::env::var("MEMFUSE_CORS_ORIGINS").unwrap(),
            "http://localhost:3000"
        );
        assert_eq!(
            std::env::var("MEMFUSE_RATE_LIMIT_ENABLED").unwrap(),
            "false"
        );
        assert_eq!(std::env::var("MEMFUSE_MAX_BODY_SIZE_MB").unwrap(), "12");
    }

    #[test]
    fn auto_provider_values_preserve_auto_detection() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[storage]
data_dir = "{}"
source_kind = "managed"

[providers]
summary_provider = "auto"
embedding_provider = "auto"
rerank_provider = "auto"
"#,
                tmp.path().join("data").display()
            ),
        )
        .unwrap();
        let _env = EnvGuard::set(&[
            ("MEMFUSE_SUMMARY_PROVIDER", None),
            ("MEMFUSE_EMBEDDING_PROVIDER", None),
            ("MEMFUSE_RERANK_PROVIDER", None),
        ]);

        let config = RuntimeConfig::load(RuntimeOverrides {
            config_path: Some(config_path),
            ..RuntimeOverrides::default()
        })
        .unwrap();
        config.apply_to_process_env();

        assert!(std::env::var("MEMFUSE_SUMMARY_PROVIDER").is_err());
        assert!(std::env::var("MEMFUSE_EMBEDDING_PROVIDER").is_err());
        assert!(std::env::var("MEMFUSE_RERANK_PROVIDER").is_err());
    }

    struct EnvGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
            let saved = vars
                .iter()
                .map(|(key, _)| (*key, std::env::var_os(key)))
                .collect();
            for (key, value) in vars {
                match value {
                    Some(value) => set_test_env(key, value),
                    None => remove_test_env(key),
                }
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..).rev() {
                match value {
                    Some(value) => set_test_env(key, value),
                    None => remove_test_env(key),
                }
            }
        }
    }

    #[allow(unsafe_code)]
    fn set_test_env(key: &str, value: impl AsRef<std::ffi::OsStr>) {
        // SAFETY: Runtime config tests hold ENV_LOCK for their whole env mutation window.
        unsafe { std::env::set_var(key, value) };
    }

    #[allow(unsafe_code)]
    fn remove_test_env(key: &str) {
        // SAFETY: Runtime config tests hold ENV_LOCK for their whole env mutation window.
        unsafe { std::env::remove_var(key) };
    }
}
