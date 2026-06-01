-- Migration 0015: Overlay Canvas canonical refs dual-write columns
-- Adds affected_node_refs and affected_edge_refs columns to active_overlays
-- These store canonical Canvas refs (canvas://...) as JSON arrays
-- alongside the existing affected_nodes/affected_edges which store local database IDs.
-- This enables SaaS cross-domain references while preserving backward compatibility.

ALTER TABLE active_overlays ADD COLUMN affected_node_refs TEXT NOT NULL DEFAULT '[]';
ALTER TABLE active_overlays ADD COLUMN affected_edge_refs TEXT NOT NULL DEFAULT '[]';