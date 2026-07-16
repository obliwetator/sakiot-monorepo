#![allow(clippy::expect_used, clippy::panic)]

use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::State,
    http::{HeaderMap, Method, Request, Response, StatusCode, header},
};
use futures_util::{StreamExt, stream};
use sakiot_storage::{
    Archive, ArchiveConfig, ByteRange, MULTIPART_THRESHOLD_BYTES, StorageErrorKind, hash_file,
};
use tokio::{io::AsyncWriteExt, sync::Mutex, task::JoinHandle};

const BUCKET: &str = "test-media-bucket";

#[derive(Clone, Debug)]
struct Object {
    body: Bytes,
    sha256: String,
    etag: String,
}

#[derive(Debug, Default)]
struct Multipart {
    key: String,
    sha256: String,
    parts: BTreeMap<i32, Bytes>,
}

#[derive(Debug, Default)]
struct FakeState {
    objects: Mutex<HashMap<String, Object>>,
    multipart: Mutex<HashMap<String, Multipart>>,
    multipart_created: AtomicUsize,
    aborts: AtomicUsize,
    aws_chunked_requests: AtomicUsize,
    fail_head: AtomicBool,
    fail_get: AtomicBool,
    fail_part: AtomicBool,
    slow_get: AtomicBool,
    release_get: Arc<tokio::sync::Notify>,
}

struct FakeS3 {
    address: SocketAddr,
    state: Arc<FakeState>,
    task: JoinHandle<()>,
}

impl FakeS3 {
    async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let state = Arc::new(FakeState::default());
        let app = Router::new()
            .fallback(handle_request)
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            address,
            state,
            task,
        })
    }

    async fn archive(&self) -> Archive {
        Archive::new(&ArchiveConfig {
            endpoint: format!("http://{}", self.address),
            region: "test-region".to_owned(),
            bucket: BUCKET.to_owned(),
            access_key_id: "test-key-id".to_owned(),
            secret_access_key: "test-secret".to_owned(),
            local_retention_days: 7,
            local_cache_max_bytes: 1024,
            local_prune_enabled: false,
        })
        .await
    }
}

impl Drop for FakeS3 {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_request(
    State(state): State<Arc<FakeState>>,
    request: Request<Body>,
) -> Response<Body> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let headers = request.headers().clone();
    let Some(key) = path.strip_prefix(&format!("/{BUCKET}/")) else {
        return error_response(StatusCode::NOT_FOUND, "NoSuchBucket");
    };
    let parameters: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();

