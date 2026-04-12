use std::sync::Arc;

use poise::serenity_prelude::{
    ComponentInteractionCollector, ComponentInteractionDataKind, CreateInteractionResponse,
    EditInteractionResponse,
};
use sqlx::{Pool, Sqlite};
use tracing::error;

use crate::{
    discord::{
        content_review::{
            BUTTON_ID_ACTION_NOT_NEEDED, BUTTON_ID_ACTION_START_PRIVATE,
            BUTTON_ID_ACTION_START_PUBLIC, BUTTON_ID_PREFIX, discussion_channel_to_guild,
        },
        data::{config::Config, discussions::DiscussionRecord},
    },
    github::GitHub,
};

pub async fn cr_component_task(
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
                    BUTTON_ID_ACTION_START_PUBLIC => {}
                    BUTTON_ID_ACTION_START_PRIVATE => {}
                    BUTTON_ID_ACTION_NOT_NEEDED => {
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
