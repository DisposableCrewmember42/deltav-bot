use std::time::Duration;

use jsonwebtoken::EncodingKey;
use octocrab::models::AppId;
use tracing::{Level, error, info, span};

use crate::github::{GhAppConfig, GitHubService};

use fred::prelude::{ClientLike, Config as RedisConfig, ReconnectPolicy};
use fred::types::Builder as RedisBuilder;

mod direction;
mod github;
mod relays;

macro_rules! required_env {
    ($variable_name: expr, $local_var: ident) => {
        let val: &str = $variable_name;
        let Ok($local_var) = std::env::var(val) else {
            tracing::error!("[FATAL] Missing environment variable: {}", val);
            return;
        };
    };
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let span = span!(Level::INFO, "main");
    let _enter = span.enter();

    required_env!("GH_REPO_OWNER", repo_owner);
    required_env!("GH_REPO_NAME", repo_name);
    required_env!("GH_APP_KEY", app_key);
    let Ok(app_key) = EncodingKey::from_rsa_pem(app_key.as_bytes()) else {
        error!("[FATAL] Invalid GitHub Application Key. Must be an RSA key in PEM format.");
        return;
    };
    required_env!("GH_APP_ID", app_id);
    let Ok(app_id) = app_id.parse::<u64>() else {
        error!("[FATAL] Invalid GitHub Application ID. Must be u64.");
        return;
    };

    required_env!("WEBHOOK_PORT", webhook_port);
    let Ok(webhook_port) = webhook_port.parse::<u16>() else {
        error!("[FATAL] Invalid Webhook port. Must be u16.");
        return;
    };
    required_env!("WEBHOOK_SECRET", webhook_secret);
    if webhook_secret.len() < 16 {
        error!("[FATAL] The Webhook secret must be at least 16 characters long.");
        return;
    }

    required_env!("DISCORD_TOKEN", discord_token);

    required_env!("REDIS_URL", redis_url);
    let Ok(config) = RedisConfig::from_url(&redis_url) else {
        error!("[FATAL] Failed to create Redis config from URL '{redis_url}'.");
        return;
    };

    info!("Setting up Redis connection.");
    let redis_pool = match RedisBuilder::from_config(config)
        .with_connection_config(|config| {
            config.connection_timeout = Duration::from_secs(10);
        })
        .set_policy(ReconnectPolicy::new_exponential(0, 100, 30_000, 2))
        .build_pool(8)
    {
        Ok(x) => x,
        Err(e) => {
            error!("[FATAL] Failed to build Redis pool: {e:#?}");
            return;
        }
    };

    if let Err(e) = redis_pool.init().await {
        error!("[FATAL] Failed to connect to Redis: {e:#?}");
        return;
    }
    info!("Sucessfully connected to Redis.");

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
        error!("[FATAL] GitHub integration failed to initialize.");
        return;
    };

    while let Some(message) = gh.webhook_receiver.recv().await {
        println!("{message:#?}")
    }

    let _ = gh.webhook_thread.await;
}
