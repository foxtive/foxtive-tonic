//! Helpers shared by the integration tests in `tests/`.
//!
//! These live in the library rather than a `tests/common/mod.rs` so each test
//! binary does not recompile them and the unused-code lint stays quiet.

use crate::proto::demo::demo_client::DemoClient;
use foxtive::App;
use foxtive_tonic::server::Server;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::Channel;

/// How long [`eventually`] waits before declaring that a condition will not hold.
const DEADLINE: Duration = Duration::from_secs(10);

/// Return type of an integration test.
///
/// A test asserts on both halves of the boundary, so the signature must accept
/// `AppMessage` from the server and `tonic::Status` from the client, neither of
/// which converts into the other.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// An `App` with no infrastructure attached.
///
/// Built without `.tracing(...)` so it neither installs a global subscriber nor
/// needs one: a test binary runs many servers in one process, and the subscriber
/// can only be set once.
pub async fn app(name: &str) -> Arc<App> {
    App::builder(name, "TEST")
        .environment(foxtive::Environment::Local)
        .build()
        .await
        .expect("an App with no infrastructure always builds")
}

/// A `Server` preset for tests: loopback, an ephemeral port, no implicit setup.
///
/// Each test reads its real address from [`ServerHandle::local_addr`], so no two
/// can collide on a fixed port and none can pass because a port happened to be free.
///
/// [`ServerHandle::local_addr`]: foxtive_tonic::server::ServerHandle::local_addr
pub fn server(app: &Arc<App>) -> Server {
    Server::new(app.clone())
        .host("127.0.0.1")
        .port(0)
        .has_started_bootstrap(true)
}

/// A connected channel to a listening server.
///
/// `connect` performs the handshake, so a server that is not accepting fails here
/// instead of on the first RPC.
pub async fn channel(addr: SocketAddr) -> Channel {
    Channel::from_shared(format!("http://{addr}"))
        .expect("a SocketAddr always forms a valid URI")
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .unwrap_or_else(|err| panic!("connecting to {addr} must work: {err}"))
}

/// A [`DemoClient`] connected to `addr`.
pub async fn demo_client(addr: SocketAddr) -> DemoClient<Channel> {
    DemoClient::new(channel(addr).await)
}

/// Wait for `check` to become true, or fail the test naming `what`.
///
/// Server-side accounting lands when the response body is released, on a different
/// task from the one that delivers the response, so polling establishes the
/// counter has updated before the test asserts on it.
pub async fn eventually(what: &str, check: impl Fn() -> bool) {
    let started = std::time::Instant::now();

    while !check() {
        assert!(
            started.elapsed() < DEADLINE,
            "{what} did not happen within {DEADLINE:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
