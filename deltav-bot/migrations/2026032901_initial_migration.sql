
CREATE TABLE cr_forums (
    channel_id        INTEGER PRIMARY KEY,
    private           INTEGER NOT NULL CHECK (private = 0 OR private = 1),
    tag_cr_approved   INTEGER NOT NULL,
    tag_cr_test_merge INTEGER NOT NULL,
    tag_cr_denied     INTEGER NOT NULL,
    tag_pr_closed     INTEGER NOT NULL,
    tag_pr_merged     INTEGER NOT NULL
);

CREATE TABLE cr_discussions (
    pr_id                    INTEGER PRIMARY KEY,
    forum_id                 INTEGER NOT NULL,
    thread_id                INTEGER NOT NULL,

    review_days_total        INTEGER,
    review_days_passed       INTEGER,
    review_days_next_micros  INTEGER,

    pr_title                 TEXT NOT NULL,
    pr_author                TEXT NOT NULL,
    pr_body                  TEXT,

    FOREIGN KEY (forum_id) REFERENCES cr_forums(channel_id)
);

CREATE TABLE cr_config (
    id                    INTEGER PRIMARY KEY CHECK (id = 1),

    intake_cr_forum       INTEGER,
    public_cr_forum       INTEGER,
    private_cr_forum      INTEGER,

    gh_label_no_review    TEXT,
    gh_label_under_review TEXT,

    FOREIGN KEY(intake_cr_forum) REFERENCES cr_forums(channel_id),
    FOREIGN KEY(public_cr_forum) REFERENCES cr_forums(channel_id),
    FOREIGN KEY(private_cr_forum) REFERENCES cr_forums(channel_id)
);
