use color_eyre::Result;
use linfa::Dataset;
use ndarray::Array2;
use rspotify::model::AudioFeatures;
use tracing::{info, instrument};

#[instrument(skip(db))]
pub(crate) async fn feature_dataset_for_fitting(
    db: sled::Db,
    feature_name: &str,
) -> Result<Dataset<f32, bool>> {
    let features_tree = db.open_tree("track_features")?;
    let feature_tree = db.open_tree(format!("input/{}", feature_name))?;
    let mut features = vec![];
    let mut targets = vec![];
    for it in feature_tree.iter() {
        let (id, target_bytes) = it?;
        if let Some(features_bytes) = features_tree.get(id)? {
            let features_option: AudioFeatures = serde_json::from_slice(&features_bytes)?;
            if let Some(features_object) = features_option {
                features.extend_from_slice(&vec![
                    features_object.acousticness,
                    features_object.danceability,
                    features_object.energy,
                    features_object.instrumentalness,
                    features_object.key as f32,
                    features_object.liveness,
                    features_object.loudness,
                    features_object.speechiness,
                    features_object.tempo,
                    features_object.time_signature as f32,
                    features_object.valence,
                ]);
                targets.push(target_bytes[0] > 0);
            }
        }
    }
    let feature_names = vec![
        "acousticness",
        "danceability",
        "energy",
        "instrumentalness",
        "key",
        "liveness",
        "loudness",
        "speechiness",
        "tempo",
        "time_signature",
        "valence",
    ];
    let dataset = Dataset::new(
        Array2::from_shape_vec((targets.len(), feature_names.len()), features)?,
        Array2::from_shape_vec((targets.len(), 1), targets)?,
    )
    .with_feature_names(feature_names);
    info!(dim = ?dataset.records().dim());
    Ok(dataset)
}

pub(crate) async fn feature_dataset_for_prediction(db: sled::Db) -> Result<Dataset<f32, String>> {
    let features_tree = db.open_tree("track_features")?;
    let mut features = vec![];
    let mut targets = vec![];
    for it in features_tree.iter() {
        let (id_bytes, features_bytes) = it?;
        let features_option: AudioFeatures = serde_json::from_slice(&features_bytes)?;
        if let Some(features_object) = features_option {
            features.extend_from_slice(&vec![
                features_object.acousticness,
                features_object.danceability,
                features_object.energy,
                features_object.instrumentalness,
                features_object.key as f32,
                features_object.liveness,
                features_object.loudness,
                features_object.speechiness,
                features_object.tempo,
                features_object.time_signature as f32,
                features_object.valence,
            ]);
            targets.push(String::from_utf8_lossy(&id_bytes).to_string());
        }
    }
    let feature_names = vec![
        "acousticness",
        "danceability",
        "energy",
        "instrumentalness",
        "key",
        "liveness",
        "loudness",
        "speechiness",
        "tempo",
        "time_signature",
        "valence",
    ];
    let dataset = Dataset::new(
        Array2::from_shape_vec((targets.len(), feature_names.len()), features)?,
        Array2::from_shape_vec((targets.len(), 1), targets)?,
    )
    .with_feature_names(feature_names);
    info!(dim = ?dataset.records().dim());
    Ok(dataset)
}
