use std::sync::Arc;

use poise::serenity_prelude::{
    ChannelId, CreateActionRow, CreateButton, CreateEmbed, CreateEmbedAuthor, CreateForumPost,
    CreateMessage, EditThread, ForumTagId, GuildChannel, futures::TryFutureExt,
};
use sqlx::{Pool, Sqlite};
use tokio::sync::{Mutex, mpsc::Receiver};
use tracing::{error, info, warn};

use crate::{
    discord::{
        Context, Error,
        storage::{
            DiscussionRecord, add_discussion, delete_forum, get_discussion_by_pr, get_forum,
            get_main_forum, set_main_forum, upsert_forum,
        },
    },
    github::{GitHub, GitHubMessage},
};

const GH_COMMENT_PREFIX: &'static str = "!discord";
const EMBED_DESC_MAX_LEN: usize = 4096;

pub async fn direction_github_task(
    ctx: poise::serenity_prelude::Context,
    receiver: Arc<Mutex<Receiver<GitHubMessage>>>,
    db: Pool<Sqlite>,
    gh: Arc<GitHub>,
) {
    while let Some(message) = receiver.lock().await.recv().await {
        match message {
            GitHubMessage::PrOpened {
                pr_id,
                pr_title,
                pr_body,
                opened_by,
            } => {
                let Some(main_forum) = get_main_forum(&db).await else {
                    warn!("Received PrOpened but main forum is not set.");
                    continue;
                };

                if let Some(discussion) = get_discussion_by_pr(&db, pr_id).await {
                    let Some(forum) = get_forum(&db, discussion.forum_id).await else {
                        error!(
                            "Discussion for {pr_id} exists, but the forum does not have a record!"
                        );
                        continue;
                    };

                    if let Err(e) = discussion
                        .thread_id
                        .send_message(
                            &ctx,
                            CreateMessage::new()
                                .content(format!("This PR has been opened by `{opened_by}`.")),
                        )
                        .await
                    {
                        error!("Failed to send message about PR {pr_id} opening: {e}");
                    }

                    let Some(guild_channel) =
                        discussion_channel_to_guild(pr_id, discussion.thread_id, &ctx).await
                    else {
                        continue;
                    };

                    if let Err(e) = discussion
                        .thread_id
                        .edit_thread(
                            &ctx,
                            EditThread::new().applied_tags(
                                guild_channel
                                    .applied_tags
                                    .iter()
                                    .filter(|x| **x != forum.tag_pr_closed)
                                    .cloned(),
                            ),
                        )
                        .await
                    {
                        error!(
                            "Failed to remove closed tag from {:?}: {e:#?}",
                            discussion.thread_id
                        );
                    }

                    continue;
                }

                // String::truncate might panic, so doing it like this.
                let embed_description: String = pr_body
                    .unwrap_or("No description.".into())
                    .chars()
                    .take(EMBED_DESC_MAX_LEN)
                    .collect();

                match main_forum
                    .create_forum_post(
                        &ctx,
                        CreateForumPost::new(
                            format!("{pr_title} #{pr_id}"),
                            CreateMessage::new().add_embeds(vec![
                                CreateEmbed::new()
                                    .author(
                                        CreateEmbedAuthor::new(format!(
                                            "PR #{pr_id}, submitted by {opened_by}"
                                        ))
                                        .url(format!(
                                            "https://github.com/{}/{}/pull/{pr_id}",
                                            gh.repo_owner, gh.repo_name
                                        )),
                                    )
                                    .title(pr_title)
                                    .description(embed_description),
                            ]), // .components(vec![CreateActionRow::Buttons(vec![
                                //     CreateButton::new("todo-0").label("Start review"), // Open dialog to select amount of days for timer
                                //     CreateButton::new("todo-1").label("No review needed"), // No review needed should just mark as approved and send a message into the thread, then close it
                                // ])]),
                        ),
                    )
                    .await
                {
                    Ok(post_channel) => {
                        info!("Created thread {} for PR #{pr_id}", post_channel.id.get());
                        if let Err(()) = add_discussion(
                            &db,
                            DiscussionRecord {
                                forum_id: main_forum,
                                pr_id,
                                thread_id: post_channel.id,
                                timer_end: None,
                            },
                        )
                        .await
                        {
                            error!("Could not record thread creation for PR #{pr_id} in database.");
                        }
                    }
                    Err(e) => {
                        error!("Failed to create forum post for opened PR {pr_id}: {e:#?}");
                    }
                }
            }

            GitHubMessage::AuthorCommented {
                issue_id,
                username,
                comment,
            } => {
                let Some(discussion) = get_discussion_by_pr(&db, issue_id).await else {
                    continue;
                };

                if !comment.to_ascii_lowercase().starts_with(GH_COMMENT_PREFIX) {
                    continue;
                }

                info!(
                    "Author {username} of PR {issue_id}, associated with thread {}, wrote a comment.",
                    discussion.thread_id.get()
                );

                let comment: String = comment[GH_COMMENT_PREFIX.len()..]
                    .chars()
                    .take(EMBED_DESC_MAX_LEN)
                    .collect();

                if let Err(e) = discussion
                    .thread_id
                    .send_message(
                        &ctx,
                        CreateMessage::new().add_embed(
                            CreateEmbed::new()
                                .author(CreateEmbedAuthor::new(format!("{username} via GitHub")))
                                .description(comment),
                        ),
                    )
                    .await
                {
                    error!(
                        "Failed to send author comment for PR #{issue_id} to {}: {e:#?}",
                        discussion.thread_id
                    );
                }
            }

            GitHubMessage::PrClosed { pr_id, closed_by } => {
                let Some(discussion) = get_discussion_by_pr(&db, pr_id).await else {
                    continue;
                };
                info!(
                    "PR {pr_id}, associated with thread {}, has been closed.",
                    discussion.thread_id.get()
                );

                let Some(forum) = get_forum(&db, discussion.forum_id).await else {
                    continue;
                };

                let Some(guild_channel) =
                    discussion_channel_to_guild(pr_id, discussion.thread_id, &ctx).await
                else {
                    continue;
                };

                if let Err(e) = guild_channel
                    .send_message(
                        &ctx,
                        CreateMessage::new()
                            .content(format!("This PR has been closed by `{closed_by}`.")),
                    )
                    .await
                {
                    error!("Failed to send message about PR {pr_id} closing: {e:#?}");
                }

                if let Err(e) = discussion
                    .thread_id
                    .edit_thread(
                        &ctx,
                        EditThread::new().applied_tags(
                            guild_channel
                                .applied_tags
                                .iter()
                                .chain(vec![forum.tag_pr_closed].iter())
                                .cloned(),
                        ),
                    )
                    .await
                {
                    error!(
                        "Failed to add closed tag to {:?}: {e:#?}",
                        discussion.thread_id
                    );
                }
            }

            GitHubMessage::PrMerged { pr_id, merged_by } => {
                let Some(discussion) = get_discussion_by_pr(&db, pr_id).await else {
                    continue;
                };
                info!(
                    "PR {pr_id}, associated with thread {}, has been merged.",
                    discussion.thread_id.get()
                );

                let Some(forum) = get_forum(&db, discussion.forum_id).await else {
                    continue;
                };

                let Some(guild_channel) =
                    discussion_channel_to_guild(pr_id, discussion.thread_id, &ctx).await
                else {
                    continue;
                };

                if let Err(e) = guild_channel
                    .send_message(
                        &ctx,
                        CreateMessage::new()
                            .content(format!("This PR has been merged by `{merged_by}`.")),
                    )
                    .await
                {
                    error!("Failed to send message about PR {pr_id} being merged: {e:#?}");
                }

                if let Err(e) = discussion
                    .thread_id
                    .edit_thread(
                        &ctx,
                        EditThread::new().applied_tags(
                            guild_channel
                                .applied_tags
                                .iter()
                                .chain(vec![forum.tag_pr_merged].iter())
                                .cloned(),
                        ),
                    )
                    .await
                {
                    error!(
                        "Failed to add merged tag to {:?}: {e:#?}",
                        discussion.thread_id
                    );
                }
            }
        }
    }
}

