
CREATE TABLE cr_forums (
    channel_id INTEGER PRIMARY KEY,
    private INTEGER NOT NULL CHECK (private = 0 OR private = 1),
    tag_cr_approved INTEGER NOT NULL,
    tag_cr_denied INTEGER NOT NULL,
    tag_pr_closed INTEGER NOT NULL,
    tag_pr_merged INTEGER NOT NULL
);

CREATE TABLE cr_discussions (
    pr_id INTEGER PRIMARY KEY,
    forum_id INTEGER NOT NULL,
    thread_id INTEGER NOT NULL,
    timer_end INTEGER,

    FOREIGN KEY (forum_id) REFERENCES cr_forums(channel_id)
);

CREATE TABLE direction_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    primary_cr_forum INTEGER,

    FOREIGN KEY(primary_cr_forum) REFERENCES cr_forums(channel_id)
);
