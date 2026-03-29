use std::sync::Arc;

use poise::serenity_prelude::{
    ChannelId, CreateActionRow, CreateButton, CreateEmbed, CreateEmbedAuthor, CreateForumPost,
    CreateMessage, ForumTagId,
};
use sqlx::{Pool, Sqlite};
use tokio::sync::{Mutex, mpsc::Receiver};
use tracing::{error, info, warn};

use crate::{
    discord::{
        Context, Error,
        storage::{delete_forum, get_main_forum, set_main_forum, upsert_forum},
    },
    github::{GitHub, GitHubMessage},
};

pub async fn direction_github_task(
    ctx: poise::serenity_prelude::Context,
    receiver: Arc<Mutex<Receiver<GitHubMessage>>>, // TODO: Shouldn't have to be like this, but it works for now.
    db: Pool<Sqlite>,
    gh: Arc<GitHub>,
) {
    while let Some(message) = receiver.lock().await.recv().await {
        match message {
            GitHubMessage::PrOpened {
                pr_id,
                pr_title,
                pr_body,
            } => {
                let Some(main_forum) = get_main_forum(&db).await else {
                    warn!("Received PrOpened but main forum is not set.");
                    return;
                };

                // TODO: If a review thread already exists, this has probably been reopened. Need to remove prior labels and notify CR.

                let mut embed_description = pr_body.unwrap_or("No description.".into());
                embed_description.truncate(4096); // max embed description length

                match main_forum
                    .create_forum_post(
                        &ctx,
                        CreateForumPost::new(
                            format!("{pr_title} #{pr_id}"),
                            CreateMessage::new()
                                .add_embeds(vec![
                                    CreateEmbed::new()
                                        .author(CreateEmbedAuthor::new(format!("PR #{pr_id}")).url(
                                            format!(
                                                "https://github.com/{}/{}/pull/{pr_id}",
                                                gh.repo_owner, gh.repo_name
                                            ),
                                        ))
                                        .title(pr_title)
                                        .description(embed_description),
                                ])
                                .components(vec![CreateActionRow::Buttons(vec![
                                    CreateButton::new("todo").label("Start review"), // Open dialog to select amount of days for timer
                                    CreateButton::new("todo").label("No review needed"), // No review needed should just mark as approved and send a message into the thread, then close it
                                ])]),
                        ),
                    )
                    .await
                {
                    Ok(post_channel) => {
                        info!("Created thread {} for PR #{pr_id}", post_channel.id.get());
                    }
                    Err(e) => {
                        error!("Failed to create forum post for opened PR {pr_id}: {e:#?}");
                    }
                }
            }
            _ => {}
        }
    }
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
