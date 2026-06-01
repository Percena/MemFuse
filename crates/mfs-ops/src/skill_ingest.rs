use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use mfs_metadata::MetadataStore;
use mfs_semantic::{SemanticPipeline, SemanticPipelineConfig};
use mfs_types::IdentityContext;
use mfs_uri::MfsUri;
use mfs_workspace::{SourceProvenance, WorkspaceLayout};

use crate::{rebuild_metadata_entries_with_provenance, snapshot_record};

#[derive(serde::Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
}

fn parse_frontmatter(content: &str) -> Option<SkillFrontmatter> {
    let body = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
        .and_then(|rest| {
            let idx = rest.find("\n---\n").or_else(|| rest.find("\n---\r\n"))?;
            Some(&rest[..idx])
        })?;
    serde_yaml::from_str(body).ok()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SkillIngestResult {
    pub skill_name: String,
    pub skill_uri: String,
    pub indexed_documents: usize,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SkillSummary {
    pub skill_name: String,
    pub skill_uri: String,
    pub description: Option<String>,
}

pub async fn ingest_skill(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    input_path: &Path,
) -> Result<SkillIngestResult, Box<dyn std::error::Error>> {
    let canonical_input = std::fs::canonicalize(input_path)?;
    let skill_name = detect_skill_name(&canonical_input)?;
    let skill_uri = format!("mfs://agent/skills/{skill_name}");
    let target_uri = MfsUri::parse(&skill_uri)?;
    let target_path = WorkspaceLayout::new(workspace_root).path_for_uri(identity, &target_uri)?;

    if tokio::fs::try_exists(&target_path).await? {
        tokio::fs::remove_dir_all(&target_path).await?;
    }
    tokio::fs::create_dir_all(&target_path).await?;

    if canonical_input.is_file() {
        copy_skill_file(&canonical_input, &target_path.join("SKILL.md")).await?;
    } else {
        copy_skill_directory(&canonical_input, &target_path).await?;
    }

    let projection_view_id = format!(
        "tenant:{}:{}:agent:{}",
        identity.account_id(),
        identity.user_id(),
        identity.agent_space_name()
    );

    let ws = workspace_root.to_path_buf();
    let pvid = projection_view_id.clone();
    let skill_uri_clone = skill_uri.clone();
    let target_path_clone = target_path.clone();
    let semantic_index = tokio::task::spawn_blocking(move || {
        mfs_index::SqliteSemanticIndex::open_at(ws.join("_system").join("semantic.sqlite"))
    })
    .await
    .map_err(|e| -> Box<dyn std::error::Error> { format!("spawn_blocking: {e}").into() })?
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let pipeline = SemanticPipeline::new(SemanticPipelineConfig::from_env(8));
    let report = pipeline
        .process_root(
            &target_path_clone,
            &pvid,
            &skill_uri_clone,
            "skill",
            None,
            &semantic_index,
        )
        .await
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let indexed_documents = report.indexed_documents;
    let mode = format!("{:?}", report.mode).to_ascii_lowercase();

    let snapshot_id = format!(
        "skill:{}:{}",
        skill_name,
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let provenance = SourceProvenance {
        source_kind: "skill_local".to_owned(),
        source_identifier: canonical_input.to_string_lossy().into_owned(),
        source_snapshot_id: snapshot_id,
        projection_view_id,
        materialization_mode: "ingest".to_owned(),
        target_uri: skill_uri.clone(),
    };
    let _ = rebuild_metadata_entries_with_provenance(
        metadata,
        identity,
        &target_path,
        &skill_uri,
        Some(&provenance),
    )?;
    metadata.append_snapshot(&snapshot_record(&provenance))?;

    Ok(SkillIngestResult {
        skill_name,
        skill_uri,
        indexed_documents,
        mode,
    })
}

pub async fn list_skills(
    workspace_root: &Path,
    identity: &IdentityContext,
) -> Result<Vec<SkillSummary>, Box<dyn std::error::Error>> {
    let skills_root_uri = "mfs://agent/skills";
    let skills_root = WorkspaceLayout::new(workspace_root)
        .path_for_uri(identity, &MfsUri::parse(skills_root_uri)?)?;
    if !tokio::fs::try_exists(&skills_root).await? {
        return Ok(Vec::new());
    }

    let mut entries = tokio::fs::read_dir(&skills_root).await?;
    let mut skills = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let skill_name = entry.file_name().to_string_lossy().into_owned();
        let skill_md = entry.path().join("SKILL.md");
        let description = if tokio::fs::try_exists(&skill_md).await? {
            parse_description(&tokio::fs::read_to_string(&skill_md).await?)
        } else {
            None
        };
        skills.push(SkillSummary {
            skill_uri: format!("mfs://agent/skills/{skill_name}"),
            skill_name,
            description,
        });
    }
    skills.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
    Ok(skills)
}

fn detect_skill_name(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let skill_md = if path.is_file() {
        path.to_path_buf()
    } else {
        path.join("SKILL.md")
    };
    let content = std::fs::read_to_string(&skill_md)?;
    if let Some(fm) = parse_frontmatter(&content) {
        if let Some(name) = fm.name {
            let parsed = sanitize_skill_name(&name);
            if !parsed.is_empty() {
                return Ok(parsed);
            }
        }
    }

    let fallback = if path.is_file() {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("skill")
    } else {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("skill")
    };
    Ok(sanitize_skill_name(fallback))
}

fn parse_description(content: &str) -> Option<String> {
    let fm = parse_frontmatter(content)?;
    fm.description.filter(|d| !d.is_empty())
}

async fn copy_skill_file(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(from, to).await?;
    Ok(())
}

async fn copy_skill_directory(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut stack = vec![from.to_path_buf()];
    while let Some(current) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&current).await?;
        while let Some(entry) = entries.next_entry().await? {
            let source_path = entry.path();
            let relative = source_path.strip_prefix(from)?;
            let destination = to.join(relative);
            if entry.file_type().await?.is_dir() {
                tokio::fs::create_dir_all(&destination).await?;
                stack.push(source_path);
            } else {
                if let Some(parent) = destination.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::copy(&source_path, &destination).await?;
            }
        }
    }
    Ok(())
}

fn sanitize_skill_name(raw: &str) -> String {
    let mut name = raw
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    name.trim_matches('-').to_owned()
}
