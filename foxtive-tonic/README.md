# foxtive-tonic

A gRPC framework for the [Foxtive](https://github.com/foxtive) ecosystem, built on [tonic](https://github.com/hyperium/tonic) 0.14.

Provides a server builder with production defaults, standardized JSON response envelopes, error mapping, and tower middleware support.

## Install

```toml
[dependencies]
foxtive-tonic = "0.1.0"

[build-dependencies]
tonic-prost-build = "0.14"
```

The library needs no `protoc`: it ships no build script, and its one `.proto`
(`proto/foxtive/tonic/v1/envelope.proto`) is a reference contract that is
never compiled here. You need `protoc` for *your own* `.proto` files, in your own crate.

Rust 1.97 or newer. CI builds the library on exactly that toolchain, and reads the number
out of the manifest rather than repeating it, so the two cannot drift apart. The floor is
`foxtive`'s, not tonic's: the core crate requires 1.97 for itself, while the tonic stack
underneath would have been satisfied by 1.88.

Feature flags:

| Flag | What it does |
|------|-------------|
| `tls` | Adds TLS/mTLS support via `tonic/tls-ring` |

That is the whole list. There is no `transport` flag, because tonic's transport layer
is enabled by the dependency itself and a flag would gate nothing. There is no
`reflection` flag either: the framework never names `tonic_reflection`, because a
reflection service is built from *your* descriptors, so depend on `tonic-reflection`
directly, as shown in [gRPC Reflection](#grpc-reflection).

## Quick Start

`build.rs`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "proto/helloworld.proto";
    tonic_prost_build::compile_protos(proto)?;
    Ok(())
}
```

`main.rs`:

```rust
use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::server::Server;
use tonic::service::Routes;

pub mod hello_proto {
    tonic::include_proto!("helloworld");
}

use hello_proto::greeter_server::{Greeter, GreeterServer};
use hello_proto::{HelloReply, HelloRequest};
use tonic::{Request, Response, Status};

struct GreeterService;

#[tonic::async_trait]
impl Greeter for GreeterService {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let name = request.into_inner().name;
        Ok(Response::new(HelloReply {
            message: format!("Hello {}!", name),
        }))
    }
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let app = App::builder("MyApp", "MYAPP")
        .build()
        .await?;

    Server::new(app)
        .host("127.0.0.1")
        .port(50051)
        .tracing(Tracing::default())
        .configure(|| {
            let mut builder = Routes::builder();
            builder.add_service(GreeterServer::new(GreeterService));
            builder.routes()
        })
        .run()
        .await
}
```

## Server Configuration

The `Server` builder gives you control over networking, timeouts, and lifecycle.

```rust
use std::time::Duration;

Server::new(app)
    .host("0.0.0.0")
    .port(50051)
    .tracing(Tracing::default())

    // Timeouts and limits
    .timeout(Duration::from_secs(30))
    .concurrency_limit(100)
    .max_concurrent_streams(1000)

    // TCP tuning
    .tcp_keepalive(Duration::from_secs(60))
    .tcp_nodelay(true)

    // HTTP/2 tuning
    .http2_keepalive_interval(Some(Duration::from_secs(30)))
    .http2_keepalive_timeout(Some(Duration::from_secs(10)))
    .http2_adaptive_window(Some(true))
    .initial_stream_window_size(Some(1024 * 1024))
    .initial_connection_window_size(Some(2 * 1024 * 1024))

    // Connection lifecycle
    .max_connection_age(Duration::from_secs(300))
    .max_connection_age_grace(Duration::from_secs(30))
    .load_shed(true)

    .configure(|| { /* ... */ })
    .run()
    .await
