//! Health check service for liveness and readiness probes.
//!
//! Health is served as the standard `grpc.health.v1.Health` service (via
//! `tonic-health`), so `grpc_health_probe`, Kubernetes gRPC probes, Envoy
//! health checks and `grpcurl` all work against it without any custom contract.
//!
//! Register a [`crate::health::HealthState`] with
//! [`Server::health`](crate::server::Server::health) and flip the status from
//! lifecycle hooks; see that method's example.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tonic::server::NamedService;
use tonic_health::ServingStatus;
use tonic_health::pb::health_server::HealthServer;
use tonic_health::server::{HealthReporter, HealthService};

/// The service name that means "the server as a whole" in the health protocol.
const OVERALL_SERVER: &str = "";

/// Encoded `grpc_health_v1.proto` descriptor set.
///
/// Register this with a reflection service if you want `grpcurl list` to report
/// `grpc.health.v1.Health` alongside your own services.
pub const FILE_DESCRIPTOR_SET: &[u8] = tonic_health::pb::FILE_DESCRIPTOR_SET;

/// Health check service state, shared between the served `Health` service and
/// whatever decides readiness (lifecycle hooks, a health endpoint, an admin task).
///
/// Cloning shares one status store, so a clone handed to any task controls the
/// same wire state.
#[derive(Clone, Debug)]
pub struct HealthState {
    reporter: HealthReporter,
    /// Local mirror of the overall-server status, so [`HealthState::is_serving`]
    /// stays a synchronous read. The reporter's value is authoritative; the two
    /// are written together.
    serving: Arc<AtomicBool>,
}

impl HealthState {
    /// Create a state that reports the server as serving.
    ///
    /// The health protocol has no "starting" state; call
    /// [`HealthState::set_not_serving`] during startup to fail probes before ready.
    pub fn new() -> Self {
        Self {
            // HealthReporter::new pre-registers "" as Serving, matching the mirror's start.
            reporter: HealthReporter::new(),
            serving: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Mark the server as serving (ready).
    pub async fn set_serving(&self) {
        self.serving.store(true, Ordering::SeqCst);
        self.reporter
            .set_service_status(OVERALL_SERVER, ServingStatus::Serving)
            .await;
    }

    /// Mark the server as not serving (not ready).
    ///
    /// Call this when draining so in-flight work can finish while probes start
    /// failing.
    pub async fn set_not_serving(&self) {
        self.serving.store(false, Ordering::SeqCst);
        self.reporter
            .set_service_status(OVERALL_SERVER, ServingStatus::NotServing)
            .await;
    }

    /// Whether this handle currently reports the server as serving.
    ///
    /// Reads the local mirror, which the setters write together with the served
    /// `Health` service.
    pub fn is_serving(&self) -> bool {
        self.serving.load(Ordering::SeqCst)
    }

    /// Mark one service (by its `NamedService` type) as serving.
    pub async fn set_service_serving<S: NamedService>(&self) {
        self.reporter
            .set_service_status(S::NAME, ServingStatus::Serving)
            .await;
    }

    /// Mark one service (by its `NamedService` type) as not serving.
    pub async fn set_service_not_serving<S: NamedService>(&self) {
        self.reporter
            .set_service_status(S::NAME, ServingStatus::NotServing)
            .await;
    }

    /// Set the status of a service named literally, for callers that do not have
    /// the service type in scope.
    pub async fn set_status(&self, service_name: &str, serving: bool) {
        let status = if serving {
            ServingStatus::Serving
        } else {
            ServingStatus::NotServing
        };
        if service_name == OVERALL_SERVER {
            self.serving.store(serving, Ordering::SeqCst);
        }
        self.reporter.set_service_status(service_name, status).await;
    }

    /// The `grpc.health.v1.Health` server backed by this state.
    ///
    /// Used by [`Server::health`](crate::server::Server::health).
    pub(crate) fn health_server(&self) -> HealthServer<HealthService> {
        HealthServer::new(HealthService::from_health_reporter(self.reporter.clone()))
    }
}

impl Default for HealthState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_reports_serving() {
        let state = HealthState::new();
        assert!(state.is_serving());
        assert!(HealthState::default().is_serving());
    }

    #[tokio::test]
    async fn setters_round_trip_the_mirror() {
        let state = HealthState::new();
        state.set_not_serving().await;
        assert!(!state.is_serving());
        state.set_serving().await;
        assert!(state.is_serving());

        state.set_status("helloworld.Greeter", false).await;
        assert!(
            state.is_serving(),
            "a per-service status must not move the mirror"
        );
        state.set_status(OVERALL_SERVER, false).await;
        assert!(!state.is_serving());
    }

    #[tokio::test]
    async fn clones_share_one_status_store() {
        let state = HealthState::new();
        let clone = state.clone();
        clone.set_not_serving().await;
        assert!(!state.is_serving());
    }

    #[test]
    fn health_server_is_named_by_the_health_protocol() {
        assert_eq!(
            <HealthServer<HealthService> as NamedService>::NAME,
            "grpc.health.v1.Health"
        );
    }

    #[test]
    fn descriptor_set_is_not_empty() {
        assert!(!FILE_DESCRIPTOR_SET.is_empty());
    }
}
