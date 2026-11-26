-- Convert the bot's quote tables from latin1 to utf8mb4.
--
-- Run against the quote database (dap2) as a user with ALTER privileges. The
-- client must send this file as UTF-8, because step 3 contains "Ã" and "Â":
--   mysql --default-character-set=utf8mb4 dap2 < migrations/2026-11-utf8mb4.sql
-- Undo with migrations/2026-11-utf8mb4-rollback.sql while the backup tables exist.
--
-- Background (checked against production 2026-11-26):
--   * All four tables are latin1 / latin1_swedish_ci, MyISAM.
--   * Every stored value containing a non-ASCII byte is valid UTF-8 that was
--     written through a latin1 connection, i.e. double-encoded. "toupée" is
--     stored as the latin1 characters "toupÃ©e". There are exactly 7 such
--     values: masterlist_quotes 655, 786, 916, 1900, 3359, 3540 and
--     nuke_quotes 43. No value holds genuine latin1 accented text.
--   * So the tables are converted with CONVERT TO (lossless for latin1), and
--     then only those 7 values have their bytes reinterpreted as UTF-8.
--
-- MyISAM has no transactions. Each ALTER is atomic on its own; if a step
-- fails, stop and run the rollback script.

-- 1. Byte-exact backups. LIKE keeps the latin1 definition, and INSERT ... SELECT
--    copies explicit values, so masterlist_quotes.time is preserved.
CREATE TABLE masterlist_quotes_pre_utf8mb4 LIKE masterlist_quotes;
INSERT INTO masterlist_quotes_pre_utf8mb4 SELECT * FROM masterlist_quotes;
CREATE TABLE masterlist_episodes_pre_utf8mb4 LIKE masterlist_episodes;
INSERT INTO masterlist_episodes_pre_utf8mb4 SELECT * FROM masterlist_episodes;
CREATE TABLE masterlist_shows_pre_utf8mb4 LIKE masterlist_shows;
INSERT INTO masterlist_shows_pre_utf8mb4 SELECT * FROM masterlist_shows;
CREATE TABLE nuke_quotes_pre_utf8mb4 LIKE nuke_quotes;
INSERT INTO nuke_quotes_pre_utf8mb4 SELECT * FROM nuke_quotes;

-- 2. Convert. ALTER TABLE copies rows without firing ON UPDATE, so
--    masterlist_quotes.time keeps its values.
ALTER TABLE masterlist_quotes CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
ALTER TABLE masterlist_episodes CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
ALTER TABLE masterlist_shows CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
ALTER TABLE nuke_quotes CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- 3. Repair the double-encoded values: turn the text back into its latin1
--    bytes, then read those bytes as UTF-8. The REGEXP guard skips a row that
--    no longer contains the mojibake (for example, edited in the meantime).
--    `time = time` stops ON UPDATE from re-stamping the repaired quotes.
UPDATE masterlist_quotes
   SET quote = CONVERT(CAST(CONVERT(quote USING latin1) AS BINARY) USING utf8mb4),
       time = time
 WHERE quote_id IN (655, 786, 916, 1900, 3359, 3540)
   AND quote REGEXP '[ÃÂ]';
UPDATE nuke_quotes
   SET pn_quote = CONVERT(CAST(CONVERT(pn_quote USING latin1) AS BINARY) USING utf8mb4)
 WHERE pn_qid = 43
   AND pn_quote REGEXP '[ÃÂ]';

-- 4. Check: every text column of the four tables should now be utf8mb4, and
--    no row should still contain the mojibake marker.
SELECT TABLE_NAME, COLUMN_NAME, CHARACTER_SET_NAME, COLLATION_NAME
  FROM information_schema.COLUMNS
 WHERE TABLE_SCHEMA = DATABASE()
   AND TABLE_NAME IN ('masterlist_quotes', 'masterlist_episodes', 'masterlist_shows', 'nuke_quotes')
   AND CHARACTER_SET_NAME IS NOT NULL;
SELECT 'quotes still double-encoded' AS check_name, COUNT(*) FROM masterlist_quotes WHERE quote REGEXP '[ÃÂ]'
UNION ALL
SELECT 'slogans still double-encoded', COUNT(*) FROM nuke_quotes WHERE pn_quote REGEXP '[ÃÂ]';

-- After the bot has run correctly for a while, drop the backups:
--   DROP TABLE masterlist_quotes_pre_utf8mb4, masterlist_episodes_pre_utf8mb4,
--              masterlist_shows_pre_utf8mb4, nuke_quotes_pre_utf8mb4;
