use mfs_retrieval::{QueryPlanMode, QueryPlanner};

#[tokio::test]
async fn search_planner_emits_prioritized_queries_with_intent_metadata() {
    let planner = QueryPlanner::default();
    let plan = planner
        .plan_search(
            "Help me create an RFC for OAuth rollout",
            Some("recent workflow docs about auth incidents"),
        )
        .await;

    assert_eq!(plan.mode, QueryPlanMode::Search);
    assert!(plan.typed_queries.len() >= 2);
    assert!(plan.typed_queries.iter().any(|q| q.context_type == "skill"));
    assert!(plan.typed_queries.iter().all(|q| q.priority >= 1));
    assert!(plan.typed_queries.iter().all(|q| !q.intent.is_empty()));
}

#[tokio::test]
async fn search_planner_can_skip_non_retrieval_queries() {
    let planner = QueryPlanner::default();
    let plan = planner
        .plan_search("thanks", Some("recent session summary"))
        .await;

    assert_eq!(plan.mode, QueryPlanMode::Search);
    assert!(plan.typed_queries.is_empty());
    assert_eq!(plan.skip_reason.as_deref(), Some("non_retrieval_query"));
}
