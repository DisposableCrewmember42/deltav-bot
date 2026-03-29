use poise::serenity_prelude::{ChannelId, ForumTag, ForumTagId};
use sqlx::{Pool, Sqlite};
use tracing::{error, info, warn};

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
    let row = match sqlx::query!("SELECT primary_forum_channel FROM direction_config WHERE id = 1")
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

    row.primary_forum_channel
        .and_then(|x| Some(ChannelId::new(x.cast_unsigned())))
}

pub async fn set_main_forum(
    db: &Pool<Sqlite>,
    channel_id: Option<ChannelId>,
) -> Result<(), String> {
    let new_id = channel_id.and_then(|x| Some(x.get().cast_signed()));
    match sqlx::query!(
        r#"
        INSERT INTO direction_config (id, primary_forum_channel)
        VALUES(1, ?1)
        ON CONFLICT(id)
        DO UPDATE SET primary_forum_channel=excluded.primary_forum_channel;
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
        INSERT INTO direction_forums (channel_id, private, tag_cr_approved, tag_cr_denied, tag_pr_closed, tag_pr_merged)
        VALUES(?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(channel_id)
        DO UPDATE SET private=excluded.private, tag_cr_approved=excluded.tag_cr_approved, tag_cr_denied=excluded.tag_cr_denied, tag_pr_closed=excluded.tag_pr_closed, tag_pr_merged=excluded.tag_pr_merged;
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

    match sqlx::query!(
        "DELETE FROM direction_forums WHERE channel_id = ?1",
        channel_id
    )
    .execute(db)
    .await
    {
        Ok(_) => {
            info!("Deleted direction forum {}.", channel_id);
            return Ok(());
        }
        Err(e) => {
            error!("Failed to delete direction forum {channel_id}: {e:#?}");
            return Err(());
        }
    };
}