async fn discussion_channel_to_guild(
    pr_id: u64,
    id: ChannelId,
    ctx: &poise::serenity_prelude::Context,
) -> Option<GuildChannel> {
    let guild_channel = match id.to_channel(ctx).await {
        Ok(x) => x,
        Err(e) => {
            error!("Failed to fetch channel to retrieve tags: {e:#?}");
            return None;
        }
    };

    let guild_channel = guild_channel.guild();
    if guild_channel.is_none() {
        error!("Discussion channel for PR {pr_id} was not a guild channel!");
    };

    guild_channel
}

/// Set config values for the Direction module
#[poise::command(slash_command)]
pub async fn direction_config(
    ctx: Context<'_>,
    public_review_forum: Option<ChannelId>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    if let Some(public_review_forum) = public_review_forum {
        if let Err(e) = set_main_forum(&ctx.data().db, Some(public_review_forum)).await {
            ctx.reply(format!("Failed to set main forum: {e}")).await?;
            return Ok(());
        }
    }

    ctx.reply("Processed without errors.").await?;
    Ok(())
}

/// Add or update a direction forum
#[poise::command(slash_command)]
pub async fn direction_forum_upsert(
    ctx: Context<'_>,
    forum: ChannelId,
    private: bool,
    tag_approved: ForumTagId,
    tag_denied: ForumTagId,
    tag_closed: ForumTagId,
    tag_merged: ForumTagId,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    match upsert_forum(
        &ctx.data().db,
        forum,
        private,
        tag_approved,
        tag_denied,
        tag_closed,
        tag_merged,
    )
    .await
    {
        Ok(_) => {
            ctx.reply("Processed without errors.").await?;
        }
        Err(_) => {
            ctx.reply("Internal error occurred while upserting forum.")
                .await?;
        }
    }

    Ok(())
}

// Remove a direction forum (this does not delete the actual channel)
#[poise::command(slash_command)]
pub async fn direction_forum_delete(ctx: Context<'_>, forum: ChannelId) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    match delete_forum(&ctx.data().db, forum).await {
        Ok(_) => {
            ctx.reply("Processed without errors.").await?;
        }
        Err(_) => {
            ctx.reply("Internal error occurred while upserting forum.")
                .await?;
        }
    }

    Ok(())
}
