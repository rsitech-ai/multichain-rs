CREATE SCHEMA IF NOT EXISTS control;

CREATE TABLE IF NOT EXISTS control.schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO control.schema_version (version)
VALUES (1)
ON CONFLICT (version) DO NOTHING;
