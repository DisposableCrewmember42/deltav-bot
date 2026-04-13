use std::{sync::Arc, time::Duration};

use poise::{
    Modal, execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteractionCollector, ComponentInteractionDataKind, CreateForumPost,
        CreateInputText, CreateInteractionResponse, CreateInteractionResponseMessage,
        CreateMessage, CreateQuickModal, EditInteractionResponse, InputTextStyle,
    },
};
use sqlx::{Pool, Sqlite};
use tracing::error;

use crate::{
    discord::{
        content_review::{
            BUTTON_ID_ACTION_NOT_NEEDED, BUTTON_ID_ACTION_START_PRIVATE,
            BUTTON_ID_ACTION_START_PUBLIC, INTERACTION_ID_PREFIX, create_pr_embed,
            discussion_channel_to_guild,
        },
        data::{config::Config, discussions::DiscussionRecord},
    },
    github::GitHub,
};

#[derive(Debug, Modal)]
#[name = "Start a review"] // Struct name by default
struct StartReviewModal {
    #[name = "Review time (days)"]
    #[placeholder = "for example: 7"]
    #[min_length = 1]
    #[max_length = 2]
    review_time_days: String,
    #[name = "Reasoning"]
    #[placeholder = "Why does this require a public/private review? This can be left empty."] // No placeholder by default
    #[paragraph]
    reasoning: Option<String>,
}

// needed to call poise functions that expect to take a poise context from the task
struct CtxWrapper<'a> {
    context: &'a poise::serenity_prelude::Context,
}

impl<'a> CtxWrapper<'a> {
    pub fn new(context: &'a poise::serenity_prelude::Context) -> Self {
        Self { context }
    }
}

impl<'a> AsRef<poise::serenity_prelude::Context> for CtxWrapper<'a> {
    fn as_ref(&self) -> &poise::serenity_prelude::Context {
        self.context
    }
}

