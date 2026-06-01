-- Add user_confirmed flag to heuristic_rules (roadmap §5.4).
-- When a user explicitly confirms a rule via the confirm_rule MCP tool,
-- this flag is set to 1, making the rule exempt from automatic decay.
-- This is distinct from lifecycle_stage='confirmed' which is reached via
-- auto-promotion (evidence accumulation). Only user_confirmed=1 grants
-- decay exemption per §5.4: "用户显式确认的规则不参与自动衰减".

ALTER TABLE heuristic_rules ADD COLUMN user_confirmed INTEGER NOT NULL DEFAULT 0;

-- Backfill: existing rules that already reached lifecycle_stage='confirmed'
-- via auto-promotion should retain their decay exemption during the transition.
-- This preserves the semantic that auto-promoted rules were implicitly stable
-- under the old regime, and gives users time to explicitly confirm them via
-- the new confirm_rule MCP tool before they become subject to decay.
UPDATE heuristic_rules SET user_confirmed = 1 WHERE lifecycle_stage = 'confirmed';