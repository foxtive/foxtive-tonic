//! Metrics and observability utilities.
//!
//! [`crate::metrics::MetricsLayer`] wraps the response body so a request is
//! accounted for exactly when the response is finished with, including when a
//! disconnect abandons it, and reads the gRPC status from the response rather
//! than the error type, because tonic reports failure as `Ok(Response)`.

use http_body::{Body as BodyTrait, Frame};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Instant;
use tonic::body::Body;
use tonic::codegen::http::{HeaderMap, Request, Response};
use tonic::codegen::{Bytes, StdError};
use tower::Layer;

/// Upper bounds (in milliseconds) of the latency histogram, ascending.
///
/// The last entry is the `+Inf` bucket, spelled [`u64::MAX`].
pub const LATENCY_BUCKETS_MS: [u64; 12] =
    [1, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, u64::MAX];

/// A point-in-time view of a [`Metrics`] collector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsSnapshot {
    /// Requests that reached the service.
    pub total_requests: u64,
    /// Requests that completed with a non-`Ok` gRPC status.
    pub total_errors: u64,
    /// Requests currently in flight.
    pub active_requests: u64,
    /// Sum of request durations in milliseconds; divide by `total_requests` for a mean.
    pub total_duration_ms: u64,
    /// Cumulative `(upper bound ms, count)` pairs, in [`LATENCY_BUCKETS_MS`] order.
    pub latency_buckets: Vec<(u64, u64)>,
}

/// Metrics collector for gRPC requests.
///
/// Cheap to clone (clones share one counter set) and safe to read from any task.
/// Counters use `Ordering::Relaxed` since they are independent.
#[derive(Clone, Default)]
pub struct Metrics {
    inner: Arc<MetricsInner>,
}

#[derive(Default)]
struct MetricsInner {
    total_requests: AtomicU64,
    total_errors: AtomicU64,
    active_requests: AtomicU64,
    total_duration_ms: AtomicU64,
    latency_buckets: [AtomicU64; LATENCY_BUCKETS_MS.len()],
}

impl Metrics {
    /// Create a new metrics collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get total request count.
    pub fn total_requests(&self) -> u64 {
        self.inner.total_requests.load(Ordering::Relaxed)
    }

    /// Get total error count.
    pub fn total_errors(&self) -> u64 {
        self.inner.total_errors.load(Ordering::Relaxed)
    }

    /// Get active request count.
    ///
    /// Released when the response is finished with, including on a client
    /// disconnect, a timeout, or a load-shed rejection.
    pub fn active_requests(&self) -> u64 {
        self.inner.active_requests.load(Ordering::Relaxed)
    }

    /// Sum of all completed request durations, in milliseconds.
    pub fn total_duration_ms(&self) -> u64 {
        self.inner.total_duration_ms.load(Ordering::Relaxed)
    }

    /// Count of completed requests whose duration was at most `bound_ms`.
    ///
    /// Buckets are cumulative, matching how histogram quantiles are computed.
    /// `bound_ms` is looked up in [`LATENCY_BUCKETS_MS`]; a bound that is not in
    /// the table returns the `+Inf` count.
    pub fn requests_under(&self, bound_ms: u64) -> u64 {
        let index = LATENCY_BUCKETS_MS
            .iter()
            .position(|b| *b == bound_ms)
            .unwrap_or(LATENCY_BUCKETS_MS.len() - 1);
        let mut count = 0;
        for bucket in &self.inner.latency_buckets[..=index] {
            count += bucket.load(Ordering::Relaxed);
        }
        count
    }