```

### Lifecycle Hooks

The order below is what `spawn()` performs: bootstrap, startup hooks, bind, serve,
`on_started`.

```rust
Server::new(app)
    // Runs before the socket is bound. Failure aborts startup with no listener open.
    .bootstrap(|app| async move {
        // Initialize database, load config, etc.
        Ok(())
    })

    // Spawned, not awaited: by the time this runs the listener is bound and
    // accepting, so a hook may call the server it is describing, and a hook that
    // never finishes cannot keep the server from serving.
    .on_started(async move {
        tracing::info!("Server is ready");
    })

    // Runs on shutdown (SIGTERM / Ctrl+C), after in-flight requests are drained
    .on_shutdown(async move {
        tracing::info!("Shutting down gracefully");
    })

    // Override the default shutdown signal
    .shutdown_signal(async move {
        // Custom signal, e.g. from a watch channel
    })
```

To start serving and keep control of the process, use `spawn()` instead of `run()`:
it binds before it returns, so a port already in use is reported as an error rather
than after the hooks have claimed success, and it hands back the bound address.

```rust
use foxtive_tonic::server::Server;

let handle = Server::new(app)
    .host("127.0.0.1")
    .port(0) // let the OS choose
    .configure(|| { /* ... */ })
    .spawn()
    .await?;

println!("listening on {}", handle.local_addr());

handle.shutdown();  // stop accepting, drain what is in flight
handle.stopped().await?; // wait for the listener to exit
```

Dropping the handle does not stop the server; only `shutdown()`, a shutdown signal, or
`on_shutdown` does.

## Middleware (Tower Layers)

Add tower layers for cross-cutting concerns. Layers run in the order they are added:
the first `.layer()` is the outermost one, so it sees a request first and its response
last, the same semantics as tower's `ServiceBuilder`.

```rust
use foxtive_tonic::metrics::{Metrics, MetricsLayer};
use tower::timeout::TimeoutLayer;

let metrics = Metrics::new();

Server::new(app)
    .layer(MetricsLayer::new(metrics.clone())) // counts every request, timed or shed
    .layer(TimeoutLayer::new(Duration::from_secs(10)))
    .configure(|| { /* ... */ })
    .run()
    .await
```

A layer has to satisfy `tower::Layer<BoxedService>`, where `foxtive_tonic::server::BoxedService`
is the service the router hands over, and its `Service` must answer `Request<Body>`
with `Response<Body>` and a boxed error. The generic pattern below meets that on its
own; see [Custom Layers](#custom-layers).

### Custom Layers

Implement the tower `Layer` + `Service` pattern:

```rust
use tonic::codegen::http::{Request, Response};
use tower::{Layer, Service};

#[derive(Clone)]
struct MyLayer;

impl<S> Layer<S> for MyLayer {
    type Service = MyService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        MyService { inner }
    }
}

#[derive(Clone)]
struct MyService<S> {
    inner: S,
}

impl<S, ReqBody> Service<Request<ReqBody>> for MyService<S>
where
    S: Service<Request<ReqBody>, Response = Response<tonic::body::Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // Extract metadata, log, auth check, etc.
        let future = self.inner.call(req);
        Box::pin(async move { future.await })
    }
}
```

## Response Envelope

Every response carries the same five fields:

```json
{
  "code": "000",
  "success": true,
  "timestamp": 1726900000,
  "message": "User created",
  "data": { "id": 42, "name": "Alice" }
}
```

They reach the client in one of two carriers: as a JSON `String` in a proto field, which
is the convention the rest of the Foxtive ecosystem (notably foxtive-axum) speaks, or as
a real protobuf [`Envelope`](#the-typed-envelope). The field mapping is shared, so the two
cannot disagree about what a response means.

### Using the Responder

```rust
use foxtive_tonic::grpc::responder::Responder;
use foxtive_tonic::enums::response_code::ResponseCode;

// Success with data
let json = Responder::send(my_data, ResponseCode::Ok);

// Success with message
let json = Responder::send_msg(my_data, ResponseCode::Created, "User created");

