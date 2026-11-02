-- no-transaction
--
-- Superseded by cyclotron_jobs_transactional: the lane's rows no longer live in
-- cyclotron_jobs, so this partial index can never match again. Dropped rather
-- than left in place because it still costs a write-path check on every
-- available-status transition in the table.
DROP INDEX CONCURRENTLY IF EXISTS idx_cyclotron_jobs_email_transactional_fair_dequeue;
