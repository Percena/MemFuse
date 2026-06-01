use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::path::{Path, PathBuf};

use mfs_types::IdentityContext;
use mfs_uri::{MfsUri, UriError};
use tokio::fs;

use crate::layout::{WorkspaceLayout, WorkspaceLayoutError};
use crate::materialize::{MaterializationResult, MaterializeError, Materializer};

const ABSTRACT_FILE_NAME: &str = ".abstract.md";
const OVERVIEW_FILE_NAME: &str = ".overview.md";

#[derive(Debug)]
pub struct WorkspaceFs {
    identity: IdentityContext,
    materialized: MaterializationResult,
    layout: WorkspaceLayout,
}

impl WorkspaceFs {
    fn from_materialized(identity: IdentityContext, materialized: MaterializationResult) -> Self {
        let layout = WorkspaceLayout::new(materialized.workspace_root.clone());
        Self {
            identity,
            materialized,
            layout,
        }
    }

    pub async fn from_localfs_source(
        workspace_root: impl AsRef<Path>,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        source_path: &str,
        target_uri: &str,
    ) -> Result<Self, FsError> {
        let identity = IdentityContext::new(account_id, user_id, agent_id);
        let materialized = Materializer::new(workspace_root)
            .materialize_localfs(&identity, source_path, target_uri)
            .await
            .map_err(FsError::Materialize)?;

        Ok(Self::from_materialized(identity, materialized))
    }

