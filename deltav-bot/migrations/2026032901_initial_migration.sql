
CREATE TABLE direction_forums (
    channel_id INTEGER PRIMARY KEY,
    private INTEGER CHECK (private = 0 OR private = 1),
    tag_cr_approved INTEGER NOT NULL,
    tag_cr_denied INTEGER NOT NULL,
    tag_pr_closed INTEGER NOT NULL,
    tag_pr_merged INTEGER NOT NULL
);

CREATE TABLE direction_discussions (
    pr_id INTEGER PRIMARY KEY,
    discussion_forum_id INTEGER NOT NULL,
    discussion_channel_id INTEGER NOT NULL,

    FOREIGN KEY (discussion_forum_id) REFERENCES direction_forums(channel_id)
);

CREATE TABLE direction_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    primary_forum_channel INTEGER,

    FOREIGN KEY(primary_forum_channel) REFERENCES direction_forums(channel_id)
);
