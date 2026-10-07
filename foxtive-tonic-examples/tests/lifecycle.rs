//! Server lifecycle over real sockets: ephemeral ports, bind failures, and the
//! drain that a graceful shutdown is supposed to perform.

use foxtive_tonic::server::Server;
use foxtive_tonic_examples::proto::demo::demo_server::DemoServer;
use foxtive_tonic_examples::proto::demo::{PingRequest, SleepRequest};
use foxtive_tonic_examples::services::DemoService;
use foxtive_tonic_examples::test_support::{TestResult, app, demo_client, server};
use std::time::{Duration, Instant};
use tonic::service::Routes;

/// `.port(0)` is usable, because `spawn()` binds before it returns.
#[tokio::test(flavor = "multi_thread")]
async fn port_zero_reports_its_real_address_and_serves() -> TestResult {
    let app = app("port-zero").await;

    let handle = server(&app)
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;

    assert_ne!(
        handle.local_addr().port(),
        0,
        "the OS-assigned port has to be readable from the handle"
    );

    let mut client = demo_client(handle.local_addr()).await;
    let reply = client.ping(PingRequest { fail: false }).await?.into_inner();
    assert_eq!(reply.message, "pong");

    handle.shutdown();
    Ok(handle.stopped().await?)
}

/// The failure mode `run()` could not report: a busy port used to surface after
/// `on_started` had already logged that the server was up.
#[tokio::test(flavor = "multi_thread")]
async fn a_busy_port_is_reported_by_spawn() -> TestResult {
    let app = app("busy-port").await;

    let first = server(&app)
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;
    let addr = first.local_addr();

    let second = Server::new(app.clone())
        .host("127.0.0.1")
        .port(addr.port())
        .has_started_bootstrap(true)
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await;

    let err = second.expect_err("binding an address that is already listening must fail");
    assert_eq!(
        err.status_code().as_u16(),
        500,
        "a bind failure is an infrastructure error"
    );

    first.shutdown();
    Ok(first.stopped().await?)
}

/// Shutdown stops the listener but lets a running handler finish.
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_drains_a_request_that_is_already_in_flight() -> TestResult {
    let app = app("drain").await;

    let handle = server(&app)
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;
    let mut client = demo_client(handle.local_addr()).await;

    let inflight = tokio::spawn(async move {
        client
            .sleep(SleepRequest { millis: 400 })
            .await
            .map(|response| response.into_inner().message)
    });

    // Give the call time to reach the handler, then ask the server to stop.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let draining_at = Instant::now();
    handle.shutdown();

    let message = inflight
        .await
        .expect("the call task must not panic")
        .expect("an in-flight request must not be cut off");
    assert_eq!(message, "slept 400ms");

    handle.stopped().await?;

    assert!(
        draining_at.elapsed() >= Duration::from_millis(250),
        "the listener waited for the drain instead of dropping the request: {:?}",
        draining_at.elapsed()
    );
    Ok(())
}

/// A `ServerHandle` that is dropped without being consumed leaves the server up:
/// ownership of the handle is not the shutdown mechanism.
#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_handle_does_not_stop_the_server() -> TestResult {
    let app = app("drop-handle").await;

    let handle = server(&app)
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;
    let addr = handle.local_addr();
    drop(handle);

    let mut client = demo_client(addr).await;
    let reply = client
        .ping(PingRequest { fail: false })
        .await
        .expect("a dropped handle must not stop the listener")
        .into_inner();
    assert_eq!(reply.message, "pong");

    Ok(())
}
