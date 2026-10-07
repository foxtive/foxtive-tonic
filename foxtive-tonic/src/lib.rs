//! # Foxtive Tonic
//!
//! A Rust gRPC framework built on top of [Tonic](https://github.com/hyperium/tonic)
//! providing standardized response formats, error handling, and utilities
//! for building gRPC services in the Foxtive ecosystem.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use foxtive::results::AppResult;
//! use foxtive::setup::trace::Tracing;
//! use foxtive_tonic::server::Server;
//! use tonic::service::Routes;
//! use foxtive::App;
//!
//! #[tokio::main]
//! async fn main() -> AppResult<()> {
//!     let app = App::builder("MyApp", "MYAPP")
//!         .tracing(Tracing::default())
//!         .build()
//!         .await?;
//!
//!     Server::new(app)
//!         .host("127.0.0.1")
//!         .port(50051)
//!         .has_started_bootstrap(true)
//!         .configure(|| {
//!             let builder = Routes::builder();
//!             // builder.add_service(&my_grpc_service);
//!             builder.routes()
//!         })
//!         .run()
//!         .await
//! }
//! ```

/// Trait contracts for response codes and other abstractions.
pub mod contracts;
/// Enumerations for response codes and gRPC status mapping.
pub mod enums;
/// gRPC error types with automatic status conversion.
pub mod error;
/// gRPC response builder, extension traits, and standardized envelopes.
pub mod grpc;
/// Health check service for liveness and readiness probes.
pub mod health;
/// Helper utilities for building JSON responses.
pub mod helpers;
/// Metrics and observability utilities.
pub mod metrics;
/// Server configuration and lifecycle management.
pub mod server;
