use poise::serenity_prelude::ChannelId;
use sqlx::{Pool, Sqlite};
use tracing::{error, warn};

#[derive(Debug, Clone)]
pub struct DiscussionRecord {
    pub pr_id: u64,
    pub forum_id: ChannelId,
    pub thread_id: ChannelId,
    pub timer_end: Option<u64>,

    pub pr_title: String,
    pub pr_author: String,
    pub pr_body: Option<String>,
}

impl DiscussionRecord {
    pub async fn set_thread_id(
        &mut self,
        db: &Pool<Sqlite>,
        new_thread: ChannelId,
    ) -> Result<(), ()> {
        let new_thread_s = new_thread.get().cast_signed();
        let pr_id_s = self.pr_id.cast_signed();

        if let Err(e) = sqlx::query!(
            "UPDATE cr_discussions SET thread_id=?1 WHERE pr_id = ?2",
            new_thread_s,
            pr_id_s
        )
        .execute(db)
        .await
        {
            error!(
                "Failed to set new thread id {new_thread} for discussion of PR #{}: {e:#?}",
                self.pr_id
            );

            return Err(());
        }

        self.thread_id = new_thread;

        Ok(())
    }

    pub async fn delete(&self, db: &Pool<Sqlite>) -> Result<(), ()> {
        let pr_id_s = self.pr_id.cast_signed();
        if let Err(e) = sqlx::query!("DELETE FROM cr_discussions WHERE pr_id = ?1", pr_id_s)
            .execute(db)
            .await
        {
            error!(
                "Failed to delete discussion record for pr #{}: {e}",
                self.pr_id
            );
            return Err(());
        }

        Ok(())
    }

    pub async fn get_by_pr(db: &Pool<Sqlite>, pr_id: u64) -> Option<DiscussionRecord> {
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
                pr_title: r.pr_title,
                pr_author: r.pr_author,
                pr_body: r.pr_body,
            }),
            Err(e) => {
                warn!("Failed to get discussion by PR#{pr_id}: {e:#?}");
                None
            }
        }
    }

    pub async fn get_by_thread(
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
                pr_title: r.pr_title,
                pr_author: r.pr_author,
                pr_body: r.pr_body,
            }),
            Err(e) => {
                warn!("Failed to get discussion by thread {thread_id}: {e:#?}");
                None
            }
        }
    }

    pub async fn insert(&self, db: &Pool<Sqlite>) -> Result<(), ()> {
        let pr_id = self.pr_id.cast_signed();
        let forum_id = self.forum_id.get().cast_signed();
        let thread_id = self.thread_id.get().cast_signed();
        let timer_end = self.timer_end.and_then(|x| Some(x.cast_signed()));

        match sqlx::query!(
            "INSERT INTO cr_discussions(pr_id, forum_id, thread_id, timer_end, pr_title, pr_author, pr_body) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            pr_id,
            forum_id,
            thread_id,
            timer_end,
            self.pr_title,
            self.pr_author,
            self.pr_body
        )
        .execute(db)
        .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("Failed to insert CR discussion {self:?}: {e:#?}");
                Err(())
            }
        }
    }
}