    pub fn open_existing(
        workspace_root: impl AsRef<Path>,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Self, FsError> {
        Self::open_existing_for_uri(workspace_root, account_id, user_id, agent_id, None)
    }

    pub fn open_existing_for_uri(
        workspace_root: impl AsRef<Path>,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        uri: Option<&str>,
    ) -> Result<Self, FsError> {
        let identity = IdentityContext::new(account_id, user_id, agent_id);
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let (target_path, target_uri) = projection_scope(&workspace_root, &identity, uri)?;
        let materialized = MaterializationResult::managed(
            workspace_root.clone(),
            target_path,
            crate::materialize::SourceProvenance {
                source_kind: "managed".to_owned(),
                source_identifier: workspace_root.to_string_lossy().into_owned(),
                source_snapshot_id: "existing-workspace".to_owned(),
                projection_view_id: projection_view_id(&identity, &target_uri),
                materialization_mode: "managed".to_owned(),
                target_uri,
            },
        );
        Ok(Self::from_materialized(identity, materialized))
    }

    pub async fn from_git_source(
        workspace_root: impl AsRef<Path>,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        source_path: &str,
        target_uri: &str,
    ) -> Result<Self, FsError> {
        let identity = IdentityContext::new(account_id, user_id, agent_id);
        let materialized = Materializer::new(workspace_root)
            .materialize_git(&identity, source_path, target_uri)
            .await
            .map_err(FsError::Materialize)?;

        Ok(Self::from_materialized(identity, materialized))
    }

    pub async fn from_fixture(
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        source_path: &str,
    ) -> Result<Self, FsError> {
        let identity = IdentityContext::new(account_id, user_id, agent_id);
        let materialized = Materializer::for_tests()
            .materialize_localfs(&identity, source_path, "mfs://resources/localfs/docs")
            .await
            .map_err(FsError::Materialize)?;
        Ok(Self::from_materialized(identity, materialized))
    }

    pub async fn ls(&self, uri: &str) -> Result<Vec<DirEntry>, FsError> {
        let path = self.resolve_uri(uri)?;
        let mut entries = fs::read_dir(&path)
            .await
            .map_err(|source| FsError::io("list directory", &path, source))?;
        let mut listing = Vec::new();

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|source| FsError::io("advance directory iterator", &path, source))?
        {
            let file_type = entry
                .file_type()
                .await
                .map_err(|source| FsError::io("inspect directory entry", entry.path(), source))?;
            listing.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: file_type.is_dir(),
            });
        }

        listing.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(listing)
    }

    pub async fn tree(&self, uri: &str, depth: usize) -> Result<TreeNode, FsError> {
        let path = self.resolve_uri(uri)?;
        self.build_tree(&path, depth).await
    }

    pub async fn stat(&self, uri: &str) -> Result<FileStat, FsError> {
        let path = self.resolve_uri(uri)?;
        let metadata = fs::metadata(&path)
            .await
            .map_err(|source| FsError::io("stat path", &path, source))?;

        Ok(FileStat {
            path,
            is_dir: metadata.is_dir(),
            size_bytes: metadata.len(),
        })
    }

    pub async fn read(&self, uri: &str) -> Result<String, FsError> {
        let path = self.resolve_uri(uri)?;
        fs::read_to_string(&path)
            .await
            .map_err(|source| FsError::io("read file", &path, source))
    }

    pub async fn mkdir(&self, uri: &str) -> Result<(), FsError> {
        let path = self.resolve_owned_write_uri(uri)?;
        fs::create_dir_all(&path)
            .await
            .map_err(|source| FsError::io("create directory", &path, source))
    }

    pub async fn write_text(&self, uri: &str, content: &str) -> Result<(), FsError> {
        let path = self.resolve_owned_write_uri(uri)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|source| FsError::io("create parent directory", parent, source))?;
        }
        fs::write(&path, content)
            .await
            .map_err(|source| FsError::io("write file", &path, source))
    }

    pub async fn move_path(&self, from_uri: &str, to_uri: &str) -> Result<(), FsError> {
        let from = self.resolve_owned_write_uri(from_uri)?;
        let to = self.resolve_owned_write_uri(to_uri)?;
        let from_root = root_name(from_uri)?;
        let to_root = root_name(to_uri)?;
        if from_root != to_root {
            return Err(FsError::WritePolicy(
                "move across MFS roots is not supported".to_owned(),
            ));
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|source| FsError::io("create move destination parent", parent, source))?;
        }
        fs::rename(&from, &to)
            .await
            .map_err(|source| FsError::io("move path", &from, source))
    }

    pub async fn remove_path(&self, uri: &str) -> Result<(), FsError> {
        let path = self.resolve_owned_write_uri(uri)?;
        let metadata = fs::metadata(&path)
            .await
            .map_err(|source| FsError::io("stat remove path", &path, source))?;
        if metadata.is_dir() {
            fs::remove_dir_all(&path)
                .await
                .map_err(|source| FsError::io("remove directory", &path, source))
        } else {
            fs::remove_file(&path)
                .await
                .map_err(|source| FsError::io("remove file", &path, source))
        }
    }

    pub async fn abstract_text(&self, uri: &str) -> Result<String, FsError> {
        self.read_summary(uri, ABSTRACT_FILE_NAME).await
    }

    pub async fn overview_text(&self, uri: &str) -> Result<String, FsError> {
        self.read_summary(uri, OVERVIEW_FILE_NAME).await
    }

    pub async fn glob(&self, uri: &str, pattern: &str) -> Result<Vec<String>, FsError> {
        let root = self.resolve_uri(uri)?;
        let metadata = fs::metadata(&root)
            .await
            .map_err(|source| FsError::io("stat glob root", &root, source))?;
        let root_uri = uri.trim_end_matches('/').to_owned();
        let mut matches = Vec::new();

        if metadata.is_file() {
            let candidate = root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            if glob_matches(pattern, &candidate) {
                matches.push(root_uri);
            }
            return Ok(matches);
        }

        self.collect_glob_matches(&root, &root, &root_uri, pattern, &mut matches)
            .await?;
        matches.sort();
        Ok(matches)
    }

    fn resolve_uri(&self, uri: &str) -> Result<PathBuf, FsError> {
        let uri = MfsUri::parse(uri).map_err(FsError::Uri)?;
        self.layout
            .path_for_uri(&self.identity, &uri)
            .map_err(FsError::Layout)
    }

    fn resolve_owned_write_uri(&self, uri: &str) -> Result<PathBuf, FsError> {
        let uri = validate_owned_write_uri(uri)?;
        self.layout
            .path_for_uri(&self.identity, &uri)
            .map_err(FsError::Layout)
    }

    async fn read_summary(&self, uri: &str, file_name: &str) -> Result<String, FsError> {
        let path = self.resolve_uri(uri)?;
        let metadata = fs::metadata(&path)
            .await
            .map_err(|source| FsError::io("stat summary path", &path, source))?;
        let summary_path = if metadata.is_dir() {
            path.join(file_name)
        } else {
            let file_name_prefix = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            path.with_file_name(format!("{file_name_prefix}{file_name}"))
        };

        Ok(fs::read_to_string(&summary_path)
            .await
            .ok()
            .unwrap_or_default())
    }

    async fn build_tree(&self, path: &Path, depth: usize) -> Result<TreeNode, FsError> {
        let metadata = fs::metadata(path)
            .await
            .map_err(|source| FsError::io("stat tree path", path, source))?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                self.materialized
                    .target_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            });

        if !metadata.is_dir() || depth == 0 {
            return Ok(TreeNode {
                name,
                is_dir: metadata.is_dir(),
                children: Vec::new(),
            });
        }

        let mut entries = fs::read_dir(path)
            .await
            .map_err(|source| FsError::io("list tree directory", path, source))?;
        let mut children = Vec::new();

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|source| FsError::io("advance tree iterator", path, source))?
        {
            let child_path = entry.path();
            children.push(Box::pin(self.build_tree(&child_path, depth - 1)).await?);
        }

        children.sort_by(|left, right| left.name.cmp(&right.name));

        Ok(TreeNode {
            name,
            is_dir: true,
            children,
        })
    }

    async fn collect_glob_matches(
        &self,
        root: &Path,
        current: &Path,
        root_uri: &str,
        pattern: &str,
        matches: &mut Vec<String>,
    ) -> Result<(), FsError> {
        let mut stack = vec![current.to_path_buf()];

        while let Some(path) = stack.pop() {
            let metadata = fs::metadata(&path)
                .await
                .map_err(|source| FsError::io("stat glob path", &path, source))?;

            if path != root {
                let relative = path
                    .strip_prefix(root)
                    .expect("glob path stays under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                if glob_matches(pattern, &relative) {
                    matches.push(format!("{}/{}", root_uri, relative));
                }
            }

            if !metadata.is_dir() {
                continue;
            }

            let mut entries = fs::read_dir(&path)
                .await
                .map_err(|source| FsError::io("read glob directory", &path, source))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|source| FsError::io("iterate glob directory", &path, source))?
            {
                stack.push(entry.path());
            }
        }

        Ok(())
    }

    pub fn projection_root(&self) -> &Path {
        &self.materialized.target_path
    }

    pub fn workspace_root(&self) -> &Path {
        &self.materialized.workspace_root
    }

    pub fn projection_uri(&self) -> &str {
        &self.materialized.provenance.target_uri
    }
}

