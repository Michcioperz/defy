use std::{collections::VecDeque, iter::FromIterator, str::FromStr};

use color_eyre::Result;
use futures_util::StreamExt;
use itertools::Itertools;
use kickstart::Client;
use rspotify::{
    clients::{BaseClient, OAuthClient},
    model::{
        FullAlbum, FullTrack, Id, PlayableId, PlayableItem, PlaylistId, SavedAlbum,
        SimplifiedTrack, TrackId,
    },
};
use sled::Db;
use tracing::{info, instrument};

mod data_input;
mod kickstart;

#[tokio::main]
async fn main() -> Result<()> {
    {
        use tracing_error::ErrorLayer;
        use tracing_subscriber::{fmt, prelude::*, EnvFilter};
        tracing_subscriber::registry()
            .with(ErrorLayer::default())
            .with(
                EnvFilter::try_from_default_env()
                    .or_else(|_| EnvFilter::try_new("info,rspotify_http=warn"))
                    .unwrap(),
            )
            .with(fmt::layer())
            .init();
    }
    color_eyre::install()?;

    info!("obtaining client");
    let client = kickstart::kickstart().await?;
    info!("opening database");
    let db = sled::open("db").unwrap();
    if std::env::var("SKIP_POPULATING").is_ok() {
        info!("skipping database populating")
    } else {
        info!("populating database");
        populate_database(&client, db.clone()).await?;
    }
    info!("launching data input interface");
    data_input::web_interface(db.clone(), client.clone()).await?;
    info!("performing programmed actions");
    perform_update(&client).await?;

    Ok(())
}

#[instrument(skip(client))]
async fn fetch_playlist(client: &Client, id: &PlaylistId) -> Result<Vec<FullTrack>> {
    use rspotify::{model::PlaylistItem, ClientError};
    let result: Vec<Result<PlaylistItem, ClientError>> =
        client.playlist_items(id, None, None).collect().await;
    let result: Result<Vec<PlaylistItem>, ClientError> = result.into_iter().collect();
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

#[instrument(skip(client, album), fields(album.id = ?album.id, album.title = ?album.name))]
async fn fetch_album_tracks(client: &Client, album: &FullAlbum) -> Result<Vec<SimplifiedTrack>> {
    use rspotify::ClientError;
    let result: Vec<Result<SimplifiedTrack, ClientError>> =
        client.album_track(&album.id).collect().await;
    let result: Result<Vec<SimplifiedTrack>, ClientError> = result.into_iter().collect();
    Ok(result?)
}

#[instrument(skip(client))]
async fn fetch_library_album_tracks(client: &Client) -> Result<Vec<SimplifiedTrack>> {
    let albums = fetch_library_albums(client).await?;
    let mut result = vec![];
    for album in albums {
        result.extend(fetch_album_tracks(client, &album.album).await?);
    }
    Ok(result)
}

#[instrument(skip(client))]
async fn fetch_library_albums(client: &Client) -> Result<Vec<SavedAlbum>> {
    use rspotify::ClientError;
    let result: Vec<Result<SavedAlbum, ClientError>> =
        client.current_user_saved_albums(None).collect().await;
    let result: Result<Vec<SavedAlbum>, ClientError> = result.into_iter().collect();
    Ok(result?)
}

#[instrument(skip(client, tracks), fields(tracks.len = tracks.len()))]
async fn write_playlist(client: &Client, id: &PlaylistId, tracks: Vec<FullTrack>) -> Result<()> {
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

#[instrument(skip(client))]
async fn perform_update(client: &Client) -> Result<()> {
    let main_playlist = fetch_playlist(
        &client,
        &PlaylistId::from_str("6CmOKM7D0nvMM1h1GQTl1L").unwrap(),
    )
    .await?;

    let reduced_tracks = main_playlist.iter().rev().take(100).cloned().collect();
    write_playlist(
        &client,
        &PlaylistId::from_str("02S7eexioL9T1xWOP53hlK").unwrap(),
        reduced_tracks,
    )
    .await?;
    Ok(())
}

#[instrument(skip(track), fields(track.id = ?track.id))]
fn simplify_track(track: FullTrack) -> SimplifiedTrack {
    let FullTrack {
        artists,
        available_markets,
        disc_number,
        duration,
        explicit,
        external_urls,
        href,
        id,
        is_local,
        is_playable,
        linked_from,
        restrictions,
        name,
        preview_url,
        track_number,
        ..
    } = track;
    SimplifiedTrack {
        artists,
        available_markets: Some(available_markets),
        disc_number,
        duration,
        explicit,
        external_urls,
        href,
        id: Some(id),
        is_local,
        is_playable,
        linked_from,
        restrictions,
        name,
        preview_url,
        track_number,
    }
}

#[instrument(skip(client, db))]
async fn populate_database(client: &Client, db: Db) -> Result<()> {
    info!("fetching main playlist");
    let main_playlist = fetch_playlist(
        &client,
        &PlaylistId::from_str("6CmOKM7D0nvMM1h1GQTl1L").unwrap(),
    )
    .await?
    .into_iter()
    .map(simplify_track);

    info!("fetching library album tracks");
    let library = fetch_library_album_tracks(&client).await?;
    let all_tracks: Vec<SimplifiedTrack> = main_playlist
        .into_iter()
        .chain(library.into_iter())
        .filter(|track| track.id.is_some())
        .collect();
    let tracks_db = db.open_tree("track_details")?;
    for track in all_tracks.iter() {
        tracks_db
            .insert(
                track.id.clone().unwrap().id(),
                serde_json::to_vec(track).unwrap(),
            )
            .unwrap();
    }

    info!("fetching missing features");
    let features_db = db.open_tree("track_features")?;
    let mut fetched_features = 0usize;
    for page in &tracks_db
        .into_iter()
        .map(Result::unwrap)
        .map(|(key, _value)| key)
        .filter(|key| !features_db.contains_key(key).unwrap())
        .chunks(100)
    {
        let page = page
            .map(|key| TrackId::from_id(std::str::from_utf8(&key).unwrap()).unwrap())
            .collect_vec();
        for (track_id, featureset) in page
            .iter()
            .zip(client.tracks_features(&page).await?.unwrap_or(vec![]))
        {
            features_db
                .insert(track_id.id(), serde_json::to_vec(&featureset).unwrap())
                .unwrap();
        }
        fetched_features += page.len();
    }
    info!(?fetched_features);

    Ok(())
}