// Convenience methods
let json = Responder::ok_message("Operation successful");
let json = Responder::not_found_message("User not found");
let json = Responder::bad_req_message("Invalid email format");
```

### Extension Traits on AppResult

If your service layer returns `AppResult<T>` (from foxtive), use the extension traits:

```rust
use foxtive_tonic::grpc::response::ext::ResponderExt;
use foxtive_tonic::grpc::response::ext::ResultResponseExt;

// In a handler:
let result: AppResult<UserData> = get_user(id);

// Respond with default Ok code
let response = result.respond()?;

// Or with a specific code and message
let response = result.respond_code("User fetched", ResponseCode::Ok)?;

// Or send with a code
let response = result.send_result_msg(ResponseCode::Ok, "User fetched")?;
```

### The Typed Envelope

The same five fields as a protobuf message, so a gRPC-only consumer gets a schema instead
of a string it has to parse.

```rust
use foxtive_tonic::grpc::envelope::Envelope;
use foxtive_tonic::enums::response_code::ResponseCode;

// data under the default success code
let envelope = Envelope::ok(&user)?;

// an explicit code, and a code with a message
let envelope = Envelope::with_code(&user, ResponseCode::Created)?;
let envelope = Envelope::with_msg(&user, ResponseCode::Created, "User created")?;

// a message and no payload at all, for acknowledgements
let envelope = Envelope::message_only("User deleted", ResponseCode::Ok);

// the payload back as JSON
let data: serde_json::Value = envelope.data_json();
```

Name the handler's return type `EnvelopeResult`, which is
`Result<tonic::Response<Envelope>, tonic::Status>`:

```rust
use foxtive_tonic::grpc::envelope::{Envelope, EnvelopeResult};

async fn get_user(&self, request: Request<UserId>) -> EnvelopeResult {
    let user = find_user(request.into_inner().id).await.map_err(GrpcError::AppError)?;
    Ok(Response::new(Envelope::ok(&user)?))
}
```

`Envelope` is wire-compatible with
`foxtive-tonic/proto/foxtive/tonic/v1/envelope.proto`, which ships as documentation and is
never compiled by this crate. The two are pinned against each other by
`foxtive-tonic-examples/tests/envelope.rs`, which decodes the library's bytes with a type
generated from that `.proto`, encodes in the other direction, and reads the field names
back out of the descriptor. Change a tag, a type, a name or the presence of a field on one
side and those tests fail.

The payload is a `google.protobuf.Value`, whose numbers are IEEE-754 doubles, so
**integers above 2^53 lose precision** through `Envelope::data`. Put ids of that size in a
typed response message, or send them as strings.

### Two Styles Side by Side

Collapsing the four `respond` traits into one is a breaking change that no release has made yet,
so both shapes coexist and one handler may mix them. The same service, both ways:

```rust
// A proto `string` holding JSON, for clients that already parse Foxtive envelopes.
async fn lookup_text(&self, request: Request<UserId>) -> Result<Response<BodyJson>, Status> {
    let found: AppResult<User> = find_user(request.into_inner().id).await;
    Ok(Response::new(BodyJson {
        body: found.respond()?, // String, and a Status on the error half
    }))
}

// The same response as protobuf.
async fn lookup_typed(&self, request: Request<UserId>) -> EnvelopeResult {
    let user = find_user(request.into_inner().id).await.map_err(GrpcError::AppError)?;
    Ok(Response::new(Envelope::ok(&user)?))
}
```

A failure is one shape either way: `GrpcError::into_status` puts the **JSON** envelope in
`Status::details`, so an error reads the same from a typed service as from a `String`
one.

Rule of thumb: use the typed `Envelope` for new services, and the `String` envelope for
payloads already consumed by foxtive-axum-shaped clients.

## Error Handling

`GrpcError` converts application errors into gRPC statuses automatically:

```rust
use foxtive_tonic::error::GrpcError;
use tonic::Status;

// From a tonic Status
let err = GrpcError::Status(Status::not_found("User not found"));

// From an AppMessage (foxtive)
let err = GrpcError::AppError(AppMessage::NotFound("User"));