fn projection_scope(
    workspace_root: &Path,
    identity: &IdentityContext,
    uri: Option<&str>,
) -> Result<(PathBuf, String), FsError> {
    let parsed = match uri {
        Some(uri) => Some(MfsUri::parse(uri).map_err(FsError::Uri)?),
        None => None,
    };

    let projection_uri = match parsed.as_ref() {
        Some(uri) => format!("mfs://{}/{}", uri.root(), uri.canonical_path())
            .trim_end_matches('/')
            .to_owned(),
        None => "mfs://resources".to_owned(),
    };
    let parsed_projection = MfsUri::parse(&projection_uri).map_err(FsError::Uri)?;
    let target_path = WorkspaceLayout::new(workspace_root)
        .path_for_uri(identity, &parsed_projection)
        .map_err(FsError::Layout)?;
    Ok((target_path, projection_uri))
}

fn projection_view_id(identity: &IdentityContext, projection_uri: &str) -> String {
    if projection_uri.starts_with("mfs://resources") {
        return format!(
            "tenant:{}:{}:resources",
            identity.account_id(),
            identity.user_id()
        );
    }
    if projection_uri.starts_with("mfs://user") {
        return format!(
            "tenant:{}:{}:user",
            identity.account_id(),
            identity.user_id()
        );
    }
    format!(
        "tenant:{}:{}:agent:{}",
        identity.account_id(),
        identity.user_id(),
        identity.agent_space_name()
    )
}

