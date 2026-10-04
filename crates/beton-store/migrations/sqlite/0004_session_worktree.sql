-- 0004_session_worktree: Worktree einer Session (SES-015) als Projektion aus
-- `git.worktree_created` (DATA-005); per Rebuild neu aufbaubar. `NULL` = kein Worktree.
-- Gegenstück: migrations/postgres/0004_session_worktree.sql.

ALTER TABLE sessions ADD COLUMN worktree_path TEXT;
ALTER TABLE sessions ADD COLUMN worktree_branch TEXT;
ALTER TABLE sessions ADD COLUMN worktree_base TEXT;
ALTER TABLE sessions ADD COLUMN worktree_base_sha TEXT;
