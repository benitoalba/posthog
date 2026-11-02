-- no-transaction
--
-- Fair-dequeue index for the transactional email lane, mirroring
-- idx_cyclotron_jobs_email_priority_fair_dequeue on the marketing lane.
--
-- The lane now carries the class that `priority` used to, so every row here
-- shares one priority value. `priority` stays the leading key anyway: the
-- worker's ORDER BY is shared with the marketing lane, and a constant leading
-- key still lets index order satisfy it. `scheduled` trails so
-- `scheduled <= NOW()` stays an Index Cond rather than a heap-level filter.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_cyclotron_jobs_email_transactional_fair_dequeue
    ON cyclotron_jobs (priority, dequeue_seq NULLS FIRST, scheduled)
    WHERE status = 'available' AND queue_name = 'emailtransactional';
