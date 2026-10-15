CREATE TABLE thread_presentations (
    thread_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    rollout_ordinal INTEGER NOT NULL,
    item_json TEXT NOT NULL,
    PRIMARY KEY (thread_id, item_id)
);
CREATE INDEX thread_presentations_order ON thread_presentations(thread_id, rollout_ordinal, item_id);
