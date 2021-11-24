use std::sync::Arc;

use axum::{
    error_handling::HandleErrorExt,
    extract::{Extension, Path},
    response::{Html, IntoResponse},
    routing::{get, post, service_method_routing},
    AddExtensionLayer, Json, Router,
};
use color_eyre::eyre::{eyre, Context};
use rspotify::{clients::BaseClient, model::SimplifiedTrack};
use sled::Db;
use tokio::sync::{oneshot, Mutex};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::instrument;

use crate::kickstart::Client;

// type Result<T> = std::result::Result<T, String>;
type Result<T> = std::result::Result<T, StringableReport>;
type State = (Db, Client, Arc<Mutex<Option<oneshot::Sender<()>>>>);

#[instrument(skip(db))]
pub(crate) async fn web_interface(db: Db, client: Client) -> color_eyre::Result<()> {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state: State = (db, client, Arc::new(Mutex::new(Some(shutdown_tx))));

    let app = Router::new()
        .nest(
            "/api",
            Router::new()
                .nest(
                    "/features",
                    Router::new()
                        .nest(
                            "/:feature_id",
                            Router::new()
                                .nest(
                                    "/tracks",
                                    Router::new()
                                        .route(
                                            "/random_untrained",
                                            get(random_untrained_track_for_feature),
                                        )
                                        .nest(
                                            "/:track_id",
                                            Router::new().route(
                                                "/rate/:rating",
                                                post(rate_feature_for_track),
                                            ),
                                        ),
                                )
                                .route("/", post(create_feature)),
                        )
                        .route("/", get(list_features)),
                )
                .route("/spotify_token", get(spotify_token))
                .route("/shutdown", post(shutdown)),
        )
        .route("/", get(data_input_html))
        .nest(
            "/static",
            service_method_routing::get(ServeDir::new("static"))
                .handle_error(|error: std::io::Error| StringableReport(error.into())),
        )
        .layer(AddExtensionLayer::new(state))
        .layer(TraceLayer::new_for_http());
    let bound_server = axum::Server::bind(
        &"127.0.0.1:3000"
            .parse()
            .wrap_err("cannot parse bind address")?,
    )
    .serve(app.into_make_service());

    webbrowser::open("http://127.0.0.1:3000/")?;

    bound_server
        .with_graceful_shutdown(async move {
            shutdown_rx.await.unwrap();
        })
        .await?;
    Ok(())
}

#[instrument(skip(db))]
async fn list_features(Extension((db, _, _)): Extension<State>) -> Result<Json<Vec<String>>> {
    Ok(Json(
        db.tree_names()
            .into_iter()
            .map(|name| String::from_utf8_lossy(&name).to_string())
            .filter_map(|name| name.strip_prefix("input/").map(|s| s.to_string()))
            .collect(),
    ))
}

#[instrument(skip(db))]
async fn create_feature(
    Extension((db, _, _)): Extension<State>,
    Path(feature_id): Path<String>,
) -> Result<&'static str> {
    db.open_tree(format!("input/{}", feature_id))?;
    Ok("ok")
}

#[instrument(skip(db))]
async fn random_untrained_track_for_feature(
    Extension((db, _, _)): Extension<State>,
    Path(feature_id): Path<String>,
) -> Result<Json<SimplifiedTrack>> {
    let details_tree = db.open_tree("track_details")?;
    let features_tree = db.open_tree("track_features")?;
    let feature_tree = db.open_tree(format!("input/{}", feature_id))?;
    let null_ivec = sled::IVec::from(serde_json::to_vec(&serde_json::Value::Null)?);
    for it in details_tree.iter() {
        let (id, details_vec) = it?;
        if !feature_tree.contains_key(&id)? {
            match features_tree.get(id)? {
                None => continue,
                Some(val) if val == null_ivec => continue,
                Some(_) => (),
            }
            let details: SimplifiedTrack = serde_json::from_slice(&details_vec)?;
            if !details
                .available_markets
                .as_ref()
                .map_or(false, |available_markets| {
                    available_markets.iter().any(|market| market == "PL")
                })
            {
                continue;
            }
            return Ok(Json(details));
        }
    }
    Err(eyre!("no more tracks").into())
}

#[instrument(skip(db))]
async fn rate_feature_for_track(
    Extension((db, _, _)): Extension<State>,
    Path((feature_id, track_id, rating)): Path<(String, String, u8)>,
) -> Result<&'static str> {
    let feature_tree = db.open_tree(format!("input/{}", feature_id))?;
    feature_tree.insert(track_id, &[rating])?;
    Ok("ok")
}

#[instrument(skip(client))]
async fn spotify_token(Extension((_, client, _)): Extension<State>) -> Result<String> {
    let token = client.get_token().lock().await.unwrap().clone().unwrap();
    Ok(token.access_token)
}

#[instrument(skip(shutdown_mechanism))]
async fn shutdown(Extension((_, _, shutdown_mechanism)): Extension<State>) -> Result<&'static str> {
    shutdown_mechanism
        .lock()
        .await
        .take()
        .expect("shutdown race lost")
        .send(())
        .unwrap();
    Ok("ok")
}

#[instrument]
async fn data_input_html() -> Result<Html<String>> {
    Ok(Html(
        maud::html! {
            (maud::DOCTYPE)
            html {
                head {
                    meta charset="UTF-8";
                }
                body {
                    script src="/static/data_input.js" {}
                }
            }
        }
        .into_string(),
    ))
}

#[derive(Debug)]
struct StringableReport(color_eyre::Report);

impl<T: Into<color_eyre::Report>> From<T> for StringableReport {
    fn from(t: T) -> Self {
        Self(t.into())
    }
}

impl IntoResponse for StringableReport {
    type Body = <String as IntoResponse>::Body;
    type BodyError = <String as IntoResponse>::BodyError;
    fn into_response(self) -> axum::http::Response<Self::Body> {
        self.0.to_string().into_response()
    }
}
