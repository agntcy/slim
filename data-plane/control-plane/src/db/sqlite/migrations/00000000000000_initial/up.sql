CREATE TABLE IF NOT EXISTS nodes (
    id TEXT NOT NULL PRIMARY KEY,
    group_name TEXT,
    conn_details TEXT NOT NULL,
    last_updated BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS routes (
    id BIGINT NOT NULL PRIMARY KEY,
    source_node_id TEXT NOT NULL,
    dest_node_id TEXT NOT NULL,
    link_id TEXT NOT NULL,
    component0 TEXT NOT NULL,
    component1 TEXT NOT NULL,
    component2 TEXT NOT NULL,
    component_id BIGINT,
    status INTEGER NOT NULL,
    status_msg TEXT NOT NULL,
    deleted BOOLEAN NOT NULL,
    last_updated BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS links (
    link_id TEXT NOT NULL,
    source_node_id TEXT NOT NULL,
    dest_node_id TEXT NOT NULL,
    dest_endpoint TEXT NOT NULL,
    conn_config_data TEXT NOT NULL,
    status INTEGER NOT NULL,
    status_msg TEXT NOT NULL,
    deleted BOOLEAN NOT NULL,
    last_updated BIGINT NOT NULL,
    PRIMARY KEY (link_id, source_node_id, dest_node_id, dest_endpoint)
);

CREATE TABLE IF NOT EXISTS channels (
    id TEXT NOT NULL PRIMARY KEY,
    moderators TEXT NOT NULL,
    participants TEXT NOT NULL
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