// From a standard error
let err = GrpcError::Std(Box::new(io::Error::last_os_error()));

// Convert to tonic Status
let status: Status = err.into();
```

Status code mapping:

| HTTP Status | gRPC Code |
|-------------|-----------|
| 200-299 | `Ok` |
| 400 | `InvalidArgument` |
| 401 | `Unauthenticated` |
| 403 | `PermissionDenied` |
| 404 | `NotFound` |
| 409 | `AlreadyExists` |
| 429 | `ResourceExhausted` |
| 501 | `Unimplemented` |
| 503 | `Unavailable` |
| 504 | `DeadlineExceeded` |
| other | `Internal` |

## Health Checks

`HealthState` drives the standard `grpc.health.v1.Health` service. You do not add that
service to your `Routes`: handing it to `.health()` makes the server register it, so
`grpc_health_probe`, Envoy's health checker and Kubernetes readiness probes work without
any wiring in your crate.

```rust
use foxtive_tonic::health::HealthState;

let health = HealthState::new();
let warming = health.clone();

Server::new(app)
    .health(health)
    .on_started(async move {
        // Run whatever readiness check you actually have, then open for business.
        warming.set_serving().await;
    })
    .configure(|| { /* ... */ })
    .run()
    .await
```

| Call | Effect |
|------|--------|
| `set_serving().await` | overall status (`""`) becomes `SERVING` |
| `set_not_serving().await` | overall status becomes `NOT_SERVING`; requests already in flight still complete |
| `set_service_serving::<S>().await` | status of one service, by its `NamedService` |
| `set_service_not_serving::<S>().await` | same, to `NOT_SERVING` |
| `set_status(name, serving).await` | status of a service named at runtime |
| `is_serving()` | the overall flag, synchronously; useful for an HTTP `/healthz` |

A fresh `HealthState` already reports `SERVING` for `""`, and a `Check` for a service name
nobody registered fails as `NotFound`, not as `NOT_SERVING`. Per-service status is
independent: marking one service `NOT_SERVING` does not move the overall flag.

Setters are `async`, because tonic-health keeps its status table behind an async lock.
Draining is two steps, and the order matters: flip health first so the load balancer
stops sending traffic, then shut the listener down.

```rust
health.set_not_serving().await;
handle.shutdown();
handle.stopped().await?;
```

If you serve reflection, register `foxtive_tonic::health::FILE_DESCRIPTOR_SET` alongside
your own, or `grpcurl list` will not show `grpc.health.v1.Health`.

## Metrics

`MetricsLayer` is a counter set you poll yourself; nothing is exported. `Metrics` is
`Clone` and every counter is shared, so read it from an admin route, a task, or a test.

```rust
use foxtive_tonic::metrics::{Metrics, MetricsLayer};

let metrics = Metrics::new();

Server::new(app)
    .layer(MetricsLayer::new(metrics.clone()))
    .configure(|| { /* ... */ })
    .run()
    .await;

println!("requests: {}", metrics.total_requests());
println!("errors:   {}", metrics.total_errors());
println!("active:   {}", metrics.active_requests());
println!("p50-ish:  {} ms", metrics.requests_under(100));

let snapshot = metrics.snapshot(); // one consistent view of all of it
```

Two details are worth knowing, because they decide whether the numbers mean anything:

- **`total_errors` counts a failing gRPC call.** A unary handler that returns a `Status`
  arrives at a layer as `Ok(Response)` with the status in the headers, not as `Err`.
  The layer reads `grpc-status`, so `NotFound`, `Internal` and a rejection by an inner
  layer (`TimeoutLayer`, `LoadShedLayer`) all count.
- **`active_requests` cannot leak.** The counter is decremented by a guard released when
  the response body is dropped *or* when the call's future is dropped: a client that
  disconnects mid-stream releases its slot. Accounting for a request therefore finishes
  a moment after its status reaches the client, which is why a test should poll for the
  value rather than assert it immediately.

`total_duration_ms` and the histogram in `snapshot().latency_buckets` are recorded on the
same drop, so they cover the full response, not just the handler. The buckets are the
public `LATENCY_BUCKETS_MS` bounds, whose last entry is `u64::MAX`, so no request falls
out of the histogram.

## TLS

Requires the `tls` feature:

```toml
foxtive-tonic = { version = "0.1.0", features = ["tls"] }
```

```rust
use tonic::transport::{ServerTlsConfig, Identity};