    if method == Method::POST && parameters.contains_key("uploads") {
        return create_multipart(&state, key, &headers).await;
    }
    if method == Method::PUT && parameters.contains_key("partNumber") {
        return upload_part(state, key, parameters, request).await;
    }
    if method == Method::POST && parameters.contains_key("uploadId") {
        return complete_multipart(&state, parameters).await;
    }
    if method == Method::DELETE && parameters.contains_key("uploadId") {
        state.aborts.fetch_add(1, Ordering::Relaxed);
        state.multipart.lock().await.remove(
            parameters
                .get("uploadId")
                .map(String::as_str)
                .unwrap_or_default(),
        );
        return empty_response(StatusCode::NO_CONTENT);
    }
    match method {
        Method::PUT => put_object(state, key, headers, request).await,
        Method::HEAD => head_object(&state, key).await,
        Method::GET => get_object(&state, key, &headers).await,
        _ => error_response(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed"),
    }
}

async fn put_object(
    state: Arc<FakeState>,
    key: &str,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response<Body> {
    let Ok(body) = to_bytes(request.into_body(), usize::MAX).await else {
        return error_response(StatusCode::BAD_REQUEST, "InvalidBody");
    };
    if is_aws_chunked(&headers) {
        state.aws_chunked_requests.fetch_add(1, Ordering::Relaxed);
    }
    let body = decode_aws_chunked(&headers, body);
    let sha256 = metadata_sha256(&headers);
    let etag = format!("\"single-{}\"", body.len());
    state.objects.lock().await.insert(
        key.to_owned(),
        Object {
            body,
            sha256,
            etag: etag.clone(),
        },
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::ETAG, etag)
        .body(Body::empty())
        .expect("valid fake response")
}

async fn head_object(state: &FakeState, key: &str) -> Response<Body> {
    if state.fail_head.load(Ordering::Relaxed) {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "ServiceUnavailable");
    }
    let objects = state.objects.lock().await;
    let Some(object) = objects.get(key) else {
        return empty_response(StatusCode::NOT_FOUND);
    };
    object_response_headers(
        StatusCode::OK,
        object,
        object.body.len(),
        None,
        Body::empty(),
    )
}

async fn get_object(state: &FakeState, key: &str, headers: &HeaderMap) -> Response<Body> {
    if state.fail_get.load(Ordering::Relaxed) {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "ServiceUnavailable");
    }
    let objects = state.objects.lock().await;
    let Some(object) = objects.get(key) else {
        return error_response(StatusCode::NOT_FOUND, "NoSuchKey");
    };
    let Some(range) = headers.get(header::RANGE) else {
        if state.slow_get.load(Ordering::Relaxed) {
            let midpoint = object.body.len().div_ceil(2);
            let first = object.body.slice(..midpoint);
            let second = object.body.slice(midpoint..);
            let release = state.release_get.clone();
            let body = Body::from_stream(
                stream::once(async move { Ok::<_, std::io::Error>(first) }).chain(stream::once(
                    async move {
                        release.notified().await;
                        Ok::<_, std::io::Error>(second)
                    },
                )),
            );
            return object_response_headers(StatusCode::OK, object, object.body.len(), None, body);
        }
        return object_response_headers(
            StatusCode::OK,
            object,
            object.body.len(),
            None,
            Body::from(object.body.clone()),
        );
    };
    let Ok(range) = range.to_str() else {
        return range_error(object.body.len());
    };
    let Ok(range) = ByteRange::parse(range, object.body.len() as u64) else {
        return range_error(object.body.len());
    };
    let body = object.body.slice(range.start as usize..=range.end as usize);
    object_response_headers(
        StatusCode::PARTIAL_CONTENT,
        object,
        body.len(),
        Some(range.content_range()),
        Body::from(body),
    )
}

async fn create_multipart(state: &FakeState, key: &str, headers: &HeaderMap) -> Response<Body> {
    let sequence = state.multipart_created.fetch_add(1, Ordering::Relaxed) + 1;
    let upload_id = format!("upload-{sequence}");
    state.multipart.lock().await.insert(
        upload_id.clone(),
        Multipart {
            key: key.to_owned(),
            sha256: metadata_sha256(headers),
            parts: BTreeMap::new(),
        },
    );
    xml_response(
        StatusCode::OK,
        format!(
            "<InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>{BUCKET}</Bucket><Key>{key}</Key><UploadId>{upload_id}</UploadId></InitiateMultipartUploadResult>"
        ),
    )
}

async fn upload_part(
    state: Arc<FakeState>,
    _key: &str,
    parameters: HashMap<String, String>,
    request: Request<Body>,
) -> Response<Body> {
    if state.fail_part.load(Ordering::Relaxed) {
        return error_response(StatusCode::BAD_REQUEST, "InvalidPart");
    }
    let Some(upload_id) = parameters.get("uploadId") else {
        return error_response(StatusCode::BAD_REQUEST, "InvalidRequest");
    };
    let Some(part_number) = parameters
        .get("partNumber")
        .and_then(|part| part.parse::<i32>().ok())
    else {
        return error_response(StatusCode::BAD_REQUEST, "InvalidRequest");
    };
    let headers = request.headers().clone();
    let Ok(body) = to_bytes(request.into_body(), usize::MAX).await else {
        return error_response(StatusCode::BAD_REQUEST, "InvalidBody");
    };
    if is_aws_chunked(&headers) {
        state.aws_chunked_requests.fetch_add(1, Ordering::Relaxed);
    }
    let body = decode_aws_chunked(&headers, body);
    let mut uploads = state.multipart.lock().await;
    let Some(upload) = uploads.get_mut(upload_id) else {
        return error_response(StatusCode::NOT_FOUND, "NoSuchUpload");
    };
    upload.parts.insert(part_number, body);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::ETAG, format!("\"part-{part_number}\""))
        .body(Body::empty())
        .expect("valid fake response")
}

