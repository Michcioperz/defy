use std::{collections::VecDeque, convert::TryInto};

use aspotify::{Client, ClientCredentials, PlaylistItem, PlaylistItemType, Scope, Track};
use axum::{
    body::{Bytes, Empty},
    extract::{Query, TypedHeader},
    handler::get,
    response::{IntoResponse, Redirect},
    Router,
};
use cookie::Cookie;
use http::Response;
use lazy_static::lazy_static;
use url::Url;

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

fn scopes() -> Vec<Scope> {
    lazy_static! {
        static ref SCOPES: Vec<Scope> = vec![
            Scope::UserLibraryRead,
            Scope::PlaylistReadPrivate,
            Scope::PlaylistModifyPrivate,
            Scope::PlaylistModifyPublic,
        ];
    }
    SCOPES.clone()
}

fn credentials<'a>() -> &'a ClientCredentials {
    lazy_static! {
        static ref CREDS: ClientCredentials =
            ClientCredentials::from_env().expect("missing or invalid credentials in env");
    }
    &CREDS
}

const REDIRECT_URL: &str = "http://localhost:3000/api/callback";

async fn log_in() -> Redirect {
    let (url, _state) =
        aspotify::authorization_url(&credentials().id, scopes(), false, REDIRECT_URL);
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
    let location = Url::parse_with_params(REDIRECT_URL, [("code", &code), ("state", &state)])
        .expect("failed to reconstruct callback location");
    let client = Client::new(credentials().clone());
    client
        .redirected(location.as_str(), &state)
        .await
        .map_err(|e| e.to_string())?;
    let mut response = Redirect::to("/".try_into().unwrap()).into_response();
    let cookies = response.headers_mut();
    cookies.insert(
        http::header::SET_COOKIE,
        Cookie::build("refresh_token", client.refresh_token().await.unwrap())
            .permanent()
            .http_only(true)
            .finish()
            .to_string()
            .try_into()
            .expect("failed to build refresh token cookie"),
    );
    Ok(response)
}

async fn fetch_playlist(client: &Client, id: &str) -> anyhow::Result<Vec<PlaylistItem>> {
    let mut acc = vec![];
    for i in 0.. {
        let page = client
            .playlists()
            .get_playlists_items(id, 100, i * 100, None)
            .await?
            .data;
        acc.extend(page.items);
        if acc.len() >= page.total {
            break;
        }
    }
    Ok(acc)
}

async fn write_playlist(client: &Client, id: &str, tracks: Vec<Track>) -> anyhow::Result<()> {
    let mut tracks: VecDeque<_> = tracks
        .into_iter()
        .flat_map(|track| track.id.map(PlaylistItemType::<String, String>::Track))
        .collect();
    client
        .playlists()
        .replace_playlists_items(id, Vec::<PlaylistItemType<String, String>>::new())
        .await?;
    for i in 0.. {
        if tracks.is_empty() {
            break;
        }
        let batch = tracks.drain(0..100);
        client
            .playlists()
            .add_to_playlist(id, batch, Some(i * 100))
            .await?;
    }
    Ok(())
}

struct RefreshToken(String);

impl RefreshToken {
    async fn into_client(self) -> Client {
        let client = Client::new(credentials().clone());
        client.set_refresh_token(Some(self.0)).await;
        client
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
            .get("refresh_token")
            .ok_or_else(headers::Error::invalid)
            .map(|token| RefreshToken(token.to_string()))
    }

    fn encode<E: Extend<http::HeaderValue>>(&self, _values: &mut E) {
        unimplemented!()
    }
}

async fn perform_update(
    TypedHeader(refresh_token): TypedHeader<RefreshToken>,
) -> Result<Response<Empty<Bytes>>, String> {
    let client = refresh_token.into_client().await;

    let main_playlist = fetch_playlist(&client, "6CmOKM7D0nvMM1h1GQTl1L")
        .await
        .map_err(|e| e.to_string())?;

    let reduced_tracks = main_playlist
        .iter()
        .rev()
        .take(100)
        .filter_map(|item| {
            if let Some(aspotify::PlaylistItemType::Track(track)) = &item.item {
                Some(track.clone())
            } else {
                None
            }
        })
        .collect();
    write_playlist(&client, "02S7eexioL9T1xWOP53hlK", reduced_tracks)
        .await
        .map_err(|e| e.to_string())?;

    let response = Response::default();
    Ok(response)
}
