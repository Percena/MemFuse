#!/usr/bin/env bash
# Integration test script for T2H Phase 1: Heuristic Rules + Retrieval + MCP Injection
# Tests the full chain: HTTP CRUD → retrieval → L0/L1 injection

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEMP_DIR="$(mktemp -d)"
HEURISTIC_TEST_DB="$TEMP_DIR/metadata.sqlite"

echo "=== T2H Phase 1 Integration Tests ==="
echo "Temp dir: $TEMP_DIR"

# Build the project first
echo "--- Building project ---"
cd "$PROJECT_ROOT"
cargo build --release 2>&1 | tail -5

# Start server in background
export MEMFUSE_WORKSPACE_ROOT="$TEMP_DIR"
export MEMFUSE_ACCOUNT_ID="test_acct"
export MEMFUSE_USER_ID="test_user"
export MEMFUSE_AGENT_ID="test_agent"
export MEMFUSE_SOURCE_KIND="localfs"
export MEMFUSE_SOURCE_PATH="$TEMP_DIR/source"
export MEMFUSE_TARGET_URI="mfs://resources/localfs/docs"
export MEMFUSE_OPENAI_API_KEY="placeholder-key"  # deterministic fallback mode

mkdir -p "$TEMP_DIR/source" "$TEMP_DIR/_system"

SERVER_PORT=18234
SERVER_URL="http://localhost:$SERVER_PORT"

echo "--- Starting server on port $SERVER_PORT ---"
"$PROJECT_ROOT/target/release/mfs-server" &
SERVER_PID=$!
sleep 2

# Check server health
echo "--- Checking server health ---"
curl -sf "$SERVER_URL/health" | head -3
echo ""

# Test 1: Create heuristic rule (draft)
echo ""
echo "=== Test 1: Create heuristic rule ==="
RULE1=$(curl -sf -X POST "$SERVER_URL/heuristics/rules" \
  -H 'Content-Type: application/json' \
  -d '{
    "rule_text": "User prefers custom error enums over anyhow in Rust CLI tools",
    "tags": ["domain:cli", "language:rust", "topic:error-handling"],
    "counter_examples": ["But for quick scripts, user tolerates anyhow"],
    "lifecycle_stage": "draft"
  }')
echo "Rule1: $RULE1"
RULE1_ID=$(echo "$RULE1" | python3 -c "import json,sys; print(json.load(sys.stdin)['rule_id'])")

# Test 2: Create another rule (confirmed)
echo ""
echo "=== Test 2: Create confirmed rule ==="
RULE2=$(curl -sf -X POST "$SERVER_URL/heuristics/rules" \
  -H 'Content-Type: application/json' \
  -d '{
    "rule_text": "User prefers concrete types over trait abstractions in backend Rust",
    "tags": ["domain:backend", "language:rust"],
    "counter_examples": ["But for public library crates, user uses trait-based abstractions"],
    "lifecycle_stage": "confirmed"
  }')
echo "Rule2: $RULE2"
RULE2_ID=$(echo "$RULE2" | python3 -c "import json,sys; print(json.load(sys.stdin)['rule_id'])")

# Test 3: Create instance (explicit negation signal)
echo ""
echo "=== Test 3: Create heuristic instance ==="
INSTANCE=$(curl -sf -X POST "$SERVER_URL/heuristics/instances" \
  -H 'Content-Type: application/json' \
  -d '{
    "context_summary": "User working on Rust CLI, asked for error handling",
    "user_reaction": "Don't use anyhow, I want custom error enum",
    "signal_type": "explicit_negation",
    "tags": ["domain:rust", "domain:cli", "topic:error-handling"],
    "agent_proposal": "Suggested using anyhow for error handling",
    "outcome": "User wrote custom AppError enum"
  }')
echo "Instance: $INSTANCE"

# Test 4: List heuristic rules
echo ""
echo "=== Test 4: List heuristic rules ==="
RULES=$(curl -sf "$SERVER_URL/heuristics/rules")
echo "Rules count: $(echo "$RULES" | python3 -c "import json,sys; print(json.load(sys.stdin)['total'])")"

# Test 5: Get specific rule
echo ""
echo "=== Test 5: Get heuristic rule ==="
RULE_DETAIL=$(curl -sf "$SERVER_URL/heuristics/rules/$RULE2_ID")
echo "Rule lifecycle: $(echo "$RULE_DETAIL" | python3 -c "import json,sys; print(json.load(sys.stdin)['lifecycle_stage'])")"

# Test 6: Promote rule
echo ""
echo "=== Test 6: Promote rule to candidate ==="
PROMOTE=$(curl -sf -X POST "$SERVER_URL/heuristics/rules/$RULE1_ID/promote" \
  -H 'Content-Type: application/json' \
  -d '{"new_stage": "candidate"}')
echo "Promote result: $PROMOTE"

# Test 7: Retrieve heuristics by query
echo ""
echo "=== Test 7: Retrieve heuristics ==="
RETRIEVED=$(curl -sf -X POST "$SERVER_URL/heuristics/retrieve" \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "Rust error handling CLI tools",
    "tags": ["domain:cli", "language:rust"],
    "top_k": 5
  }')
echo "Retrieved count: $(echo "$RETRIEVED" | python3 -c "import json,sys; print(json.load(sys.stdin)['total'])")"

# Test 8: L0 confirmed rules
echo ""
echo "=== Test 8: L0 confirmed rules ==="
L0=$(curl -sf -X POST "$SERVER_URL/heuristics/l0-confirmed" \
  -H 'Content-Type: application/json' \
  -d '{"max_rules": 3}')
echo "L0 rules count: $(echo "$L0" | python3 -c "import json,sys; print(json.load(sys.stdin)['total'])")"

# Test 9: resolve_memory_context now includes heuristics
echo ""
echo "=== Test 9: resolve_memory_context includes heuristics ==="
CONTEXT=$(curl -sf -X POST "$SERVER_URL/context/resolve" \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "Rust backend error handling",
    "token_budget": 1500
  }')
echo "Context has behavioral_heuristics: $(echo "$CONTEXT" | python3 -c "
import json,sys
data = json.load(sys.stdin)
h = data.get('sections', {}).get('behavioral_heuristics', [])
print(f'YES ({len(h)} rules)' if h else 'NO')
")"

# Test 10: List instances
echo ""
echo "=== Test 10: List heuristic instances ==="
INSTANCES=$(curl -sf "$SERVER_URL/heuristics/instances?status=open")
echo "Instances count: $(echo "$INSTANCES" | python3 -c "import json,sys; print(json.load(sys.stdin)['total'])")"

# Cleanup
echo ""
echo "--- Cleanup ---"
kill $SERVER_PID 2>/dev/null || true
rm -rf "$TEMP_DIR"

echo ""
echo "=== All T2H Phase 1 integration tests completed ==="