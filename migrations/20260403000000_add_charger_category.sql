CREATE TYPE charger_category AS ENUM ('COMING_SOON', 'WINNER', 'CURRENT_WINNER');

ALTER TABLE coming_soon_superchargers
    ADD COLUMN charger_category charger_category NOT NULL DEFAULT 'COMING_SOON';
