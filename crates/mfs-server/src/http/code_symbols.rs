use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct ListCodeSymbolsQuery {
    pub projection_view_id: Option<String>,
    pub canonical_uri: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateCodeSymbolsRequest {
    pub symbols: Vec<CreateCodeSymbolItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateCodeSymbolItem {
    pub id: String,
    pub projection_view_id: String,
    pub canonical_uri: String,
    pub symbol_type: String,
    pub symbol_name: String,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub line_number: Option<i64>,
    pub agent_id: Option<String>,
    pub embedding_json: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SearchCodeSymbolsQuery {
    pub projection_view_id: String,
    pub q: String,
    pub limit: Option<usize>,
}

pub(super) async fn list_code_symbols(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListCodeSymbolsQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let view_id = query
        .projection_view_id
        .unwrap_or_else(|| resource_projection_view_id(&state.config));

    let symbols = metadata
        .get_code_symbols(&view_id, query.canonical_uri.as_deref())
        .map_err(AppError::from_error)?;
    let limit = query.limit.unwrap_or(100);
    let total_count = symbols.len();
    let items: Vec<_> = symbols.into_iter().take(limit).collect();

    Ok(Json(serde_json::json!({
        "items": items.clone(),
        "symbols": items,
        "next_cursor": null,
        "total_count": total_count,
        "count": total_count,
        "limit": limit,
    })))
}

pub(super) async fn create_code_symbols(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateCodeSymbolsRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();

    let records: Vec<mfs_metadata::CodeSymbolRecord<'_>> = request
        .symbols
        .iter()
        .map(|item| mfs_metadata::CodeSymbolRecord {
            id: &item.id,
            account_id: &state.config.account_id,
            user_id: &state.config.user_id,
            agent_id: item.agent_id.as_deref(),
            projection_view_id: &item.projection_view_id,
            canonical_uri: &item.canonical_uri,
            symbol_type: &item.symbol_type,
            symbol_name: &item.symbol_name,
            signature: item.signature.as_deref(),
            docstring: item.docstring.as_deref(),
            line_number: item.line_number,
            embedding_json: item.embedding_json.as_deref(),
        })
        .collect();

    metadata
        .insert_code_symbols_batch(&records)
        .map_err(AppError::from_error)?;

    append_audit(
        &state,
        "code_symbol.create_batch",
        Some(&format!("count:{}", request.symbols.len())),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "count": request.symbols.len(),
    })))
}

pub(super) async fn search_code_symbols(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchCodeSymbolsQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();

    let symbols = metadata
        .search_code_symbols(&query.projection_view_id, &query.q)
        .map_err(AppError::from_error)?;
    let limit = query.limit.unwrap_or(100);
    let total_count = symbols.len();
    let items: Vec<_> = symbols.into_iter().take(limit).collect();

    Ok(Json(serde_json::json!({
        "items": items.clone(),
        "symbols": items.clone(),
        "results": items,
        "next_cursor": null,
        "total_count": total_count,
        "count": total_count,
        "limit": limit,
    })))
}

pub(super) async fn delete_code_symbols(
    State(state): State<Arc<AppState>>,
    Path(view_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let deleted = metadata
        .delete_code_symbols_for_view(&view_id)
        .map_err(AppError::from_error)?;

    append_audit(
        &state,
        "code_symbol.delete",
        Some(&format!("view:{}", view_id)),
        Some(&format!("{{\"deleted\":{}}}", deleted)),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "deleted": deleted,
    })))
}