let cert = std::fs::read("cert.pem")?;
let key = std::fs::read("key.pem")?;
let identity = Identity::from_pem(cert, key);

Server::new(app)
    .tls(ServerTlsConfig::new().identity(identity))
    .configure(|| { /* ... */ })
    .run()
    .await
```

For mTLS, add a client CA root:

```rust
use tonic::transport::Certificate;

let client_ca = std::fs::read("ca.pem")?;
let tls = ServerTlsConfig::new()
    .identity(identity)
    .client_ca_root(Certificate::from_pem(client_ca));
```

## gRPC Reflection

Reflection is wired by the consumer, not the framework: the service is built from your own
encoded descriptors, so there is no `reflection` feature here. Depend on
`tonic-reflection` yourself:

```toml
foxtive-tonic = "0.1.0"
tonic-reflection = { version = "0.14", features = ["server"] }
```

Update `build.rs` to generate the file descriptor set:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "proto/helloworld.proto";
    let out_dir = std::env::var("OUT_DIR")?;

    tonic_prost_build::configure()
        .file_descriptor_set_path(format!("{}/helloworld.bin", out_dir))
        .compile_protos(&[proto], &["proto/"])?;

    Ok(())
}
```

Register the reflection service:

```rust
pub mod hello_proto {
    tonic::include_proto!("helloworld");
    pub const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("helloworld");
}

Server::new(app)
    .configure(|| {
        let mut builder = Routes::builder();
        builder.add_service(GreeterServer::new(GreeterService));

        let reflection = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(hello_proto::FILE_DESCRIPTOR_SET)
            .build_v1()
            .expect("Failed to build reflection service");
        builder.add_service(reflection);

        builder.routes()
    })
    .run()
    .await
```

Now `grpcurl` works without proto files:

```bash
grpcurl -plaintext localhost:50051 list
grpcurl -plaintext -d '{"name":"Alice"}' localhost:50051 helloworld.Greeter/SayHello
```

## Examples

The examples live in the `foxtive-tonic-examples` workspace member, which owns
`proto/helloworld.proto` and its build script. That keeps `protoc` out of the
library entirely; you only need `protoc` for your own `.proto` files.

| Example | Run |
|---------|-----|
| Basic server | `cargo run -p foxtive-tonic-examples --example basic` |
| HelloWorld + reflection | `cargo run -p foxtive-tonic-examples --example helloworld` |
| TLS server | `cargo run -p foxtive-tonic-examples --example tls_server` |
| Custom interceptors | `cargo run -p foxtive-tonic-examples --example custom_interceptor` |
| Graceful shutdown | `cargo run -p foxtive-tonic-examples --example graceful_shutdown` |
| High-traffic tuning | `cargo run -p foxtive-tonic-examples --example high_traffic` |
| Error handling | `cargo run -p foxtive-tonic-examples --example error_handling` |
| Request ID propagation | `cargo run -p foxtive-tonic-examples --example request_id` |
| Typed envelope responses | `cargo run -p foxtive-tonic-examples --example typed_envelope` |

`reflection` and `tls` are default features of that member, so no `--features`
flag is needed. `tls_server` reads `certs/server.pem` and `certs/server.key`
relative to the directory you run it from; generate them with:

```bash
mkdir -p certs && openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout certs/server.key -out certs/server.pem \
  -days 365 -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost"
```

## License

MIT
