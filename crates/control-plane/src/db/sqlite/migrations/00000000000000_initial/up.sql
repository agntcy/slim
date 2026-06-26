CREATE TABLE IF NOT EXISTS nodes (
    id TEXT NOT NULL PRIMARY KEY,
    group_name TEXT,
    conn_details TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    last_updated BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS routes (
    id TEXT NOT NULL PRIMARY KEY,
    source_node_id TEXT NOT NULL,
    source_group TEXT NOT NULL DEFAULT '',
    dest_node_id TEXT NOT NULL,
    dest_group TEXT NOT NULL DEFAULT '',
    link_id TEXT,
    component0 TEXT NOT NULL,
    component1 TEXT NOT NULL,
    component2 TEXT NOT NULL,
    component_id TEXT,
    status INTEGER NOT NULL,
    status_msg TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    last_updated BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS links (
    link_id TEXT NOT NULL,
    source_node_id TEXT NOT NULL,
    source_group TEXT NOT NULL DEFAULT '',
    dest_node_id TEXT NOT NULL,
    dest_group TEXT NOT NULL,
    dest_endpoint TEXT NOT NULL,
    conn_config_data TEXT NOT NULL,
    status INTEGER NOT NULL,
    status_msg TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    last_updated BIGINT NOT NULL,
    PRIMARY KEY (link_id, source_node_id, dest_node_id, dest_endpoint)
);

-- Hot-path indices for routes.
--
-- source_node_id: get_routes_for_node_id, get_route_for_src_dest_name,
--                 filter_routes_by_src_dest, get_destination_node_id_for_name
CREATE INDEX IF NOT EXISTS idx_routes_source_node_id ON routes (source_node_id);

-- dest_node_id: get_routes_for_dest_node_id, get_routes_for_dest_node_id_and_name,
--               filter_routes_by_src_dest
CREATE INDEX IF NOT EXISTS idx_routes_dest_node_id ON routes (dest_node_id);

-- link_id: get_routes_by_link_id (called after every successful link apply)
CREATE INDEX IF NOT EXISTS idx_routes_link_id ON routes (link_id);

-- Composite covering the name lookup fields so the planner can satisfy
-- get_route_for_src_dest_name and get_destination_node_id_for_name with a
-- single index range scan rather than a scan + filter.
CREATE INDEX IF NOT EXISTS idx_routes_src_name
    ON routes (source_node_id, component0, component1, component2, component_id);

-- Hot-path indices for links.
--
-- source_node_id: get_links_for_node (source side), get_link_for_source_and_endpoint,
--                 find_link_between_nodes (forward direction)
CREATE INDEX IF NOT EXISTS idx_links_source_node_id ON links (source_node_id);

-- dest_node_id: get_links_for_node (dest side), find_link_between_nodes (reverse)
CREATE INDEX IF NOT EXISTS idx_links_dest_node_id ON links (dest_node_id);

-- link_id: get_link lookup by link_id
CREATE INDEX IF NOT EXISTS idx_links_link_id ON links (link_id);

-- Composite for the endpoint-reuse check in add_link and get_link_for_source_and_endpoint.
CREATE INDEX IF NOT EXISTS idx_links_src_endpoint
    ON links (source_node_id, dest_endpoint);

-- At most one non-deleted link between any ordered node pair (regardless of direction).
-- Only applies to claimed links (dest_node_id != '').
-- status=4 is LinkStatus::Deleted.
CREATE UNIQUE INDEX IF NOT EXISTS idx_links_unique_active_pair
    ON links (
        CASE WHEN source_node_id < dest_node_id THEN source_node_id ELSE dest_node_id END,
        CASE WHEN source_node_id < dest_node_id THEN dest_node_id ELSE source_node_id END
    ) WHERE status != 4 AND dest_node_id != '';

-- At most one unclaimed link per (source_node_id, dest_group) pair.
CREATE UNIQUE INDEX IF NOT EXISTS idx_links_unique_unclaimed
    ON links (source_node_id, dest_group)
    WHERE status != 4 AND dest_node_id = '';

-- ─── Topology tables (API-managed mode) ──────────────────────────────────────

CREATE TABLE IF NOT EXISTS topology_segments (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS topology_segment_links (
    segment_id TEXT NOT NULL REFERENCES topology_segments(id) ON DELETE CASCADE,
    source_group TEXT NOT NULL,
    dest_group TEXT NOT NULL,
    PRIMARY KEY (segment_id, source_group, dest_group)
);
