use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Instant,
};

use actix_web::{
    HttpRequest, HttpResponse,
    http::{Method, StatusCode, header},
};
use bytes::Bytes;
use futures_util::{Stream, TryStreamExt};
use sakiot_storage::{ByteRange, RangeError, StorageError, StorageErrorKind};
use sqlx::{Pool, Postgres};
use tokio_util::io::ReaderStream;
use tracing::warn;

use super::{MediaArchive, metrics, repository};
use crate::errors::AppError;

#[derive(Clone, Copy, Debug)]
pub enum RemoteDisposition {
    Inline,
    Attachment,
}

pub(crate) async fn serve_remote(
    media: &MediaArchive,
    request: &HttpRequest,
    pool: &Pool<Postgres>,
    source: repository::SourceId,
    disposition: RemoteDisposition,
) -> Result<HttpResponse, AppError> {
    let archive = media.archive().ok_or(AppError::FileNotFound)?;
    let object = repository::available_object(pool, &source)
        .await?
        .ok_or(AppError::FileNotFound)?;
    let range = requested_range(request, object.bytes)?;
    let started = Instant::now();

    if request.method() == Method::HEAD {
        let head = archive
            .head(&object.object_key)
            .await
            .map_err(map_remote_error)?;
        let Some(head) = head else {
            repository::mark_remote_missing(pool, object.id, "B2 object missing during HEAD")
                .await?;
            return Err(AppError::FileNotFound);
        };
        if head.bytes != object.bytes || head.sha256.as_deref() != Some(&object.sha256) {
            metrics::remote_read_failure();
            repository::mark_verification_conflict(
                pool,
                object.id,
                "B2 HEAD metadata differs from the verified archive ledger",
            )
            .await?;
            return Err(AppError::ServiceUnavailable(
                "archived media failed metadata validation".to_owned(),
            ));
        }
        metrics::remote_read_success(0, started.elapsed());
        return Ok(
            response_builder(range, object.bytes, head.etag.as_deref(), disposition).finish(),
        );
    }

    let remote = match archive.get(&object.object_key, range).await {
        Ok(remote) => remote,
        Err(error) if error.kind() == StorageErrorKind::NotFound => {
            metrics::remote_read_failure();
            repository::mark_remote_missing(pool, object.id, &error.to_string()).await?;
            return Err(AppError::FileNotFound);
        }
        Err(error) => return Err(map_remote_error(error)),
    };
    let expected_bytes = range.map_or(object.bytes, ByteRange::content_length);
    if remote.head.bytes != expected_bytes
        || remote.head.sha256.as_deref() != Some(object.sha256.as_str())
    {
        metrics::remote_read_failure();
        repository::mark_verification_conflict(
            pool,
            object.id,
            "B2 GET metadata differs from the verified archive ledger",
        )
        .await?;
        return Err(AppError::ServiceUnavailable(
            "archived media failed metadata validation".to_owned(),
        ));
    }

    let stream =
        InstrumentedRemoteStream::new(ReaderStream::new(remote.body.into_async_read()), started)
            .map_err(actix_web::error::ErrorServiceUnavailable);
    Ok(response_builder(
        range,
        object.bytes,
        remote.head.etag.as_deref(),
        disposition,
    )
    .streaming(stream))
}

struct InstrumentedRemoteStream<S> {
    inner: S,
    started: Instant,
    bytes: u64,
    finished: bool,
}

impl<S> InstrumentedRemoteStream<S> {
    fn new(inner: S, started: Instant) -> Self {
        Self {
            inner,
            started,
            bytes: 0,
            finished: false,
        }
    }

    fn finish(&mut self) {
        if !self.finished {
            metrics::remote_read_success(self.bytes, self.started.elapsed());
            self.finished = true;
        }
    }
}

impl<S> Stream for InstrumentedRemoteStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(context) {
            Poll::Ready(Some(Ok(bytes))) => {
                self.bytes = self.bytes.saturating_add(bytes.len() as u64);
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(error))) => {
                metrics::remote_read_failure();
                self.finish();
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                self.finish();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> Drop for InstrumentedRemoteStream<S> {
    fn drop(&mut self) {
        self.finish();
    }
}

fn requested_range(request: &HttpRequest, total: u64) -> Result<Option<ByteRange>, AppError> {
    let Some(value) = request.headers().get(header::RANGE) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| AppError::InvalidParam("invalid Range header".to_owned()))?;
    match ByteRange::parse(value, total) {
        Ok(range) => Ok(Some(range)),
        Err(RangeError::Invalid | RangeError::Unsatisfiable) => {
            Err(AppError::RangeNotSatisfiable { total })
        }
    }
}

fn response_builder(
    range: Option<ByteRange>,
    total: u64,
    etag: Option<&str>,
    disposition: RemoteDisposition,
) -> actix_web::HttpResponseBuilder {
    let mut response = HttpResponse::build(if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    });
    response
        .insert_header((header::ACCEPT_RANGES, "bytes"))
        .insert_header((header::CONTENT_TYPE, "audio/ogg"))
        .insert_header((
            header::CONTENT_DISPOSITION,
            match disposition {
                RemoteDisposition::Inline => "inline",
                RemoteDisposition::Attachment => "attachment",
            },
        ))
        .insert_header((
            header::CONTENT_LENGTH,
            range.map_or(total, ByteRange::content_length).to_string(),
        ));
    if let Some(range) = range {
        response.insert_header((header::CONTENT_RANGE, range.content_range()));
    }
    if let Some(etag) = etag.and_then(|value| header::HeaderValue::from_str(value).ok()) {
        response.insert_header((header::ETAG, etag));
    }
    response
}

fn map_remote_error(error: StorageError) -> AppError {
    metrics::remote_read_failure();
    match error.kind() {
        StorageErrorKind::NotFound => AppError::FileNotFound,
        _ => {
            warn!(?error, "B2 remote media read failed");
            AppError::ServiceUnavailable("media archive unavailable".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{ResponseError, http::header, test::TestRequest};

    use super::*;

    fn response_header(response: &HttpResponse, name: header::HeaderName) -> &str {
        response
            .headers()
            .get(name)
            .expect("response header")
            .to_str()
            .expect("ASCII response header")
    }

    #[test]
    fn range_response_preserves_streaming_headers() {
        let range = ByteRange::parse("bytes=2-5", 10).expect("valid range");
        let response = response_builder(
            Some(range),
            10,
            Some("\"etag-value\""),
            RemoteDisposition::Attachment,
        )
        .finish();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response_header(&response, header::CONTENT_LENGTH), "4");
        assert_eq!(
            response_header(&response, header::CONTENT_RANGE),
            "bytes 2-5/10"
        );
        assert_eq!(response_header(&response, header::ACCEPT_RANGES), "bytes");
        assert_eq!(
            response_header(&response, header::CONTENT_TYPE),
            "audio/ogg"
        );
        assert_eq!(
            response_header(&response, header::CONTENT_DISPOSITION),
            "attachment"
        );
        assert_eq!(response_header(&response, header::ETAG), "\"etag-value\"");
    }

    #[test]
    fn unsatisfiable_range_returns_416_with_total() {
        let request = TestRequest::get()
            .insert_header((header::RANGE, "bytes=10-"))
            .to_http_request();
        let error = requested_range(&request, 10).expect_err("range should fail");
        let response = error.error_response();
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response_header(&response, header::CONTENT_RANGE),
            "bytes */10"
        );
    }
}
