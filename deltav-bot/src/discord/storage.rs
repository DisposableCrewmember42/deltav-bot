use poise::serenity_prelude::{ChannelId, ForumTagId};
use sqlx::{Pool, Sqlite};
use tracing::{error, info, warn};

#[derive(Debug, Copy, Clone)]
pub struct ForumRecord {
    pub channel_id: ChannelId,
    pub private: bool,
    pub tag_cr_approved: ForumTagId,
    pub tag_cr_denied: ForumTagId,
    pub tag_pr_merged: ForumTagId,
    pub tag_pr_closed: ForumTagId,
}

#[derive(Debug, Copy, Clone)]
pub struct DiscussionRecord {
    pub pr_id: u64,
    pub forum_id: ChannelId,
    pub thread_id: ChannelId,
    pub timer_end: Option<u64>,
}

macro_rules! id_to_int {
    ($e: ident) => {
        let $e = $e.get().cast_signed();
    };
    ($e: ident, $($es:ident),+) => {
        id_to_int!($e);
        id_to_int!($($es),+);
    };
}

pub async fn get_main_forum(db: &Pool<Sqlite>) -> Option<ChannelId> {
    let row = match sqlx::query!("SELECT primary_cr_forum FROM direction_config WHERE id = 1")
        .fetch_optional(db)
        .await
    {
        Ok(Some(x)) => x,
        Ok(None) => {
            warn!("Main direction forum is unset.");
            return None;
        }
        Err(e) => {
            error!("Failed to fetch direction main forum: {e:#?}");
            return None;
        }
    };

    row.primary_cr_forum
        .and_then(|x| Some(ChannelId::new(x.cast_unsigned())))
}

pub async fn get_discussion_by_pr(db: &Pool<Sqlite>, pr_id: u64) -> Option<DiscussionRecord> {
    let pr_id_s = pr_id.cast_signed();
    match sqlx::query!("SELECT * FROM cr_discussions WHERE pr_id = ?1", pr_id_s)
        .fetch_one(db)
        .await
    {
        Ok(r) => Some(DiscussionRecord {
            forum_id: ChannelId::new(r.forum_id.cast_unsigned()),
            pr_id: r.pr_id.cast_unsigned(),
            thread_id: ChannelId::new(r.thread_id.cast_unsigned()),
            timer_end: r.timer_end.and_then(|x| Some(x.cast_unsigned())),
        }),
        Err(e) => {
            warn!("Failed to get discussion by PR#{pr_id}: {e:#?}");
            None
        }
    }
}

pub async fn get_discussion_by_thread(
    db: &Pool<Sqlite>,
    thread_id: ChannelId,
) -> Option<DiscussionRecord> {
    let thread_id_s = thread_id.get().cast_signed();
    match sqlx::query!(
        "SELECT * FROM cr_discussions WHERE thread_id = ?1",
        thread_id_s
    )
    .fetch_one(db)
    .await
    {
        Ok(r) => Some(DiscussionRecord {
            forum_id: ChannelId::new(r.forum_id.cast_unsigned()),
            pr_id: r.pr_id.cast_unsigned(),
            thread_id: ChannelId::new(r.thread_id.cast_unsigned()),
            timer_end: r.timer_end.and_then(|x| Some(x.cast_unsigned())),
        }),
        Err(e) => {
            warn!("Failed to get discussion by thread {thread_id}: {e:#?}");
            None
        }
    }
}

pub async fn add_discussion(db: &Pool<Sqlite>, discussion: DiscussionRecord) -> Result<(), ()> {
    let pr_id = discussion.pr_id.cast_signed();
    let forum_id = discussion.forum_id.get().cast_signed();
    let thread_id = discussion.thread_id.get().cast_signed();
    let timer_end = discussion.timer_end.and_then(|x| Some(x.cast_signed()));

    match sqlx::query!(
        "INSERT INTO cr_discussions(pr_id, forum_id, thread_id, timer_end) VALUES(?1, ?2, ?3, ?4)",
        pr_id,
        forum_id,
        thread_id,
        timer_end
    )
    .execute(db)
    .await
    {
        Ok(_) => Ok(()),
        Err(e) => {
            error!("Failed to insert CR discussion {discussion:?}: {e:#?}");
            Err(())
        }
    }
}

pub async fn set_discussion_timer(
    db: &Pool<Sqlite>,
    pr_id: u64,
    timer_end: Option<u64>,
) -> Result<(), ()> {
    let timer_end_s = timer_end.and_then(|x| Some(x.cast_signed()));
    let pr_id_s = pr_id.cast_signed();

    match sqlx::query!(
        "UPDATE cr_discussions SET timer_end=?1 WHERE pr_id=?2",
        timer_end_s,
        pr_id_s
    )
    .execute(db)
    .await
    {
        Ok(r) => {
            if r.rows_affected() == 0 {
                error!("Unable to set discussion timer for {pr_id}: no rows affected.");
                Err(())
            } else {
                Ok(())
            }
        }
        Err(e) => {
            error!("Failed to set discussion time for {pr_id} to {timer_end:?} due to error: {e}");
            Err(())
        }
    }
}

