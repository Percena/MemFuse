//! Skill export subcommand for mfs-cli.
//!
//! **DEPRECATED**: This subcommand generates a minimal skill bundle based on
//! the Rust CLI's offline retrieval commands (ls, read, abstract, overview,
//! search, find, grep). The SDK skill (`sdk/src/skills/memfuse/SKILL.md`)
//! is now the primary Agent integration path — it uses the `memfuse` Node.js
//! CLI (110 commands, 100% HTTP API coverage) with `allowed-tools: Bash(memfuse:*)`.
//!
//! This export remains available for offline/diagnostic scenarios where only
//! the Rust CLI is available, but Agent integrations should prefer the SDK skill.
//!
//! Changes made during deprecation pass (2026-05-06):
//! - Skill name: `mfs-query` → `memfuse-offline` (avoid collision with SDK skill)
//! - Default port: 3000 → 8720 (consistency with actual server)
//! - Added deprecation warning in generated SKILL.md

use clap::{Subcommand, ValueEnum};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_TARGET_URI: &str = "mfs://resources/<logical-name>";
const SKILL_NAME: &str = "memfuse-offline";

#[derive(Subcommand, Debug)]
pub enum SkillCommands {
    /// Export a skill bundle for offline retrieval scenarios.
    /// **DEPRECATED**: Prefer the SDK skill for Agent integrations.
    Export {
        #[arg(long, value_enum)]
        platform: SkillPlatform,
        #[arg(long)]
        output_dir: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum SkillPlatform {
    Codex,
    ClaudeCode,
}

impl SkillPlatform {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderedFile {
    pub relative_path: PathBuf,
    pub contents: String,
}

#[derive(Clone, Debug)]
pub struct RenderedSkillBundle {
    pub files: Vec<RenderedFile>,
}

impl RenderedSkillBundle {
    pub fn skill_markdown(&self) -> &str {
        self.files
            .iter()
            .find(|file| file.relative_path == Path::new("SKILL.md"))
            .map(|file| file.contents.as_str())
            .expect("skill bundle missing SKILL.md")
    }

    pub fn write_to(
        &self,
        output_dir: &Path,
        platform: SkillPlatform,
    ) -> Result<(), std::io::Error> {
        let root = output_dir
            .join("skills")
            .join(platform.slug())
            .join(SKILL_NAME);
        for file in &self.files {
            let path = root.join(&file.relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, &file.contents)?;
        }
        Ok(())
    }
}

pub fn render_skill_bundle(platform: SkillPlatform) -> RenderedSkillBundle {
    let mut files = vec![
        RenderedFile {
            relative_path: PathBuf::from("SKILL.md"),
            contents: build_skill_markdown(platform),
        },
        RenderedFile {
            relative_path: PathBuf::from("references/workflow.md"),
            contents: build_workflow_reference(),
        },
        RenderedFile {
            relative_path: PathBuf::from("references/commands.md"),
            contents: build_commands_reference(),
        },
        RenderedFile {
            relative_path: PathBuf::from("shell/mfs-env.sh"),
            contents: build_shell_template(),
        },
    ];

    if platform == SkillPlatform::Codex {
        files.push(RenderedFile {
            relative_path: PathBuf::from("agents/openai.yaml"),
            contents: build_codex_metadata(),
        });
    }

    RenderedSkillBundle { files }
}

fn build_skill_markdown(platform: SkillPlatform) -> String {
    let platform_title = platform.title();
    let shell_template = build_shell_template();

    format!(
        r"# MFS Query Skill For {platform_title} (Offline/Diagnostic Use Only)

> **Note**: This skill covers only offline retrieval via the Rust CLI (`mfs-cli`).
> For Agent integrations, prefer the SDK skill which uses `memfuse` CLI (110 commands,
> 100% HTTP API coverage) with `allowed-tools: Bash(memfuse:*)`.

Use MFS before raw filesystem exploration whenever the request is about managed project knowledge.

## Workflow
1. Start with `/search` for natural-language questions or multi-hop context discovery.
2. Use `/find` when you already know the resource scope and want tighter retrieval.
3. If MFS returns a directory URI, treat that URI as the next-round target and run `/search` or `/find` again inside that scope.
4. Use `/grep` only after MFS returns candidate paths and you need literal verification.
5. Use `/ls` or `/tree` to inspect a returned directory URI.
6. Use `/read`, `/abstract`, or `/overview` to inspect a returned file or summary URI.
7. Repeat the narrowing loop until the evidence is sufficient.

## Default Target
`{DEFAULT_TARGET_URI}`

## Progressive Example
```text
Round 1:
- search target={DEFAULT_TARGET_URI}
- identify a high-signal directory URI from MFS results

Round 2:
- find target=<returned-directory-uri>
- inspect ls/tree for that narrowed scope

Round 3:
- grep or read against <returned-file-uri>
- stop only when the result is backed by concrete file evidence
```

## Guardrails
- Do not replace MFS semantic retrieval with ad-hoc grep alone.
- Treat MFS result URIs as the canonical follow-up scope.
- If semantic processing mode is degraded, still report that to the user before drawing strong conclusions.

## Shell Template
```bash
{shell_template}```

## References
- `references/workflow.md`
- `references/commands.md`
- `shell/mfs-env.sh`
"
    )
}

fn build_shell_template() -> String {
    format!(
        r#"export MEMFUSE_SERVER_URL=http://127.0.0.1:8720
export MEMFUSE_TARGET_URI={DEFAULT_TARGET_URI}

# broad semantic retrieval
curl --get "$MEMFUSE_SERVER_URL/search" \
  --data-urlencode "query=how does authentication rotation work" \
  --data-urlencode "target=$MEMFUSE_TARGET_URI"

# narrower scoped lookup
curl --get "$MEMFUSE_SERVER_URL/find" \
  --data-urlencode "query=rotation" \
  --data-urlencode "target=$MEMFUSE_TARGET_URI"

# narrow to a returned directory URI and search again
export MEMFUSE_NARROW_URI="$MEMFUSE_TARGET_URI/<returned-directory>"
curl --get "$MEMFUSE_SERVER_URL/search" \
  --data-urlencode "query=device binding rotation" \
  --data-urlencode "target=$MEMFUSE_NARROW_URI"

# literal verification after MFS points you at a resource
curl --get "$MEMFUSE_SERVER_URL/grep" \
  --data-urlencode "query=device binding" \
  --data-urlencode "target=$MEMFUSE_NARROW_URI"

# inspect a directory result
curl --get "$MEMFUSE_SERVER_URL/ls" \
  --data-urlencode "uri=$MEMFUSE_NARROW_URI"
curl --get "$MEMFUSE_SERVER_URL/tree" \
  --data-urlencode "uri=$MEMFUSE_NARROW_URI"

# inspect a file result
curl --get "$MEMFUSE_SERVER_URL/read" \
  --data-urlencode "uri=$MEMFUSE_NARROW_URI/<path/to/file.md>"
"#
    )
}

fn build_workflow_reference() -> String {
    format!(
        r"# MFS Query Workflow

Use MFS as the first retrieval layer for managed project knowledge.

1. Start broad with `search` or `find` against `{DEFAULT_TARGET_URI}`.
2. Keep the best returned MFS URI as the next-round scope.
3. Inspect that scope with `ls`, `tree`, `abstract`, `overview`, or `read`.
4. Use `grep` only after MFS has narrowed the scope.
5. Do not finalize an answer until it is backed by file evidence.
"
    )
}

fn build_commands_reference() -> String {
    r"# MFS Command Reference (Offline/Diagnostic Subset)

- `search`: semantic or natural-language retrieval
- `find`: tighter scoped retrieval when you already know the domain
- `grep`: literal verification after MFS narrows the scope
- `ls`: inspect a directory URI
- `tree`: inspect nested directory structure
- `abstract`: read the L0 summary for a directory or file
- `overview`: read the L1 summary for a directory or file
- `read`: inspect the full file body
"
    .to_owned()
}

fn build_codex_metadata() -> String {
    r"display_name: MFS Query (Offline)
short_description: MFS-first retrieval workflow for managed project knowledge (offline/diagnostic subset only).
default_prompt: |
  Use MFS retrieval before raw filesystem exploration. Start with search or find, narrow scope with returned MFS URIs, and finish with file-backed evidence.
"
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn offline_skill_export_uses_non_conflicting_skill_name() {
        let temp = std::env::temp_dir().join(format!("mfs-cli-skill-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);

        let bundle = render_skill_bundle(SkillPlatform::Codex);
        bundle.write_to(&temp, SkillPlatform::Codex).unwrap();

        assert!(temp.join("skills/codex/memfuse-offline/SKILL.md").exists());
        assert!(!temp.join("skills/codex/memfuse/SKILL.md").exists());

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn offline_skill_markdown_declares_deprecated_scope() {
        let bundle = render_skill_bundle(SkillPlatform::ClaudeCode);
        let markdown = bundle.skill_markdown();

        assert!(markdown.contains("Offline/Diagnostic Use Only"));
        assert!(markdown.contains("prefer the SDK skill"));
    }
}
