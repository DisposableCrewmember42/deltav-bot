use std::sync::Arc;

use poise::serenity_prelude::{
    ChannelId, ComponentInteractionCollector, ComponentInteractionDataKind, CreateActionRow,
    CreateButton, CreateEmbed, CreateEmbedAuthor, CreateForumPost, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateMessage, EditInteractionResponse, EditThread,
    ForumTagId, GuildChannel,
};
use sqlx::{Pool, Sqlite};
use tokio::sync::{Mutex, mpsc::Receiver};
use tracing::{error, info, warn};

use crate::{
    discord::{
        Context, Error,
        storage::{
            DiscussionRecord, add_discussion, delete_forum, get_discussion_by_pr, get_forum,
            get_intake_forum, get_no_review_needed_label, set_intake_forum,
            set_no_review_needed_label, set_private_forum, set_public_forum, upsert_forum,
        },
    },
    github::{GitHub, GitHubMessage},
};

const GH_COMMENT_PREFIX: &'static str = "!discord";
const EMBED_DESC_MAX_LEN: usize = 4096;

const BUTTON_ID_PREFIX: &'static str = "cr";
const BUTTON_ID_ACTION_START_PUBLIC: &'static str = "reviewStartPublic";
const BUTTON_ID_ACTION_START_PRIVATE: &'static str = "reviewStartPrivate";
const BUTTON_ID_ACTION_NOT_NEEDED: &'static str = "reviewNotNeeded";

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
                let Some(main_forum) = get_intake_forum(&db).await else {
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
                            CreateMessage::new()
                                .add_embeds(vec![
                                    CreateEmbed::new()
                                        .author(
                                            CreateEmbedAuthor::new(format!(
                                                "PR #{pr_id}, submitted by {opened_by}"
                                            ))
                                            .url(
                                                format!(
                                                    "https://github.com/{}/{}/pull/{pr_id}",
                                                    gh.repo_owner, gh.repo_name
                                                ),
                                            ),
                                        )
                                        .title(&pr_title)
                                        .description(&embed_description),
                                ])
                                .components(vec![CreateActionRow::Buttons(vec![
                                    CreateButton::new(format!(
                                        "{BUTTON_ID_PREFIX}_{BUTTON_ID_ACTION_START_PUBLIC}_{pr_id}"
                                    ))
                                    .label("Public review"),
                                    CreateButton::new(format!(
                                        "{BUTTON_ID_PREFIX}_{BUTTON_ID_ACTION_START_PRIVATE}_{pr_id}"
                                    ))
                                    .label("Private review"),
                                    CreateButton::new(format!(
                                        "{BUTTON_ID_PREFIX}_{BUTTON_ID_ACTION_NOT_NEEDED}_{pr_id}"
                                    ))
                                    .label("No review needed"),
                                ])]),
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
                                pr_title,
                                pr_author: opened_by,
                                pr_body: Some(embed_description),
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

pub async fn direction_component_task(
    ctx: poise::serenity_prelude::Context,
    db: Pool<Sqlite>,
    gh: Arc<GitHub>,
) {
    while let Some(interaction) = ComponentInteractionCollector::new(&ctx)
        .filter(move |i| {
            i.data
                .custom_id
                .starts_with(&format!("{BUTTON_ID_PREFIX}_"))
        })
        .await
    {
        match interaction.data.kind {
            ComponentInteractionDataKind::Button => {
                if let Err(e) = interaction.defer_ephemeral(&ctx).await {
                    error!(
                        "Failed to defer ephemeral on button press with id '{}': {e:#?}",
                        interaction.data.custom_id
                    );
                }

                let _ = interaction
                    .create_response(&ctx, CreateInteractionResponse::Acknowledge)
                    .await;

                let error_response =
                    EditInteractionResponse::new().content("An internal error occurred.");

                // TODO: Check permissions

                let id_parts: Vec<&str> = interaction.data.custom_id.split("_").collect();
                if id_parts.len() != 3 {
                    error!("Received invalid button press with ID {id_parts:?}.");
                    let _ = interaction.edit_response(&ctx, error_response).await;

                    continue;
                }

                let Ok(pr_id) = id_parts[2].parse::<u64>() else {
                    error!(
                        "Received invalid button press with pr_id='{}' ({id_parts:?}).",
                        id_parts[2]
                    );
                    let _ = interaction.edit_response(&ctx, error_response).await;

                    continue;
                };

                let Some(mut discussion) = get_discussion_by_pr(&db, pr_id).await else {
                    error!("Received button press {id_parts:?}, but could not find discussion.");
                    let _ = interaction.edit_response(&ctx, error_response).await;

                    continue;
                };

                let Some(parent_forum) =
                    discussion_channel_to_guild(pr_id, discussion.thread_id, &ctx)
                        .await
                        .and_then(|x| x.parent_id)
                else {
                    error!(
                        "Failed to get parent forum for discussion thread {}",
                        discussion.thread_id
                    );
                    continue;
                };

                let Some(intake_forum) = get_intake_forum(&db).await else {
                    continue;
                };

                if parent_forum != intake_forum {
                    error!(
                        "Received button press {id_parts:?}, but parent forum was not intake forum."
                    );
                    let _ = interaction.edit_response(&ctx, error_response).await;

                    continue;
                }

                let intake_thread = discussion.thread_id;

                match id_parts[1] {
                    BUTTON_ID_ACTION_START_PUBLIC => {}
                    BUTTON_ID_ACTION_START_PRIVATE => {}
                    BUTTON_ID_ACTION_NOT_NEEDED => {
                        let Some(no_review_needed_label) = get_no_review_needed_label(&db).await
                        else {
                            error!("Can't process no review press without label.");
                            let _ = interaction
                                .edit_response(
                                    &ctx,
                                    EditInteractionResponse::new().content(
                                        "Can't process No Review Needed with GitHub label unset.",
                                    ),
                                )
                                .await;
                            continue;
                        };

                        if let Err(()) = discussion.delete(&db).await {
                            error!(
                                "Failed to delete discussion from DB. Can't process no review needed press further."
                            );
                            continue;
                        }

                        if let Err(e) = gh
                            .octo_install
                            .issues(&gh.repo_owner, &gh.repo_name)
                            .add_labels(discussion.pr_id, &vec![no_review_needed_label])
                            .await
                        {
                            error!(
                                "Failed to set no review needed label on PR #{}: {e:#?}",
                                discussion.pr_id
                            );
                        }

                        if let Err(e) = gh
                            .octo_install
                            .issues(&gh.repo_owner, &gh.repo_name)
                            .create_comment(discussion.pr_id, format!("**Triaged by {}:** This PR does not require a content review discussion.", interaction.user.name))
                            .await
                        {
                            error!(
                                "Failed to create no review needed comment on PR #{}: {e:#?}",
                                discussion.pr_id
                            );
                        }
                    }
                    action => {
                        error!("Received button press with invalid action {}", action);
                        let _ = interaction.edit_response(&ctx, error_response).await;
                        continue;
                    }
                }

                if let Err(e) = intake_thread.delete(&ctx).await {
                    error!("Failed to delete intake discussion for pr {pr_id}: {e:#?}");
                    let _ = interaction.edit_response(&ctx, error_response).await;
                    continue;
                }
            }
            _ => {}
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
            error!("Failed to fetch channel from id {id}: {e:#?}");
            return None;
        }
    };

    let guild_channel = guild_channel.guild();
    if guild_channel.is_none() {
        error!("Discussion channel for PR {pr_id} was not a guild channel!");
    };

    guild_channel
}

#[poise::command(slash_command, subcommands("cr_forum", "cr_config", "cr_review"))]
pub async fn cr(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(slash_command, rename = "review")]
pub async fn cr_review(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(
    slash_command,
    subcommands("cr_forum_upsert", "cr_forum_delete"),
    rename = "forum"
)]
pub async fn cr_forum(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Set config values for the Content Review module
#[poise::command(slash_command, rename = "config")]
pub async fn cr_config(
    ctx: Context<'_>,
    intake_cr_forum: Option<ChannelId>,
    public_cr_forum: Option<ChannelId>,
    private_cr_forum: Option<ChannelId>,
    gh_label_no_review: Option<String>,
    gh_label_under_review: Option<String>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    if let Some(intake_cr_forum) = intake_cr_forum {
        if let Err(e) = set_intake_forum(&ctx.data().db, Some(intake_cr_forum)).await {
            ctx.reply(format!("Failed to set intake forum: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(public_cr_forum) = public_cr_forum {
        if let Err(e) = set_public_forum(&ctx.data().db, Some(public_cr_forum)).await {
            ctx.reply(format!("Failed to set public forum: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(private_cr_forum) = private_cr_forum {
        if let Err(e) = set_private_forum(&ctx.data().db, Some(private_cr_forum)).await {
            ctx.reply(format!("Failed to set private forum: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(no_review_needed_label) = gh_label_no_review {
        if let Err(()) = set_no_review_needed_label(&ctx.data().db, no_review_needed_label).await {
            ctx.reply(format!("Failed to set no review needed label."))
                .await?;
            return Ok(());
        }
    }

    ctx.reply("Processed without errors.").await?;
    Ok(())
}

/// Add or update a direction forum
#[poise::command(slash_command, rename = "upsert")]
pub async fn cr_forum_upsert(
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
#[poise::command(slash_command, rename = "delete")]
pub async fn cr_forum_delete(ctx: Context<'_>, forum: ChannelId) -> Result<(), Error> {
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
