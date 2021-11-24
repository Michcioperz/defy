use std::sync::Arc;

use axum::{
    extract::{Extension, Query},
    routing::get,
    AddExtensionLayer, Router,
};
use color_eyre::eyre::Context;

use rspotify::{
    clients::{BaseClient, OAuthClient},
    AuthCodeSpotify,
};
use tokio::sync::{oneshot, Mutex};
use tracing::instrument;

#[instrument]
pub(crate) async fn kickstart() -> color_eyre::Result<Client> {
    if let Ok(client) = authed_client().await {
        return Ok(client);
    }
    loop {
        match authed_client().await {
            Ok(client) => return Ok(client),
            _ => {
                let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
                let txs = Arc::new(Mutex::new(Some(shutdown_tx)));
                let app = Router::new()
                    .route("/api/callback", get(auth_callback))
                    .layer(AddExtensionLayer::new(txs));
                let bound_server = axum::Server::bind(
                    &"127.0.0.1:3000"
                        .parse()
                        .wrap_err("cannot parse bind address")?,
                )
                .serve(app.into_make_service());

                let login_url = base_client().get_authorize_url(false)?;
                webbrowser::open(&login_url)?;

                bound_server
                    .with_graceful_shutdown(async move {
                        shutdown_rx.await.unwrap();
                    })
                    .await?;
            }
        }
    }
}

#[instrument]
fn base_client() -> Client {
    const REDIRECT_URL: &str = "http://localhost:3000/api/callback";
    rspotify::AuthCodeSpotify::with_config(
        rspotify::Credentials::from_env().expect("missing credentials in env"),
        rspotify::OAuth {
            redirect_uri: REDIRECT_URL.to_string(),
            scopes: rspotify::scopes!(
                "user-library-read",
                "playlist-read-private",
                "playlist-modify-private",
                "playlist-modify-public",
                "streaming",
                "user-read-email",
                "user-read-private"
            ),
            ..Default::default()
        },
        rspotify::Config {
            token_cached: true,
            token_refreshing: true,
            ..Default::default()
        },
    )
}

#[instrument]
async fn authed_client() -> Result<Client, String> {
    let mut client = base_client();
    match client.read_token_cache().await {
        Ok(Some(token)) => Ok(Client::from_token(token)),
        _ => Err("unauthenticated".to_string()),
    }
}

#[derive(serde::Deserialize)]
struct AuthCallbackQuery {
    code: String,
    state: String,
}

#[instrument]
async fn auth_callback(
    Query(AuthCallbackQuery { code, state }): Query<AuthCallbackQuery>,
    Extension(txs): Extension<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
) -> &'static str {
    let mut client = base_client();
    client.oauth.state = state;
    client
        .request_token(&code)
        .await
        .expect("requesting token failed");
    client
        .write_token_cache()
        .await
        .expect("writing token cache failed");
    txs.lock()
        .await
        .take()
        .expect("auth race lost")
        .send(())
        .unwrap();
    "ok"
}

pub type Client = AuthCodeSpotify;
