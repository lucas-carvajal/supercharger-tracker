ALTER TABLE coming_soon_superchargers
    ADD COLUMN raw_project_status     TEXT,
    ADD COLUMN num_charger_stalls     INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN charging_accessibility TEXT,
    ADD COLUMN street_address         TEXT,
    ADD COLUMN county                 TEXT,
    ADD COLUMN postal_code            TEXT,
    ADD COLUMN country_code           TEXT;