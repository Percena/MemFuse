fn main() {
    // Initialize structured logging for the MCP stdio process.
    // MCP communicates over stdout, so log output goes to stderr.
    // Level controlled by RUST_LOG; default to "warn" to avoid polluting
    // the JSON-RPC stream with log lines.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    mfs_mcp::run().expect("MCP server failed");
}