pub async fn cr_component_task(
    ctx: poise::serenity_prelude::Context,
    db: Pool<Sqlite>,
    gh: Arc<GitHub>,
) {
    while let Some(interaction) = ComponentInteractionCollector::new(&ctx)
        .filter(move |i| {
            i.data
                .custom_id
                .starts_with(&format!("{INTERACTION_ID_PREFIX}_"))
        })
        .await
    {
        match interaction.data.kind {
            ComponentInteractionDataKind::Button => {
                // if let Err(e) = interaction.defer_ephemeral(&ctx).await {
                //     error!(
                //         "Failed to defer ephemeral on button press with id '{}': {e:#?}",
                //         interaction.data.custom_id
                //     );
                // }

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

                let Some(mut discussion) = DiscussionRecord::get_by_pr(&db, pr_id).await else {
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

                let Some(intake_forum) = Config::get_intake_forum(&db).await else {
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
                    BUTTON_ID_ACTION_START_PUBLIC => {
                        let Some(public_forum) = Config::get_public_forum(&db).await else {
                            error!("Can't process public review press without public forum.");
                            let _ = interaction.create_response(
                                &ctx,
                                CreateInteractionResponse::Message(
                                    CreateInteractionResponseMessage::new().content(
                                        "Can't process public start with public forum unset.",
                                    ),
                                ),
                            );
                            continue;
                        };

                        let Some(under_review_label) = Config::get_under_review_label(&db).await
                        else {
                            error!(
                                "Can't process public review press without under review github label."
                            );
                            let _ = interaction.create_response(
                                &ctx,
                                CreateInteractionResponse::Message(
                                    CreateInteractionResponseMessage::new().content(
                                        "Can't process public start with under review label unset.",
                                    ),
                                ),
                            );
                            continue;
                        };

                        let interaction = interaction.clone();
                        let ctx = ctx.clone();
                        let db = db.clone();
                        let gh = gh.clone();
                        tokio::spawn(async move {
                            let Ok(Some(review_settings)) =
                                execute_modal_on_component_interaction::<StartReviewModal>(
                                    CtxWrapper::new(&ctx),
                                    interaction.clone(),
                                    None,
                                    Some(Duration::from_mins(60)),
                                )
                                .await
                            else {
                                let _ = interaction
                                    .edit_response(
                                        &ctx,
                                        EditInteractionResponse::new().content(
                                            "Modal timed out or an internal error occurred.",
                                        ),
                                    )
                                    .await;
                                return;
                            };

                            let Ok(review_time_days) =
                                review_settings.review_time_days.parse::<u64>()
                            else {
                                let _ = interaction.edit_response(&ctx, EditInteractionResponse::new().content("Invalid review time provided. Input must only be composed of numeric characters and cannot be a decimal or negative.")).await;
                                return;
                            };

                            if let Err(e) = gh
                                .octo_install
                                .issues(&gh.repo_owner, &gh.repo_name)
                                .add_labels(discussion.pr_id, &vec![under_review_label])
                                .await
                            {
                                error!(
                                    "Failed to set under review label for PR #{}: {e:#?}",
                                    discussion.pr_id
                                );
                                return;
                            }

                            if let Err(e) = gh
                                .octo_install
                                .issues(&gh.repo_owner, &gh.repo_name)
                                .create_comment(discussion.pr_id, format!(
                                    "**Triaged by {}:** This PR requires a content review discussion, which will be held in public.{}",
                                    interaction.user.name,
                                    if let Some(reasoning) = review_settings.reasoning {
                                        format!("\n```{reasoning}```")
                                    } else
                                    {
                                        String::new()
                                    }
                                ))
                                .await
                            {
                                error!(
                                    "Failed to set under review label for PR #{}: {e:#?}",
                                    discussion.pr_id
                                );
                                return;
                            }

                            let new_thread = match public_forum
                                .create_forum_post(
                                    &ctx,
                                    CreateForumPost::new(
                                        format!("{} #{}", discussion.pr_title, discussion.pr_id),
                                        CreateMessage::new().add_embed(create_pr_embed(
                                            discussion.pr_id,
                                            discussion.pr_title.clone(),
                                            discussion.pr_author.clone(),
                                            discussion.pr_body.clone(),
                                            &gh,
                                        )),
                                    ),
                                )
                                .await
                            {
                                Ok(x) => x,
                                Err(e) => {
                                    error!(
                                        "Failed to create forum post to start review of PR #{}: {e:#?}",
                                        discussion.pr_id
                                    );
                                    let _ = interaction.edit_response(&ctx, error_response).await;
                                    return;
                                }
                            };
                            if let Err(()) = discussion.set_thread_id(&db, new_thread.id).await {
                                let _ = interaction.edit_response(&ctx, error_response).await;
                                return;
                            }

                            if let Err(()) =
                                discussion.setup_review_time(&db, review_time_days).await
                            {
                                let _ = interaction.edit_response(&ctx, error_response).await;
                                return;
                            }

                            if let Err(e) = intake_thread.delete(&ctx).await {
                                error!("Failed to delete intake discussion for pr {pr_id}: {e:#?}");
                                let _ = interaction.edit_response(&ctx, error_response).await;
                            }
                        });
                    }
                    BUTTON_ID_ACTION_START_PRIVATE => {}
                    BUTTON_ID_ACTION_NOT_NEEDED => {
                        let _ = interaction
                            .create_response(&ctx, CreateInteractionResponse::Acknowledge)
                            .await;
                        let Some(no_review_needed_label) =
                            Config::get_no_review_needed_label(&db).await
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

                        if let Err(e) = intake_thread.delete(&ctx).await {
                            error!("Failed to delete intake discussion for pr {pr_id}: {e:#?}");
                            let _ = interaction.edit_response(&ctx, error_response).await;
                            continue;
                        }
                    }
                    action => {
                        error!("Received button press with invalid action {}", action);
                        let _ = interaction.edit_response(&ctx, error_response).await;
                        continue;
                    }
                }
            }
            _ => {}
        }
    }
}