fn glob_matches(pattern: &str, candidate: &str) -> bool {
    let pattern_segments = if pattern.is_empty() {
        Vec::new()
    } else {
        pattern
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect()
    };
    let candidate_segments = if candidate.is_empty() {
        Vec::new()
    } else {
        candidate
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect()
    };
    glob_segments_match(&pattern_segments, &candidate_segments)
}

fn glob_segments_match(pattern: &[&str], candidate: &[&str]) -> bool {
    if pattern.is_empty() {
        return candidate.is_empty();
    }

    if pattern[0] == "**" {
        if glob_segments_match(&pattern[1..], candidate) {
            return true;
        }
        return !candidate.is_empty() && glob_segments_match(pattern, &candidate[1..]);
    }

    if candidate.is_empty() {
        return false;
    }

    segment_matches(pattern[0], candidate[0]) && glob_segments_match(&pattern[1..], &candidate[1..])
}

fn segment_matches(pattern: &str, candidate: &str) -> bool {
    let pattern = pattern.as_bytes();
    let candidate = candidate.as_bytes();
    let mut pattern_index = 0;
    let mut candidate_index = 0;
    let mut star_index = None;
    let mut match_index = 0;

    while candidate_index < candidate.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?'
                || pattern[pattern_index] == candidate[candidate_index])
        {
            pattern_index += 1;
            candidate_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            match_index = candidate_index;
            pattern_index += 1;
        } else if let Some(star_index) = star_index {
            pattern_index = star_index + 1;
            match_index += 1;
            candidate_index = match_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub name: String,
    pub is_dir: bool,
    pub children: Vec<TreeNode>,
}

#[derive(Debug)]
pub enum FsError {
    Uri(UriError),
    Layout(WorkspaceLayoutError),
    Materialize(MaterializeError),
    WritePolicy(String),
    IoRaw(io::Error),
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl FsError {
    fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}

impl Display for FsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uri(source) => write!(f, "invalid MFS URI: {source}"),
            Self::Layout(source) => write!(f, "workspace layout error: {source}"),
            Self::Materialize(source) => write!(f, "failed to materialize fixture: {source}"),
            Self::WritePolicy(message) => write!(f, "write policy error: {message}"),
            Self::IoRaw(source) => write!(f, "io error: {source}"),
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} '{}': {source}", path.display()),
        }
    }
}

impl Error for FsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Uri(source) => Some(source),
            Self::Layout(source) => Some(source),
            Self::Materialize(source) => Some(source),
            Self::WritePolicy(_) => None,
            Self::IoRaw(source) => Some(source),
            Self::Io { source, .. } => Some(source),
        }
    }
}

fn validate_owned_write_uri(uri: &str) -> Result<MfsUri, FsError> {
    let parsed = MfsUri::parse(uri).map_err(FsError::Uri)?;
    match parsed.root() {
        "user" | "agent" => Ok(parsed),
        "resources" | "session" => Err(FsError::WritePolicy(format!(
            "MFS write plane does not allow writes to root '{}'",
            parsed.root()
        ))),
        root => Err(FsError::WritePolicy(format!(
            "unsupported MFS root '{root}' for write operations"
        ))),
    }
}

fn root_name(uri: &str) -> Result<String, FsError> {
    Ok(MfsUri::parse(uri).map_err(FsError::Uri)?.root().to_owned())
}