    /// Record an application-level error that never surfaced as a gRPC status
    /// (e.g. one swallowed by a custom interceptor).
    pub fn record_error(&self) {
        self.inner.total_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot every counter in one call.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            total_requests: self.total_requests(),
            total_errors: self.total_errors(),
            active_requests: self.active_requests(),
            total_duration_ms: self.total_duration_ms(),
            latency_buckets: LATENCY_BUCKETS_MS
                .iter()
                .copied()
                .zip(self.inner.latency_buckets.iter())
                .map(|(bound, count)| (bound, count.load(Ordering::Relaxed)))
                .collect(),
        }
    }

    /// Begin accounting for one request.
    fn begin(&self) -> ActiveGuard {
        self.inner.total_requests.fetch_add(1, Ordering::Relaxed);
        self.inner.active_requests.fetch_add(1, Ordering::Relaxed);
        ActiveGuard {
            metrics: self.clone(),
            started: Instant::now(),
            errored: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// RAII guard that finalizes one request's accounting exactly once.
///
/// Held by [`MetricsBody`], so accounting closes when the response body drops,
/// including on disconnect, timeout, or load-shed.
struct ActiveGuard {
    metrics: Metrics,
    started: Instant,
    errored: Arc<AtomicBool>,
}

impl ActiveGuard {
    /// Flag this request as failed, so it is counted on release.
    fn mark_error(&self) {
        self.errored.store(true, Ordering::Relaxed);
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let duration = self.started.elapsed();
        let ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);

        self.metrics
            .inner
            .active_requests
            .fetch_sub(1, Ordering::Relaxed);
        self.metrics
            .inner
            .total_duration_ms
            .fetch_add(ms, Ordering::Relaxed);
        self.metrics.inner.latency_buckets[bucket_index_for(ms)].fetch_add(1, Ordering::Relaxed);
        if self.errored.load(Ordering::Relaxed) {
            self.metrics.record_error();
        }

        tracing::debug!(duration_ms = ms, "Request completed");
    }
}

/// `true` if `headers` carry a gRPC status other than `Ok`.
///
/// Covers trailers-only responses: a unary handler's `Status` arrives in the
/// response head, before any body.
fn headers_report_error(headers: &HeaderMap) -> bool {
    headers
        .get(tonic::Status::GRPC_STATUS)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|status| status != "0")
}

/// Response body wrapper that releases the [`ActiveGuard`] when the response is
/// finished with, and reads a streaming response's status from its trailers.
struct MetricsBody<B> {
    inner: B,
    guard: ActiveGuard,
}

impl<B> MetricsBody<B> {
    /// `true` if a trailer block reports a non-`Ok` gRPC status.
    fn note_trailers(&self, trailers: &HeaderMap) {
        if headers_report_error(trailers) {
            self.guard.mark_error();
        }
    }
}

impl<B> BodyTrait for MetricsBody<B>
where
    B: BodyTrait<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Into<StdError>,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let poll = Pin::new(&mut this.inner).poll_frame(cx);
        // http-body 1.x delivers a streaming response's status as a trailer *frame*
        // from poll_frame (no separate poll_trailers), so inspect it here.
        if let Poll::Ready(Some(Ok(frame))) = &poll
            && let Some(trailers) = frame.trailers_ref()
        {
            this.note_trailers(trailers);
        }
        poll
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
}

/// Layer for adding metrics collection to services.
///
/// Apply it with [`Server::layer`](crate::server::Server::layer).
#[derive(Clone)]
pub struct MetricsLayer {
    metrics: Metrics,
}

impl MetricsLayer {
    /// Create a new metrics layer.
    pub fn new(metrics: Metrics) -> Self {
        Self { metrics }
    }

    /// The collector this layer writes to.
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }
}

impl<S> Layer<S> for MetricsLayer
where
    S: tower::Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService {
            inner,
            metrics: self.metrics.clone(),
        }
    }
}

/// Service wrapper that collects metrics.
pub struct MetricsService<S> {
    inner: S,
    metrics: Metrics,
}

impl<S> Clone for MetricsService<S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

impl<S> tower::Service<Request<Body>> for MetricsService<S>
where
    S: tower::Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future =
        Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // begin() before the inner future so a request rejected in poll_ready still
        // counts; the guard moves into the response body, or drops here.
        let guard = self.metrics.begin();
        let future = self.inner.call(req);

        Box::pin(async move {
            match future.await {
                Ok(response) => {
                    if headers_report_error(response.headers()) {
                        guard.mark_error();
                    }
                    Ok(response.map(|body| Body::new(MetricsBody { inner: body, guard })))
                }
                Err(err) => {
                    // No body will ever be streamed, so account for it now.
                    guard.mark_error();
                    drop(guard);
                    Err(err)
                }
            }
        })
    }
}

