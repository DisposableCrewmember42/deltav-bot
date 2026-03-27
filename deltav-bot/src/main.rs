use jsonwebtoken::EncodingKey;
use octocrab::models::AppId;
use tracing::error;

use crate::github::{GhAppConfig, GitHubService};

mod direction;
mod github;
mod relays;

macro_rules! get_env {
    ($variable_name: expr, $local_var: ident) => {
        let val: &str = $variable_name;
        let Ok($local_var) = std::env::var(val) else {
            tracing::error!("Missing {}", stringify!($variable_name));
            return;
        };
    };
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    get_env!("GH_REPO_OWNER", repo_owner);
    get_env!("GH_REPO_NAME", repo_name);
    get_env!("GH_APP_KEY", app_key);
    let Ok(app_key) = EncodingKey::from_rsa_pem(app_key.as_bytes()) else {
        error!("Invalid GitHub Application Key. Must be an RSA key in PEM format.");
        return;
    };
    get_env!("GH_APP_ID", app_id);
    let Ok(app_id) = app_id.parse::<u64>() else {
        error!("Invalid GitHub Application ID. Must be u64.");
        return;
    };

    get_env!("DISCORD_TOKEN", discord_token);

    let Ok(mut gh) = GitHubService::initialize(
        8080,
        "yeet".into(),
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
