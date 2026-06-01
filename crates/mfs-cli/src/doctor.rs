pub fn run_doctor(workspace_root: &std::path::Path) {
    let mut checks_passed = 0;
    let mut checks_failed = 0;
    let mut checks_warned = 0;

    println!("=== MemFuse Doctor ===");
    println!();

    // Check 1: Workspace directory exists and is writable
    println!("[1] Workspace directory");
    if workspace_root.exists() {
        println!("  ✓ workspace root exists: {}", workspace_root.display());
        checks_passed += 1;

        // Check writability
        let test_file = workspace_root.join(".mfs_doctor_test");
        match std::fs::write(&test_file, "test") {
            Ok(()) => {
                let _ = std::fs::remove_file(&test_file);
                println!("  ✓ workspace root is writable");
                checks_passed += 1;
            }
            Err(e) => {
                println!("  ✗ FAIL: workspace root is not writable: {}", e);
                checks_failed += 1;
            }
        }
    } else {
        println!(
            "  ✗ FAIL: workspace root does not exist: {}",
            workspace_root.display()
        );
        checks_failed += 1;
    }

    // Check 2: _system directory and metadata.sqlite
    println!();
    println!("[2] Metadata store");
    let system_dir = workspace_root.join("_system");
    if system_dir.exists() {
        println!("  ✓ _system directory exists");
        checks_passed += 1;

        let metadata_path = system_dir.join("metadata.sqlite");
        if metadata_path.exists() {
            match mfs_metadata::MetadataStore::open_at(&metadata_path, false) {
                Ok(_) => {
                    println!("  ✓ metadata.sqlite is readable");
                    checks_passed += 1;
                }
                Err(e) => {
                    println!(
                        "  ✗ FAIL: metadata.sqlite is corrupted or unreadable: {}",
                        e
                    );
                    checks_failed += 1;
                }
            }
        } else {
            println!("  ⚠ WARN: metadata.sqlite does not exist (will be created on first use)");
            checks_warned += 1;
        }
    } else {
        println!("  ⚠ WARN: _system directory does not exist (will be created on first use)");
        checks_warned += 1;
    }

    // Check 3: Semantic index
    println!();
    println!("[3] Semantic index");
    let semantic_dir = system_dir.join("semantic");
    if semantic_dir.exists() {
        println!("  ✓ semantic index directory exists");
        checks_passed += 1;

        // Check if sqlite index exists
        let index_path = semantic_dir.join("index.sqlite");
        if index_path.exists() {
            println!("  ✓ semantic index.sqlite exists");
            checks_passed += 1;
        } else {
            println!(
                "  ⚠ WARN: semantic index.sqlite does not exist (run `mfs rebuild` to create)"
            );
            checks_warned += 1;
        }
    } else {
        println!(
            "  ⚠ WARN: semantic index directory does not exist (will be created on first use)"
        );
        checks_warned += 1;
    }

    // Check 4: PID lock status
    println!();
    println!("[4] PID lock");
    let pid_file = workspace_root.join(".mfs.pid");
    if pid_file.exists() {
        match std::fs::read_to_string(&pid_file) {
            Ok(contents) => {
                let pid_str = contents.trim().lines().next().unwrap_or("").trim();
                match pid_str.parse::<u32>() {
                    Ok(pid) => {
                        #[allow(unsafe_code)]
                        // Check if the process is still alive (Unix only)
                        #[cfg(unix)]
                        {
                            // SAFETY: kill(pid, 0) is a well-defined POSIX call that only checks
                            // process existence without sending any signal. Signal number 0
                            // does not affect the target process — it merely returns 0 if
                            // the process exists and the caller has permission to signal it,
                            // or -1 with ESRCH/EPERM otherwise.
                            let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
                            if ret == 0 {
                                println!("  ⚠ WARN: PID lock held by running process PID={}", pid);
                                println!(
                                    "  ⚠ Another MemFuse server instance is running on this workspace"
                                );
                                checks_warned += 1;
                            } else {
                                println!(
                                    "  ✓ PID lock file exists but process PID={} is stale (no longer running)",
                                    pid
                                );
                                println!(
                                    "  → Run `mfs rebuild` or start server to clean up stale lock"
                                );
                                checks_passed += 1;
                            }
                        }
                        #[cfg(not(unix))]
                        {
                            println!(
                                "  ⚠ WARN: PID lock file exists (PID={}), cannot verify process status on non-Unix",
                                pid
                            );
                            checks_warned += 1;
                        }
                    }
                    Err(_) => {
                        println!(
                            "  ⚠ WARN: PID lock file contains invalid content: {}",
                            pid_str
                        );
                        checks_warned += 1;
                    }
                }
            }
            Err(e) => {
                println!("  ✗ FAIL: Cannot read PID lock file: {}", e);
                checks_failed += 1;
            }
        }
    } else {
        println!("  ✓ No PID lock file (server not running or clean shutdown)");
        checks_passed += 1;
    }

    // Check 5: Provider configuration
    println!();
    println!("[5] Provider configuration");
    let summary_provider =
        std::env::var("MEMFUSE_SUMMARY_PROVIDER").unwrap_or_else(|_| "deterministic".to_owned());
    let embedding_provider =
        std::env::var("MEMFUSE_EMBEDDING_PROVIDER").unwrap_or_else(|_| "deterministic".to_owned());
    println!("  summary_provider={}", summary_provider);
    println!("  embedding_provider={}", embedding_provider);

    if summary_provider == "openai" || embedding_provider == "openai" {
        let api_key_set = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .or_else(|| {
                std::env::var("MEMFUSE_OPENAI_API_KEY")
                    .ok()
                    .filter(|k| !k.is_empty())
            })
            .is_some();
        if api_key_set {
            println!("  ✓ OPENAI_API_KEY is set");
            checks_passed += 1;
        } else {
            println!("  ✗ FAIL: openai provider selected but OPENAI_API_KEY is not set");
            checks_failed += 1;
        }

        let api_base = std::env::var("OPENAI_API_BASE")
            .or_else(|_| std::env::var("MEMFUSE_OPENAI_API_BASE"))
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
        println!("  api_base={}", api_base);
        checks_passed += 1;
    } else {
        println!("  ✓ Using deterministic providers (no API key required)");
        checks_passed += 1;
    }

    // Check 6: Resilience configuration
    println!();
    println!("[6] Resilience configuration");
    let max_retries = std::env::var("MEMFUSE_MAX_RETRIES").unwrap_or_else(|_| "3".to_owned());
    let cb_threshold =
        std::env::var("MEMFUSE_CB_FAILURE_THRESHOLD").unwrap_or_else(|_| "5".to_owned());
    let ssrf_check = std::env::var("MEMFUSE_URL_SSRF_CHECK").unwrap_or_else(|_| "true".to_owned());
    println!("  max_retries={}", max_retries);
    println!("  cb_failure_threshold={}", cb_threshold);
    println!("  url_ssrf_check={}", ssrf_check);
    checks_passed += 1;

    // Summary
    println!();
    println!("=== Doctor Summary ===");
    println!("  passed: {}", checks_passed);
    println!("  warned: {}", checks_warned);
    println!("  failed: {}", checks_failed);
    println!();

    if checks_failed > 0 {
        println!(
            "❌ System has {} critical issues that must be resolved before production use.",
            checks_failed
        );
    } else if checks_warned > 0 {
        println!(
            "⚠️  System has {} warnings. Review and address before production deployment.",
            checks_warned
        );
    } else {
        println!("✅ All checks passed. System is healthy.");
    }
}
