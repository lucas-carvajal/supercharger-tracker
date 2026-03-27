CREATE TYPE site_status AS ENUM (
    'IN_DEVELOPMENT',
    'UNDER_CONSTRUCTION',
    'UNKNOWN'
);

CREATE TABLE scrape_runs (
    id          BIGSERIAL PRIMARY KEY,
    country     TEXT NOT NULL,
    scraped_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    total_count INT,
    error       TEXT
);

CREATE TABLE coming_soon_superchargers (
    uuid              TEXT PRIMARY KEY,
    title             TEXT NOT NULL,
    latitude          DOUBLE PRECISION NOT NULL,
    longitude         DOUBLE PRECISION NOT NULL,
    status            site_status NOT NULL DEFAULT 'UNKNOWN',
    location_url_slug TEXT,
    raw_status_value  TEXT,
    first_seen_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_scraped_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    is_active         BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE status_changes (
    id                BIGSERIAL PRIMARY KEY,
    supercharger_uuid TEXT NOT NULL REFERENCES coming_soon_superchargers(uuid),
    scrape_run_id     BIGINT NOT NULL REFERENCES scrape_runs(id),
    old_status        site_status,         -- NULL = first time we see this charger
    new_status        site_status NOT NULL,
    changed_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX ON status_changes (supercharger_uuid);
CREATE INDEX ON coming_soon_superchargers (status);
CREATE INDEX ON coming_soon_superchargers (is_active);