async fn complete_multipart(
    state: &FakeState,
    parameters: HashMap<String, String>,
) -> Response<Body> {
    let Some(upload_id) = parameters.get("uploadId") else {
        return error_response(StatusCode::BAD_REQUEST, "InvalidRequest");
    };
    let Some(upload) = state.multipart.lock().await.remove(upload_id) else {
        return error_response(StatusCode::NOT_FOUND, "NoSuchUpload");
    };
    let mut body = Vec::new();
    for bytes in upload.parts.values() {
        body.extend_from_slice(bytes);
    }
    let etag = format!("\"multipart-{}\"", upload.parts.len());
    state.objects.lock().await.insert(
        upload.key.clone(),
        Object {
            body: Bytes::from(body),
            sha256: upload.sha256,
            etag: etag.clone(),
        },
    );
    xml_response(
        StatusCode::OK,
        format!(
            "<CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Location>http://localhost/{BUCKET}/{}</Location><Bucket>{BUCKET}</Bucket><Key>{}</Key><ETag>{etag}</ETag></CompleteMultipartUploadResult>",
            upload.key, upload.key
        ),
    )
}

fn metadata_sha256(headers: &HeaderMap) -> String {
    headers
        .get("x-amz-meta-sha256")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

fn decode_aws_chunked(headers: &HeaderMap, body: Bytes) -> Bytes {
    if !is_aws_chunked(headers) {
        return body;
    }

    let mut cursor = 0usize;
    let mut decoded = Vec::new();
    while cursor < body.len() {
        let Some(header_end) = body[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|position| cursor + position)
        else {
            return body;
        };
        let chunk_header = &body[cursor..header_end];
        let Some(size) = chunk_header
            .split(|byte| *byte == b';')
            .next()
            .and_then(|size| std::str::from_utf8(size).ok())
            .and_then(|size| usize::from_str_radix(size, 16).ok())
        else {
            return body;
        };
        cursor = header_end + 2;
        if size == 0 {
            return Bytes::from(decoded);
        }
        let Some(chunk_end) = cursor.checked_add(size) else {
            return body;
        };
        if chunk_end + 2 > body.len() || &body[chunk_end..chunk_end + 2] != b"\r\n" {
            return body;
        }
        decoded.extend_from_slice(&body[cursor..chunk_end]);
        cursor = chunk_end + 2;
    }
    Bytes::from(decoded)
}

fn is_aws_chunked(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|encoding| encoding.trim() == "aws-chunked")
        })
}

fn object_response_headers(
    status: StatusCode,
    object: &Object,
    content_length: usize,
    content_range: Option<String>,
    body: Body,
) -> Response<Body> {
    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_LENGTH, content_length)
        .header(header::CONTENT_TYPE, "audio/ogg")
        .header(header::ETAG, &object.etag)
        .header("x-amz-meta-sha256", &object.sha256);
    if let Some(content_range) = content_range {
        response = response.header(header::CONTENT_RANGE, content_range);
    }
    response.body(body).expect("valid fake response")
}

fn range_error(total: usize) -> Response<Body> {
    Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(header::CONTENT_RANGE, format!("bytes */{total}"))
        .body(Body::empty())
        .expect("valid fake response")
}

fn empty_response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .expect("valid fake response")
}

fn error_response(status: StatusCode, code: &str) -> Response<Body> {
    xml_response(
        status,
        format!("<Error><Code>{code}</Code><Message>{code}</Message></Error>"),
    )
}

fn xml_response(status: StatusCode, xml: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(xml))
        .expect("valid fake response")
}

async fn write_file(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut file = tokio::fs::File::create(path).await?;
    file.write_all(bytes).await?;
    file.flush().await
}

#[tokio::test]
async fn single_upload_head_verify_range_and_idempotent_retry()
-> Result<(), Box<dyn std::error::Error>> {
    let fake = FakeS3::start().await?;
    let archive = fake.archive().await;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("small.ogg");
    write_file(&path, b"0123456789").await?;
    let digest = hash_file(&path).await?;
    let key = "media/v1/recordings/1/digest.ogg";

    archive.upload_file(key, &path, &digest).await?;
    archive.upload_file(key, &path, &digest).await?;
    let head = archive.head(key).await?.expect("uploaded object");
    assert_eq!(head.bytes, 10);
    assert_eq!(head.sha256.as_deref(), Some(digest.sha256.as_str()));
    archive.verify_object(key, &digest).await?;
    let restored = directory.path().join("restored/small.ogg");
    archive.download_verified(key, &restored, &digest).await?;
    assert_eq!(tokio::fs::read(&restored).await?, b"0123456789");

    let range = ByteRange::parse("bytes=2-5", digest.bytes)?;
    let body = archive.get(key, Some(range)).await?.body.collect().await?;
    assert_eq!(body.into_bytes(), Bytes::from_static(b"2345"));
    assert_eq!(fake.state.multipart_created.load(Ordering::Relaxed), 0);
    assert_eq!(fake.state.aws_chunked_requests.load(Ordering::Relaxed), 0);
    Ok(())
}