pub async fn set_main_forum(
    db: &Pool<Sqlite>,
    channel_id: Option<ChannelId>,
) -> Result<(), String> {
    let new_id = channel_id.and_then(|x| Some(x.get().cast_signed()));
    match sqlx::query!(
        r#"
        INSERT INTO direction_config (id, primary_cr_forum)
        VALUES(1, ?1)
        ON CONFLICT(id)
        DO UPDATE SET primary_cr_forum=excluded.primary_cr_forum;
        "#,
        new_id
    )
    .execute(db)
    .await
    {
        Ok(_) => {
            info!("Main direction forum set to {channel_id:?}.");
            return Ok(());
        }
        Err(e) => {
            error!("Failed to set direction main forum: {e:#?}");
            return Err("Did you register it as a forum first?".into());
        }
    };
}

pub async fn get_forum(db: &Pool<Sqlite>, forum: ChannelId) -> Option<ForumRecord> {
    id_to_int!(forum);
    match sqlx::query!("SELECT * FROM cr_forums WHERE channel_id = ?1", forum)
        .fetch_one(db)
        .await
    {
        Ok(r) => Some(ForumRecord {
            channel_id: ChannelId::new(r.channel_id.cast_unsigned()),
            private: r.private == 1,
            tag_cr_approved: ForumTagId::new(r.tag_cr_approved.cast_unsigned()),
            tag_cr_denied: ForumTagId::new(r.tag_cr_denied.cast_unsigned()),
            tag_pr_closed: ForumTagId::new(r.tag_pr_closed.cast_unsigned()),
            tag_pr_merged: ForumTagId::new(r.tag_pr_merged.cast_unsigned()),
        }),
        Err(e) => {
            error!("Failed to get_forum {forum}: {e:#?}");
            None
        }
    }
}

pub async fn get_forums(db: &Pool<Sqlite>) -> Result<Vec<ChannelId>, ()> {
    match sqlx::query!("SELECT channel_id FROM cr_forums")
        .fetch_all(db)
        .await
    {
        Ok(r) => Ok(r
            .iter()
            .map(|r| ChannelId::new(r.channel_id.cast_unsigned()))
            .collect()),
        Err(e) => {
            error!("Failed to get_forums: {e:#?}");
            Err(())
        }
    }
}

pub async fn upsert_forum(
    db: &Pool<Sqlite>,
    channel_id: ChannelId,
    private: bool,
    tag_cr_approved: ForumTagId,
    tag_cr_denied: ForumTagId,
    tag_pr_closed: ForumTagId,
    tag_pr_merged: ForumTagId,
) -> Result<(), ()> {
    id_to_int!(
        channel_id,
        tag_cr_approved,
        tag_cr_denied,
        tag_pr_closed,
        tag_pr_merged
    );
    let private: i64 = if private { 1 } else { 0 };

    info!(
        "Trying to upsert direction forum: {channel_id}, Private {private}, Approve {tag_cr_approved}, Deny {tag_cr_denied}, Close {tag_pr_closed}, Merge {tag_pr_merged}"
    );
    match sqlx::query!(
        r#"
        INSERT INTO cr_forums (channel_id, private, tag_cr_approved, tag_cr_denied, tag_pr_closed, tag_pr_merged)
        VALUES(?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(channel_id)
        DO UPDATE SET private=excluded.private, tag_cr_approved=excluded.tag_cr_approved, tag_cr_denied=excluded.tag_cr_denied,
        tag_pr_closed=excluded.tag_pr_closed, tag_pr_merged=excluded.tag_pr_merged;
        "#,
        channel_id, private, tag_cr_approved, tag_cr_denied, tag_pr_closed, tag_pr_merged
    )
    .execute(db)
    .await
    {
        Ok(_) => {
            info!("Successfully upserted direction forum {channel_id}.");
            return Ok(());
        }
        Err(e) => {
            error!("Failed to upsert direction forum: {e:#?}");
            return Err(());
        }
    };
}

pub async fn delete_forum(db: &Pool<Sqlite>, channel_id: ChannelId) -> Result<(), ()> {
    if get_main_forum(&db).await == Some(channel_id) {
        warn!("Main direction forum is being deleted.");
        set_main_forum(&db, None).await.map_err(|_| ())?;
    }

    id_to_int!(channel_id);

    match sqlx::query!("DELETE FROM cr_forums WHERE channel_id = ?1", channel_id)
        .execute(db)
        .await
    {
        Ok(r) => {
            if r.rows_affected() != 0 {
                info!("Deleted direction forum {}.", channel_id);
            }
            return Ok(());
        }
        Err(e) => {
            error!("Failed to delete direction forum {channel_id}: {e:#?}");
            return Err(());
        }
    };
}
