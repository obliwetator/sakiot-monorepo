use std::collections::HashMap;

#[derive(serde::Serialize, Debug)]
pub struct Directories {
    pub year: i32,
    pub months: Option<Months>,
}

#[derive(serde::Serialize, Debug)]
pub struct Channels {
    pub channel_id: String,
    pub dirs: Vec<Directories>,
}

#[derive(serde::Serialize, Debug)]
pub struct File {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip)]
    pub start_ts_ms: Option<i64>,
}

pub type Months = HashMap<i32, Option<Vec<File>>>;

#[derive(serde::Deserialize, Debug)]
pub struct StartEnd {
    pub start: Option<f32>,
    pub end: Option<f32>,
    pub name: Option<String>,
}

#[derive(Debug)]
pub struct HashMapContainer(
    pub tokio::sync::RwLock<HashMap<String, tokio::sync::broadcast::Sender<i32>>>,
);

#[derive(Debug)]
pub struct WaveformProgressContainer(pub tokio::sync::RwLock<HashMap<String, i16>>);
