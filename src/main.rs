use std::{collections::VecDeque, convert::TryInto, iter::FromIterator, str::FromStr};

use axum::{
    body::{Bytes, Empty},
    extract::Query,
    handler::get,
    response::{IntoResponse, Redirect},
    Router,
};

use http::Response;
use rspotify::{
    clients::{BaseClient, OAuthClient},
    model::{FullTrack, PlayableId, PlayableItem, PlaylistId, TrackId},
    AuthCodeSpotify,
};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let app = Router::new()
        .route("/api/perform_update", get(perform_update))
        .route("/api/callback", get(auth_callback))
        .route("/api/log_in", get(log_in));
    axum::Server::bind(&"127.0.0.1:3000".parse().expect("cannot parse bind address"))
        .serve(app.into_make_service())
        .await
        .expect("failure while serving")
}

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
                "playlist-modify-public"
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

async fn authed_client() -> Result<Client, String> {
    let mut client = base_client();
    match client.read_token_cache().await {
        Ok(Some(token)) => Ok(Client::from_token(token)),
        _ => Err("unauthenticated".to_string()),
    }
}

async fn log_in() -> Redirect {
    let client = base_client();
    let url = client
        .get_authorize_url(false)
        .expect("failed to build authorization url");
    Redirect::to(url.try_into().expect("failed to build authorization url"))
}

#[derive(serde::Deserialize)]
struct AuthCallbackQuery {
    code: String,
    state: String,
}

async fn auth_callback(
    Query(AuthCallbackQuery { code, state }): Query<AuthCallbackQuery>,
) -> Result<Response<Empty<Bytes>>, String> {
    let mut client = base_client();
    client.oauth.state = state;
    client
        .request_token(&code)
        .await
        .map_err(|e| e.to_string())?;
    client
        .write_token_cache()
        .await
        .map_err(|e| e.to_string())?;
    let response = Redirect::to("/".try_into().unwrap()).into_response();
    Ok(response)
}

type Client = AuthCodeSpotify;

async fn fetch_playlist(client: &Client, id: &PlaylistId) -> anyhow::Result<Vec<FullTrack>> {
    const PAGE_SIZE: u32 = 100;
    let mut result = vec![];
    for i in 0.. {
        let page = client
            .playlist_items_manual(id, None, None, Some(PAGE_SIZE), Some(i * PAGE_SIZE))
            .await?;
        if page.items.is_empty() {
            break;
        }
        result.extend(page.items);
    }
    Ok(result
        .into_iter()
        .filter_map(|item| {
            if let Some(PlayableItem::Track(track)) = &item.track {
                Some(track.clone())
            } else {
                None
            }
        })
        .collect())
}

async fn write_playlist(
    client: &Client,
    id: &PlaylistId,
    tracks: Vec<FullTrack>,
) -> anyhow::Result<()> {
    client.playlist_replace_items(id, vec![]).await?;
    let mut tracks = VecDeque::from_iter(tracks.into_iter());
    for i in 0.. {
        if tracks.is_empty() {
            break;
        }
        let batch: Vec<TrackId> = tracks.drain(0..100).map(|track| track.id).collect();

        client
            .playlist_add_items(
                id,
                batch.iter().map(|id| id as &dyn PlayableId),
                Some(i * 100),
            )
            .await?;
    }
    Ok(())
}

async fn perform_update() -> Result<Response<Empty<Bytes>>, String> {
    let client = authed_client().await?;

    let main_playlist = fetch_playlist(
        &client,
        &PlaylistId::from_str("6CmOKM7D0nvMM1h1GQTl1L").unwrap(),
    )
    .await
    .map_err(|e| e.to_string())?;

    let reduced_tracks = main_playlist.iter().rev().take(100).cloned().collect();
    write_playlist(
        &client,
        &PlaylistId::from_str("02S7eexioL9T1xWOP53hlK").unwrap(),
        reduced_tracks,
    )
    .await
    .map_err(|e| e.to_string())?;

    let response = Response::default();
    Ok(response)
}
