-- Derived history only. Version zero is replayed transactionally before advancing it.
ALTER TABLE thread_history_projection_state ADD COLUMN external_input_version INTEGER NOT NULL DEFAULT 0;
