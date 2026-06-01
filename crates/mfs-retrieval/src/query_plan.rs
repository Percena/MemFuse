use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QueryPlanMode {
    Find,
    Search,
}

impl QueryPlanMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Find => "find",
            Self::Search => "search",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedQuery {
    pub query: String,
    pub context_type: String,
    pub intent: String,
    pub priority: u8,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryPlan {
    pub mode: QueryPlanMode,
    pub typed_queries: Vec<PlannedQuery>,
    pub skip_reason: Option<String>,
}