/// Histogram bucket index for a duration in milliseconds.
fn bucket_index_for(ms: u64) -> usize {
    LATENCY_BUCKETS_MS
        .iter()
        .position(|bound| ms <= *bound)
        .unwrap_or(LATENCY_BUCKETS_MS.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tower::Service as _;

    async fn say_ok(_req: Request<Body>) -> Result<Response<Body>, tonic::Status> {
        Ok(Response::new(Body::empty()))
    }

    async fn say_never(_req: Request<Body>) -> Result<Response<Body>, tonic::Status> {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Ok(Response::new(Body::empty()))
    }

    /// A `Status` returned by a unary handler arrives as `Ok(Response)` with the
    /// status in the header map, never as `Err`.
    async fn say_not_found_in_headers(
        _req: Request<Body>,
    ) -> Result<Response<Body>, tonic::Status> {
        let mut response = Response::new(Body::empty());
        response
            .headers_mut()
            .insert(tonic::Status::GRPC_STATUS, "5".parse().unwrap());
        Ok(response)
    }

    #[tokio::test]
    async fn request_is_counted_at_call_time_and_released_when_the_future_is_dropped() {
        // An abandoned future (client disconnect, timeout, load shedding) must
        // still release its active-request slot.
        let metrics = Metrics::new();
        let mut svc = MetricsLayer::new(metrics.clone()).layer(tower::service_fn(say_never));

        assert_eq!(metrics.active_requests(), 0);
        let fut = svc.call(Request::new(Body::empty()));
        assert_eq!(metrics.total_requests(), 1, "counted without being polled");
        assert_eq!(metrics.active_requests(), 1);

        drop(fut);
        assert_eq!(metrics.active_requests(), 0, "guard released by Drop");
    }

    #[tokio::test]
    async fn completed_request_is_fully_accounted() {
        let metrics = Metrics::new();
        let mut svc = MetricsLayer::new(metrics.clone()).layer(tower::service_fn(say_ok));

        let response = svc.call(Request::new(Body::empty())).await.unwrap();
        assert_eq!(metrics.total_requests(), 1);
        assert_eq!(metrics.total_errors(), 0);

        drop(response);
        assert_eq!(metrics.active_requests(), 0);
        assert!(metrics.total_duration_ms() < 60_000);
        assert_eq!(
            metrics
                .snapshot()
                .latency_buckets
                .iter()
                .map(|(_, c)| c)
                .sum::<u64>(),
            1,
            "every completed request lands in exactly one bucket"
        );
    }

    #[tokio::test]
    async fn trailer_only_status_in_headers_counts_as_an_error() {
        let metrics = Metrics::new();
        let mut svc =
            MetricsLayer::new(metrics.clone()).layer(tower::service_fn(say_not_found_in_headers));

        let response = svc.call(Request::new(Body::empty())).await.unwrap();
        assert!(headers_report_error(response.headers()));
        drop(response);

        assert_eq!(
            metrics.total_errors(),
            1,
            "a gRPC failure is an Ok(Response)"
        );
        assert_eq!(metrics.total_requests(), 1);
    }

    #[tokio::test]
    async fn service_level_error_counts_as_an_error() {
        let metrics = Metrics::new();
        let mut svc = MetricsLayer::new(metrics.clone()).layer(tower::service_fn(
            |_req: Request<Body>| async { Err(tonic::Status::unavailable("down")) },
        ));

        let err = svc.call(Request::new(Body::empty())).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unavailable);
        assert_eq!(metrics.active_requests(), 0);
        assert_eq!(metrics.total_errors(), 1);
    }

    #[test]
    fn latency_buckets_are_ordered_and_catch_everything() {
        assert_eq!(LATENCY_BUCKETS_MS.first(), Some(&1));
        // +Inf must be last so no request can fall out of the histogram.
        assert_eq!(LATENCY_BUCKETS_MS.last(), Some(&u64::MAX));
        assert!(LATENCY_BUCKETS_MS.windows(2).all(|w| w[0] < w[1]));

        assert_eq!(bucket_index_for(0), 0);
        assert_eq!(bucket_index_for(1), 0);
        assert_eq!(bucket_index_for(7), 2);
        assert_eq!(bucket_index_for(5_001), LATENCY_BUCKETS_MS.len() - 1);
        assert_eq!(bucket_index_for(u64::MAX), LATENCY_BUCKETS_MS.len() - 1);
    }

    #[tokio::test]
    async fn snapshot_aggregates_every_counter() {
        let metrics = Metrics::new();
        metrics.record_error();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_errors, 1);
        assert_eq!(snapshot.total_requests, 0);
        assert_eq!(snapshot.active_requests, 0);
        assert_eq!(snapshot.latency_buckets.len(), LATENCY_BUCKETS_MS.len());
        assert_eq!(snapshot.latency_buckets.last().unwrap().0, u64::MAX);
    }

    #[tokio::test]
    async fn requests_under_is_cumulative() {
        let metrics = Metrics::new();
        let mut svc = MetricsLayer::new(metrics.clone()).layer(tower::service_fn(say_ok));
        let response = svc.call(Request::new(Body::empty())).await.unwrap();
        drop(response);

        // A completed no-op request lands in the fastest bucket, so the 1ms bound
        // already covers it, and `+Inf` always holds every request.
        assert_eq!(metrics.requests_under(1), metrics.total_requests());
        assert_eq!(metrics.requests_under(u64::MAX), 1);
    }
}
