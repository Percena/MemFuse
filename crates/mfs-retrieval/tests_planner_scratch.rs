use mfs_retrieval::QueryPlanner;

fn main() {
    let mut planner = QueryPlanner::new(true); // enable llm
    std::env::set_var("OPENAI_API_KEY", ""); // ensure it's not totally broken if local
    let queries = planner.plan_search("我想清理一下当前的冗余文件，顺便看下之前设定的备份策略", Some("user is trying to manage their disk space"));
    println!("{:#?}", queries);
}