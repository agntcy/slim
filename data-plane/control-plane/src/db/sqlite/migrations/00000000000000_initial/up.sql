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
