//! Observability over a real socket: what the metrics layer counts, which order
//! the layers run in, and whether the health service is actually served.

use foxtive_tonic::health::HealthState;
use foxtive_tonic::metrics::{Metrics, MetricsLayer};
use foxtive_tonic::server::BoxedService;
use foxtive_tonic_examples::proto::demo::demo_server::DemoServer;
use foxtive_tonic_examples::proto::demo::{PingRequest, SleepRequest};
use foxtive_tonic_examples::services::DemoService;
use foxtive_tonic_examples::test_support::{
    TestResult, app, channel, demo_client, eventually, server,
};
use std::sync::{Arc, Mutex};
use tonic::Code;
use tonic::body::Body;
use tonic::codegen::http::Request;
use tonic::server::NamedService;
use tonic::service::Routes;
use tonic::transport::Channel;
use tonic_health::pb::HealthCheckRequest;
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;
use tower::Layer;
use tower::Service;
use tower::util::BoxCloneService;

/// A layer that records its tag on the way in, so a test can read the order back.
#[derive(Clone)]
struct Marker {
    tag: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Layer<BoxedService> for Marker {
    type Service = BoxedService;

    fn layer(&self, inner: BoxedService) -> BoxedService {
        let tag = self.tag;
        let log = self.log.clone();

        BoxCloneService::new(tower::service_fn(move |req: Request<Body>| {
            let log = log.clone();
            let mut inner = inner.clone();
            async move {
                log.lock().unwrap().push(tag);
                inner.call(req).await
            }
        }))
    }
}

/// One `Check` call, to keep the assertions below readable.
///
/// `status` is the raw `i32` prost gives an `enumeration` field, so the helpers
/// compare against `ServingStatus::* as i32` (pb enum, wire numbers) rather than
/// `tonic_health::ServingStatus`.
async fn check(client: &mut HealthClient<Channel>, service: &str) -> TestResult<i32> {
    let response = client
        .check(HealthCheckRequest {
            service: service.to_owned(),
        })
        .await?;
    Ok(response.into_inner().status)
}

/// A handler's `Status::not_found` reaches the client as `NotFound` *and*
/// reaches the metrics collector as one error.
///
/// Tonic reports a unary failure as an `Ok(Response)` whose headers carry
/// `grpc-status`, so the layer has to look there rather than at its own error type.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_call_reports_not_found_and_counts_one_error() -> TestResult {
    let metrics = Metrics::new();
    let handle = server(&app("metrics-error").await)
        .layer(MetricsLayer::new(metrics.clone()))
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;

    let mut client = demo_client(handle.local_addr()).await;
    let status = client
        .ping(PingRequest { fail: true })
        .await
        .expect_err("the handler reports NotFound");

    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(metrics.total_requests(), 1, "the call reached the service");

    // Accounting closes when the response body is released, which happens on a
    // different task from the one that delivered the status to this client.
    eventually("the failed call to be counted as an error", || {
        metrics.total_errors() == 1
    })
    .await;
    eventually("the failed call to stop being active", || {
        metrics.active_requests() == 0
    })
    .await;

    // A successful call on the same server must not be counted as an error.
    client.ping(PingRequest { fail: false }).await?;
    eventually("the successful call to be recorded", || {
        metrics.total_requests() == 2
    })
    .await;
    assert_eq!(metrics.total_errors(), 1, "only the failed call counts");

    handle.shutdown();
    Ok(handle.stopped().await?)
}

/// `.layer(a).layer(b)` runs `a` before `b`, with an unrelated layer in between
/// so the order cannot be an accident of how they were written.
#[tokio::test(flavor = "multi_thread")]
async fn layers_run_in_the_order_they_are_added() -> TestResult {
    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    let handle = server(&app("layer-order").await)
        .layer(Marker {
            tag: "first",
            log: log.clone(),
        })
        .layer(MetricsLayer::new(Metrics::new()))
        .layer(Marker {
            tag: "second",
            log: log.clone(),
        })
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;

    let mut client = demo_client(handle.local_addr()).await;
    client.ping(PingRequest { fail: false }).await?;
    eventually("the request to reach both markers", || {
        log.lock().unwrap().len() == 2
    })
    .await;

    assert_eq!(*log.lock().unwrap(), vec!["first", "second"]);

    handle.shutdown();
    Ok(handle.stopped().await?)
}

/// An abandoned request releases its slot, so a long-running server does not
/// drift towards a permanently full concurrency count.
#[tokio::test(flavor = "multi_thread")]
async fn abandoned_requests_do_not_leak_active_slots() -> TestResult {
    let metrics = Metrics::new();
    let handle = server(&app("active-leak").await)
        .layer(MetricsLayer::new(metrics.clone()))
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;

    let client = demo_client(handle.local_addr()).await;

    // Fifteen calls that answer, five that the caller walks away from.
    let mut finishing = Vec::new();
    let mut abandoned = Vec::new();
    for index in 0..20 {
        let mut client = client.clone();
        let task = tokio::spawn(async move {
            client
                .sleep(SleepRequest {
                    millis: if index < 15 { 50 } else { 30_000 },
                })
                .await
        });
        if index < 15 {
            finishing.push(task);
        } else {
            abandoned.push(task);
        }
    }

    for task in finishing {
        task.await??;
    }
    assert_eq!(
        metrics.total_requests(),
        20,
        "every call reached the service"
    );
    eventually("the abandoned calls to be in flight", || {
        metrics.active_requests() == 5
    })
    .await;

    for task in abandoned {
        task.abort();
    }
    eventually("the abandoned calls to release their slot", || {
        metrics.active_requests() == 0
    })
    .await;
    assert_eq!(
        metrics.total_requests(),
        20,
        "walking away from a call must not be counted as another one"
    );

    handle.shutdown();
    Ok(handle.stopped().await?)
}

/// `Server::health` serves the standard `grpc.health.v1.Health` service, and a
/// status flip is visible to a client that never met the `HealthState`.
#[tokio::test(flavor = "multi_thread")]
async fn health_is_served_and_follows_the_state() -> TestResult {
    let health = HealthState::new();

    let handle = server(&app("health").await)
        // `configure` registers only the demo service: the health service has to
        // arrive through `Server::health`.
        .health(health.clone())
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;

    let mut probe = HealthClient::new(channel(handle.local_addr()).await);

    assert_eq!(
        check(&mut probe, "").await?,
        ServingStatus::Serving as i32,
        "a fresh server reports itself serving"
    );

    // Draining: probes start failing while requests still finish.
    health.set_not_serving().await;
    assert_eq!(
        check(&mut probe, "").await?,
        ServingStatus::NotServing as i32,
        "the served status has to move with the state"
    );

    health.set_serving().await;
    assert_eq!(check(&mut probe, "").await?, ServingStatus::Serving as i32);

    // A per-service status is independent of the overall one.
    health
        .set_service_not_serving::<DemoServer<DemoService>>()
        .await;
    assert_eq!(
        check(&mut probe, DemoServer::<DemoService>::NAME).await?,
        ServingStatus::NotServing as i32
    );
    assert_eq!(
        check(&mut probe, "").await?,
        ServingStatus::Serving as i32,
        "naming one service must not move the overall status"
    );
    assert!(health.is_serving());

    handle.shutdown();
    Ok(handle.stopped().await?)
}
