use std::path::PathBuf;

/// Canonical default port for the MemFuse server.
///
/// Single source of truth on the Rust side — every crate (server runtime
/// config, CLI, docs templates) must reference these constants instead of
/// hardcoding a port. At runtime the value is always overridable via
/// `MEMFUSE_BIND_ADDR` / `MEMFUSE_SERVER_URL` (environment / `.env` /
/// `config.toml`). The TypeScript counterpart lives in
/// `sdk/src/shared/config.ts`.
pub const DEFAULT_PORT: u16 = 18720;

/// Canonical default bind address (`127.0.0.1:{DEFAULT_PORT}`).
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:18720";

/// Canonical default server URL (`http://{DEFAULT_BIND_ADDR}`).
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:18720";

/// Expand a leading `~` in a string to the value of `$HOME`.
///
/// Neither `dotenvy` nor Rust's `PathBuf` perform shell tilde expansion,
/// so a `.env` value like `~/.memfuse/data` is treated as a literal
/// relative path starting with a directory named `~`.  This function
/// corrects that by substituting the actual home directory.
///
/// Returns the input unchanged if it does not start with `~`.
pub fn expand_tilde(input: &str) -> PathBuf {
    if input == "~" || input.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_owned());
        PathBuf::from(input.replacen('~', &home, 1))
    } else {
        PathBuf::from(input)
    }
}

/// Expand a leading `~` in a `Path`-like value.
///
/// Convenience wrapper around [`expand_tilde`] that accepts `&Path`
/// (the idiomatic Rust path parameter type) and returns `PathBuf`,
/// making it easy to post-process clap arguments.
pub fn expand_tilde_path(path: &std::path::Path) -> PathBuf {
    match path.to_str() {
        Some(s) => expand_tilde(s),
        None => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_endpoint_constants_are_consistent() {
        assert_eq!(DEFAULT_BIND_ADDR, format!("127.0.0.1:{DEFAULT_PORT}"));
        assert_eq!(DEFAULT_SERVER_URL, format!("http://{DEFAULT_BIND_ADDR}"));
    }

    #[test]
    fn expands_tilde_slash() {
        // Guard: HOME must be set on the test host (always true on macOS/Linux).
        let home = std::env::var("HOME").unwrap();
        let result = expand_tilde("~/.memfuse/data");
        assert_eq!(result, PathBuf::from(format!("{}/.memfuse/data", home)));
    }

    #[test]
    fn expands_bare_tilde() {
        let home = std::env::var("HOME").unwrap();
        let result = expand_tilde("~");
        assert_eq!(result, PathBuf::from(home));
    }

    #[test]
    fn no_expand_without_tilde() {
        let result = expand_tilde("/absolute/path");
        assert_eq!(result, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn no_expand_relative_without_tilde() {
        let result = expand_tilde("relative/path");
        assert_eq!(result, PathBuf::from("relative/path"));
    }

    #[test]
    fn expand_tilde_path_wrapper() {
        let home = std::env::var("HOME").unwrap();
        let input = PathBuf::from("~/.memfuse/data");
        let result = expand_tilde_path(&input);
        assert_eq!(result, PathBuf::from(format!("{}/.memfuse/data", home)));
    }

    #[test]
    fn no_expand_in_middle() {
        // "~" only expands at the start; "foo~/bar" stays literal.
        let result = expand_tilde("foo~/bar");
        assert_eq!(result, PathBuf::from("foo~/bar"));
    }

    #[test]
    fn no_expand_tilde_username() {
        // "~user/path" is NOT expanded — only bare "~" and "~/" are handled.
        let result = expand_tilde("~root/tmp");
        assert_eq!(result, PathBuf::from("~root/tmp"));
    }
}
