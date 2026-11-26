-- Undo migrations/2026-11-utf8mb4.sql by swapping the byte-exact backups back in.
--   mysql dap2 < migrations/2026-11-utf8mb4-rollback.sql
--
-- Any quote, show, episode or slogan added or edited after the migration is
-- lost, because the backups predate it. Check before running:
--   SELECT MAX(quote_id) FROM masterlist_quotes;              -- vs. ..._pre_utf8mb4
--   SELECT MAX(episode_id) FROM masterlist_episodes;
--   SELECT MAX(show_id) FROM masterlist_shows;
--   SELECT MAX(pn_qid) FROM nuke_quotes;
--
-- RENAME TABLE swaps all eight tables in one atomic step. The converted
-- tables are kept as *_utf8mb4_failed for inspection.

RENAME TABLE
    masterlist_quotes TO masterlist_quotes_utf8mb4_failed,
    masterlist_quotes_pre_utf8mb4 TO masterlist_quotes,
    masterlist_episodes TO masterlist_episodes_utf8mb4_failed,
    masterlist_episodes_pre_utf8mb4 TO masterlist_episodes,
    masterlist_shows TO masterlist_shows_utf8mb4_failed,
    masterlist_shows_pre_utf8mb4 TO masterlist_shows,
    nuke_quotes TO nuke_quotes_utf8mb4_failed,
    nuke_quotes_pre_utf8mb4 TO nuke_quotes;
