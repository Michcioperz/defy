use std::{collections::VecDeque, convert::TryInto, iter::FromIterator, str::FromStr};

use axum::{Router, body::{Bytes, Empty}, extract::{Query, TypedHeader}, handler::{Handler, get}, response::{IntoResponse, Redirect}};
use cookie::Cookie;
use futures_util::StreamExt;
use http::Response;
use rspotify::{
    clients::{BaseClient, OAuthClient},
    model::{FullTrack, PlayableId, PlayableItem, PlaylistId, PlaylistItem, TrackId},
    AuthCodeSpotify, ClientResult, Token,
};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let app = Router::new()
        .route("/api/perform_update", get(perform_update))
        .route("/api/callback", get(auth_callback))
        .route("/api/log_in", get(log_in));
    axum::Server::bind(&"127.0.0.1:3000".parse().expect("cannot parse bind address"))
        // .serve(app.into_make_service())
        .await
        .expect("failure while serving")
}

fn base_client() -> rspotify::AuthCodeSpotify {
    const REDIRECT_URL: &str = "http://localhost:3000/api/callback";
    rspotify::AuthCodeSpotify::new(
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
    )
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

const COOKIE_NAME: &str = "defy_token";

async fn auth_callback(
    Query(AuthCallbackQuery { code, state }): Query<AuthCallbackQuery>,
) -> Result<Response<Empty<Bytes>>, String> {
    let mut client = base_client();
    client.oauth.state = state;
    client
        .request_token(&code)
        .await
        .map_err(|e| e.to_string())?;
    let mut response = Redirect::to("/".try_into().unwrap()).into_response();
    let cookies = response.headers_mut();
    cookies.insert(
        http::header::SET_COOKIE,
        Cookie::build(
            COOKIE_NAME,
            serde_json::to_string_pretty(&client.token.lock().await.unwrap().clone().unwrap())
                .unwrap(),
        )
        .permanent()
        .http_only(true)
        .finish()
        .to_string()
        .try_into()
        .expect("failed to build refresh token cookie"),
    );
    Ok(response)
}

type Client = AuthCodeSpotify;

struct RefreshToken(Token);

impl RefreshToken {
    async fn into_client(self) -> Client {
        AuthCodeSpotify::from_token(self.0)
    }
}

impl headers::Header for RefreshToken {
    fn name() -> &'static headers::HeaderName {
        &http::header::COOKIE
    }

    fn decode<'i, I>(values: &mut I) -> Result<Self, headers::Error>
    where
        Self: Sized,
        I: Iterator<Item = &'i http::HeaderValue>,
    {
        let cookies = headers::Cookie::decode(values)?;
        cookies
            .get(COOKIE_NAME)
            .ok_or_else(headers::Error::invalid)
            .and_then(|s| {
                serde_json::from_str(s)
                    .map_err(|_| headers::Error::invalid())
                    .map(|token| RefreshToken(token))
            })
    }

    fn encode<E: Extend<http::HeaderValue>>(&self, _values: &mut E) {
        unimplemented!()
    }
}

async fn fetch_playlist(client: &Client, id: &PlaylistId) -> anyhow::Result<Vec<FullTrack>> {
    let intermediate: Vec<ClientResult<PlaylistItem>> =
        client.playlist_items(id, None, None).collect().await;
    let result: ClientResult<Vec<PlaylistItem>> = intermediate.into_iter().collect();
    Ok(result?
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
        let batch: Vec<&dyn PlayableId> = batch.iter().map(|id| id as &dyn PlayableId).collect();
        client
            .playlist_add_items(id, batch.into_iter(), Some(i * 100))
            .await?;
    }
    Ok(())
}

async fn perform_update(
    TypedHeader(refresh_token): TypedHeader<RefreshToken>,
) -> Result<Response<Empty<Bytes>>, String> {
    let client = refresh_token.into_client().await;

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
