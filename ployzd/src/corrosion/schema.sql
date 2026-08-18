CREATE TABLE cluster
(
    key        TEXT      NOT NULL PRIMARY KEY,
    value      ANY,
    updated_at TIMESTAMP NOT NULL DEFAULT '1970-01-01 00:00:00'
);

CREATE TABLE machines
(
    id         TEXT      NOT NULL PRIMARY KEY,
    name       TEXT AS (json_extract(info, '$.name')),
    info       TEXT      NOT NULL DEFAULT '{}' CHECK (json_valid(info)),
    created_at TIMESTAMP NOT NULL DEFAULT '1970-01-01 00:00:00',
    updated_at TIMESTAMP NOT NULL DEFAULT '1970-01-01 00:00:00'
);

CREATE TABLE containers
(
    id                 TEXT      NOT NULL PRIMARY KEY,
    container          TEXT      NOT NULL DEFAULT '{}' CHECK (json_valid(container)),
    machine_id         TEXT      NOT NULL DEFAULT '',
    service_id         TEXT AS (json_extract(container, '$.service_id')),
    project_name       TEXT AS (json_extract(container, '$.project_name')),
    service_name       TEXT AS (json_extract(container, '$.service_name')),
    updated_at         TIMESTAMP NOT NULL DEFAULT '1970-01-01 00:00:00'
);

CREATE TABLE certificates
(
    hostname   TEXT      NOT NULL PRIMARY KEY,
    body       TEXT      NOT NULL DEFAULT '{}' CHECK (json_valid(body)),
    updated_at TIMESTAMP NOT NULL DEFAULT '1970-01-01 00:00:00'
);

CREATE TABLE volumes
(
    machine_id TEXT      NOT NULL,
    name       TEXT      NOT NULL CHECK (name != ''),
    volume     TEXT      NOT NULL DEFAULT '{}' CHECK (json_valid(volume)),
    updated_at TIMESTAMP NOT NULL DEFAULT '1970-01-01 00:00:00',
    PRIMARY KEY (machine_id, name)
);

CREATE INDEX idx_machines_name ON machines (name);
CREATE INDEX idx_containers_machine_id ON containers (machine_id);
CREATE INDEX idx_containers_service_id ON containers (service_id);
CREATE INDEX idx_containers_project_service ON containers (project_name, service_name);
