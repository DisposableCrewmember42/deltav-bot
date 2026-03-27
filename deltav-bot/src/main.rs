use jsonwebtoken::EncodingKey;
use octocrab::models::AppId;
use tracing::error;

use crate::github::{GhAppConfig, GitHubService};

mod direction;
mod github;
mod relays;

macro_rules! required_env {
    ($variable_name: expr, $local_var: ident) => {
        let val: &str = $variable_name;
        let Ok($local_var) = std::env::var(val) else {
            tracing::error!("Missing environment variable: {}", val);
            return;
        };
    };
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    required_env!("GH_REPO_OWNER", repo_owner);
    required_env!("GH_REPO_NAME", repo_name);
    required_env!("GH_APP_KEY", app_key);
    let Ok(app_key) = EncodingKey::from_rsa_pem(app_key.as_bytes()) else {
        error!("Invalid GitHub Application Key. Must be an RSA key in PEM format.");
        return;
    };
    required_env!("GH_APP_ID", app_id);
    let Ok(app_id) = app_id.parse::<u64>() else {
        error!("Invalid GitHub Application ID. Must be u64.");
        return;
    };

    required_env!("WEBHOOK_PORT", webhook_port);
    let Ok(webhook_port) = webhook_port.parse::<u16>() else {
        error!("Invalid Webhook port. Must be u16.");
        return;
    };
    required_env!("WEBHOOK_SECRET", webhook_secret);
    if webhook_secret.len() < 16 {
        error!("The Webhook secret must be at least 16 characters long.");
        return;
    }

    required_env!("DISCORD_TOKEN", discord_token);

    let Ok(mut gh) = GitHubService::initialize(
        webhook_port,
        webhook_secret,
        GhAppConfig {
            id: AppId(app_id),
            key: app_key,
            repo_owner,
            repo_name,
        },
    )
    .await
    else {
        error!("GitHub integration failed to initialize, shutting down!");
        return;
    };

    while let Some(message) = gh.webhook_receiver.recv().await {
        println!("{message:#?}")
    }

    let _ = gh.webhook_thread.await;
}
