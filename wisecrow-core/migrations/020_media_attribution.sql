-- Stock photo licences require the source to be shown wherever results are
-- displayed. The providers already return a credit string; it was discarded
-- after the download, so a cached image had no way to name its photographer.
ALTER TABLE media_cache ADD COLUMN IF NOT EXISTS attribution TEXT;
