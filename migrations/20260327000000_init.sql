-- Supercharger tracker schema.
--
-- Each coming-soon location is identified by its Tesla location URL slug
-- (e.g. "11255" from https://www.tesla.com/findus?location=11255).
-- This slug is stable across scrapes and is used as the primary key (`id`)
-- throughout the system. Tesla's internal UUID field is intentionally not
-- stored — it changes arbitrarily for the same physical location and is
-- therefore unreliable as an identifier.

CREATE TYPE site_status AS ENUM (
    'IN_DEVELOPMENT',
    'UNDER_CONSTRUCTION',
    'UNKNOWN'
);

-- One row per scrape execution.
CREATE TABLE scrape_runs (
    id               BIGSERIAL PRIMARY KEY,
    country          TEXT NOT NULL,
    scraped_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    total_count      INT,
    details_failures INT NOT NULL DEFAULT 0,
    run_type         TEXT NOT NULL DEFAULT 'full'  -- 'full' | 'retry'
);

-- One row per coming-soon supercharger location.
-- `id` is the Tesla location URL slug and serves as the stable system identifier.
CREATE TABLE coming_soon_superchargers (
    id                   TEXT PRIMARY KEY,
    title                TEXT NOT NULL,
    city                 TEXT,
    region               TEXT,
    latitude             DOUBLE PRECISION NOT NULL,
    longitude            DOUBLE PRECISION NOT NULL,
    status               site_status NOT NULL DEFAULT 'UNKNOWN',
    raw_status_value     TEXT,                          -- raw string from Tesla details API
    first_seen_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_scraped_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    is_active            BOOLEAN NOT NULL DEFAULT TRUE, -- false when absent from latest scrape
    details_fetch_failed BOOLEAN NOT NULL DEFAULT FALSE -- true when details endpoint failed
);

-- Audit log of every status transition, including first appearance (old_status = NULL).
CREATE TABLE status_changes (
    id              BIGSERIAL PRIMARY KEY,
    supercharger_id TEXT NOT NULL REFERENCES coming_soon_superchargers(id),
    scrape_run_id   BIGINT NOT NULL REFERENCES scrape_runs(id),
    old_status      site_status,          -- NULL means first time this charger was seen
    new_status      site_status NOT NULL,
    changed_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX ON status_changes (supercharger_id);
CREATE INDEX ON coming_soon_superchargers (status);
CREATE INDEX ON coming_soon_superchargers (is_active);
CREATE INDEX ON coming_soon_superchargers (details_fetch_failed) WHERE details_fetch_failed = TRUE;
CREATE INDEX ON status_changes (changed_at DESC);
CREATE INDEX ON coming_soon_superchargers (first_seen_at DESC);
CREATE INDEX ON coming_soon_superchargers (region);
