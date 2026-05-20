-- Add per-user i18n preferences. Both nullable; validation lives at the
-- HTTP edge (HTTP rejects values outside the supported set / unknown tz
-- names, see crates/i18n/src/{locale,tz}.rs).
ALTER TABLE users ADD COLUMN locale   TEXT NULL;
ALTER TABLE users ADD COLUMN timezone TEXT NULL;