#[tokio::test]
async fn multipart_upload_and_failed_part_abort() -> Result<(), Box<dyn std::error::Error>> {
    let fake = FakeS3::start().await?;
    let archive = fake.archive().await;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("large.ogg");
    let file = tokio::fs::File::create(&path).await?;
    file.set_len(MULTIPART_THRESHOLD_BYTES).await?;
    let digest = hash_file(&path).await?;

    archive
        .upload_file("media/v1/recordings/2/large.ogg", &path, &digest)
        .await?;
    assert_eq!(fake.state.multipart_created.load(Ordering::Relaxed), 1);
    archive
        .verify_object("media/v1/recordings/2/large.ogg", &digest)
        .await?;

    fake.state.fail_part.store(true, Ordering::Relaxed);
    let failure = archive
        .upload_file("media/v1/recordings/3/abort.ogg", &path, &digest)
        .await;
    assert!(failure.is_err());
    assert_eq!(fake.state.aborts.load(Ordering::Relaxed), 1);
    assert_eq!(fake.state.aws_chunked_requests.load(Ordering::Relaxed), 0);
    Ok(())
}

#[tokio::test]
async fn checksum_mismatch_and_head_get_failures_are_classified()
-> Result<(), Box<dyn std::error::Error>> {
    let fake = FakeS3::start().await?;
    let archive = fake.archive().await;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("corrupt.ogg");
    write_file(&path, b"correct").await?;
    let digest = hash_file(&path).await?;
    let key = "media/v1/clips/test/corrupt.ogg";
    archive.upload_file(key, &path, &digest).await?;

    fake.state
        .objects
        .lock()
        .await
        .get_mut(key)
        .expect("uploaded object")
        .body = Bytes::from_static(b"corrupt");
    let error = archive
        .verify_object(key, &digest)
        .await
        .expect_err("mismatch");
    assert_eq!(error.kind(), StorageErrorKind::Integrity);
    let target = directory.path().join("must-not-appear.ogg");
    let error = archive
        .download_verified(key, &target, &digest)
        .await
        .expect_err("corrupt download");
    assert_eq!(error.kind(), StorageErrorKind::Integrity);
    assert!(!target.exists());

    fake.state.fail_head.store(true, Ordering::Relaxed);
    let error = archive.head(key).await.expect_err("HEAD failure");
    assert_eq!(error.kind(), StorageErrorKind::Unavailable);
    fake.state.fail_head.store(false, Ordering::Relaxed);
    fake.state.fail_get.store(true, Ordering::Relaxed);
    let Err(error) = archive.get(key, None).await else {
        panic!("GET should fail");
    };
    assert_eq!(error.kind(), StorageErrorKind::Unavailable);
    Ok(())
}

#[tokio::test]
async fn get_streams_before_complete_body_is_available() -> Result<(), Box<dyn std::error::Error>> {
    use tokio::io::AsyncReadExt;

    let fake = FakeS3::start().await?;
    let archive = fake.archive().await;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("stream.ogg");
    write_file(&path, b"stream-with-backpressure").await?;
    let digest = hash_file(&path).await?;
    let key = "media/v1/recordings/4/stream.ogg";
    archive.upload_file(key, &path, &digest).await?;
    fake.state.slow_get.store(true, Ordering::Relaxed);

    let remote =
        tokio::time::timeout(std::time::Duration::from_secs(1), archive.get(key, None)).await??;
    let mut reader = remote.body.into_async_read();
    let mut first = vec![0u8; 4];
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        reader.read_exact(&mut first),
    )
    .await??;
    assert_eq!(&first, b"stre");
    fake.state.release_get.notify_waiters();
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).await?;
    assert_eq!([first, rest].concat(), b"stream-with-backpressure");
    Ok(())
}
