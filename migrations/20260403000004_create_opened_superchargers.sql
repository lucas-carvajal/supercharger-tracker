CREATE TABLE opened_superchargers (
  id                TEXT PRIMARY KEY,
  title             TEXT NOT NULL,
  city              TEXT,
  region            TEXT,
  latitude          DOUBLE PRECISION NOT NULL,
  longitude         DOUBLE PRECISION NOT NULL,
  opening_date      DATE,
  num_stalls        INTEGER,
  open_to_non_tesla BOOLEAN,
  detected_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
