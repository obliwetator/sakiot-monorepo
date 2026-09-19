#![allow(clippy::expect_used)]

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Method, Request, Response, StatusCode},
};
use sakiot_storage::{Archive, ArchiveConfig};
use tokio::sync::Mutex;

#[derive(Clone)]
struct Version {
    key: String,
    id: String,
    marker: bool,
}

#[derive(Default)]
struct FakeState {
    versions: Mutex<Vec<Version>>,
    fail_once: Mutex<bool>,
}

async fn fake(State(state): State<Arc<FakeState>>, request: Request<Body>) -> Response<Body> {
    let query = request.uri().query().unwrap_or("");
    if request.method() == Method::GET && query.contains("versions") {
        let prefix = url::form_urlencoded::parse(query.as_bytes())
            .find(|(name, _)| name == "prefix")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        let mut body = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListVersionsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>test-media-bucket</Name><IsTruncated>false</IsTruncated>",
        );
        for version in state
            .versions
            .lock()
            .await
            .iter()
            .filter(|v| v.key.starts_with(&prefix))
        {
            let tag = if version.marker {
                "DeleteMarker"
            } else {
                "Version"
            };
            body.push_str(&format!(
                "<{tag}><Key>{}</Key><VersionId>{}</VersionId><IsLatest>true</IsLatest></{tag}>",
                version.key, version.id
            ));
        }
        body.push_str("</ListVersionsResult>");
        return Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/xml")
            .body(Body::from(body))
            .expect("valid response");
    }
    if request.method() == Method::DELETE {
        let id = url::form_urlencoded::parse(query.as_bytes())
            .find(|(name, _)| name == "versionId")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        let mut fail_once = state.fail_once.lock().await;
        if *fail_once && id == "v2" {
            *fail_once = false;
            return Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(Body::from("<Error><Code>ServiceUnavailable</Code></Error>"))
                .expect("valid response");
        }
        let mut versions = state.versions.lock().await;
        versions.retain(|version| version.id != id);
        return Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("valid response");
    }
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::empty())
        .expect("valid response")
}

async fn start() -> (Archive, Arc<FakeState>, tokio::task::JoinHandle<()>) {
    let state = Arc::new(FakeState::default());
    let app = Router::new().fallback(fake).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake S3 server");
    let address: SocketAddr = listener.local_addr().expect("fake S3 address");
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let archive = Archive::new(&ArchiveConfig {
        endpoint: format!("http://{address}"),
        region: "test-region".into(),
        bucket: "test-media-bucket".into(),
        access_key_id: "test-key".into(),
        secret_access_key: "test-secret".into(),
        local_retention_days: 7,
        local_cache_max_bytes: 1024,
        local_prune_enabled: false,
    })
    .await;
    (archive, state, task)
}

#[tokio::test]
async fn purge_removes_all_versions_and_markers_but_no_other_prefix() {
    let (archive, state, task) = start().await;
    let prefix = "media/v1/recordings/42/";
    state.versions.lock().await.extend([
        Version {
            key: format!("{prefix}a.ogg"),
            id: "v1".into(),
            marker: false,
        },
        Version {
            key: format!("{prefix}a.ogg"),
            id: "v2".into(),
            marker: false,
        },
        Version {
            key: format!("{prefix}a.ogg"),
            id: "marker".into(),
            marker: true,
        },
        Version {
            key: "media/v1/recordings/43/b.ogg".into(),
            id: "other".into(),
            marker: false,
        },
    ]);
    *state.fail_once.lock().await = true;
    assert_eq!(
        archive
            .purge_versions(prefix)
            .await
            .expect("purge versions"),
        3
    );
    assert_eq!(
        archive
            .purge_versions(prefix)
            .await
            .expect("idempotent purge"),
        0
    );
    assert_eq!(state.versions.lock().await.len(), 1);
    assert!(
        archive
            .purge_versions("media/v1/recordings/../")
            .await
            .is_err()
    );
    task.abort();
}
