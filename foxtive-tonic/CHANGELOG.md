# Foxtive Tonic Changelog

### 0.1.0 (2026-11-19)

Initial release of foxtive-tonic, a gRPC micro-framework for the Foxtive ecosystem, built on
tonic 0.14 and layered over the `foxtive` core crate. Nothing was published before this version,
so this entry describes what the crate is rather than what changed: there is no earlier API to
migrate from.

#### Server

- `Server::new(app)` binds `0.0.0.0:50051` unless told otherwise, and forwards the tuning it
  exposes (address, timeout, per-connection concurrency, TCP keepalive with its interval and
  retries, TCP nodelay, HTTP/2 keepalive interval and timeout, adaptive window, stream and
  connection window sizes, max concurrent streams, max connection age and its grace period, load
  shedding, TLS) to tonic's transport builder or to the `TcpIncoming` that owns the socket.
- `spawn()` and `ServerHandle` (`local_addr`, `shutdown`, `stopped`): serve in the background and
  stop on your terms. Everything up to and including the bind runs on the calling task, so a port
  already in use, a port below 1024 without permission, or a malformed host is an `Err` rather
  than a success message followed by silence. `.port(0)` serves on and reports the port the OS
  assigned. `run()` is `spawn()` + `stopped()`.
- Lifecycle order is fixed and documented: bootstrap, app startup hooks, bind, serve,
  `on_started`. Hooks that can fail run before the bind, so a database that will not answer
  keeps the server from ever accepting traffic. `on_started` runs after the bind and is spawned,
  not awaited: it may call the server it is describing, and a hook that never finishes cannot
  keep that server from serving.
- Layers added with `.layer()` run in the order they are added: the first is outermost. That is
  tower's `ServiceBuilder` semantics, and it is what composing `MetricsLayer` with
  `TimeoutLayer` depends on.
- Shutdown drains requests already in flight, then runs `.on_shutdown()` and the app's shutdown
  hooks. `.shutdown_signal()` replaces the default `Ctrl+C` plus `SIGTERM`, and a signal that
  cannot be installed falls back to the ones that can instead of aborting the process.
- Nothing in the library panics over configuration it dislikes: a missing `.configure()` is an
  `AppMessage::Infrastructure` error, and a global tracing subscriber that already exists wins:
  the ignored `.tracing()` argument is logged, so two servers in one process (a test suite, an
  app on two ports) both start. `.panic_hook(true)` opts in to `foxtive`'s panic hook.

#### Responses

- The JSON envelope (`code`, `success`, `timestamp`, `message`, `data`) carried as a `String`
  in a proto field, which is the convention the rest of the Foxtive ecosystem (foxtive-axum above
  all) speaks: `Responder::send`, `send_msg`, `ok_message`, `not_found_message` and
  `bad_req_message`, plus the `AppResult` extension traits `ResultResponseExt`, `AppMessageExt`,
  `ResponderExt` and `OptionResultResponseExt`.
- Typed `Envelope` (`grpc::envelope`): the same five fields as a real protobuf message with a
  `google.protobuf.Value` payload, with `Envelope::ok`, `with_code`, `with_msg`, `message_only`,
  `error`, `data_json`, and the `EnvelopeResult` handler alias.
  `proto/foxtive/tonic/v1/envelope.proto` is the contract. The library has no build script, so it
  is never compiled here; the examples' tests pin the hand-written struct and that `.proto`
  against each other in both directions, so a tag, type or name drift on one side fails a test.
- `helpers::proto_json`: hand-written conversions between `serde_json::Value` and
  `prost_types::Value`, since prost-types ships no bridge. They inherit `Value`'s precision limit,
  which the module documents: its numbers are doubles, so integers above 2^53 do not survive the
  typed path.
- Both carriers are supported and neither is deprecated. The four `respond` traits stay four:
  collapsing them into a single `Respond` is a breaking change this release does not make, so the
  two idioms coexist and one handler may mix them. The field mapping is shared, so they cannot
  disagree about what a response means.

#### Errors

- `GrpcError` (`Status`, `AppError`, `Std`) with conversions from `AppMessage`, `tonic::Status`
  and `std::error::Error`, and the documented HTTP-to-gRPC status mapping.
- `ResponseCode` covers all 19 codes, each carrying its gRPC status, its HTTP status and its
  success flag.
- Every variant reaches `Status::details` as the same JSON envelope shape, and no error path can
  report `code: "000"` / `Code::Ok`.
- A serialization failure logs an `error!` and emits a valid `009` envelope, not an empty body,
  which on the wire is indistinguishable from a successful empty payload, and not a panic in the
  caller.

#### Health

- `grpc.health.v1.Health` served by tonic-health and registered by `Server::health(state)`, so
  `grpc_health_probe`, Envoy's health checker and Kubernetes readiness probes work with no wiring
  in your crate.
- `HealthState`: `set_serving`, `set_not_serving`, `set_service_serving::<S>`,
  `set_service_not_serving::<S>` and `set_status(name, serving)` are `async`, because tonic-health
  keeps its status table behind an async lock; `is_serving()` stays synchronous for an HTTP
  `/healthz`. A fresh state reports `SERVING` for the overall `""`, per-service status is
  independent of that flag, and a `Check` for a name nobody registered fails as `NotFound` rather
  than `NOT_SERVING`.
- `health::FILE_DESCRIPTOR_SET` to register alongside your own descriptors if you serve
  reflection, or `grpcurl list` will not show the health service.

#### Metrics

- `Metrics` and `MetricsLayer`: `total_requests`, `total_errors`, `active_requests`,
  `total_duration_ms`, `requests_under(ms)`, and `snapshot()` for one consistent view, whose
  histogram uses the public `LATENCY_BUCKETS_MS` bounds (the last is `u64::MAX`), so no request
  falls outside it. `MetricsLayer::metrics()` reads the counters back off the layer. `Metrics` is
  `Clone` and every counter is shared, so an admin route, a background task or a test can poll it.
  Nothing is exported; these are counters you read yourself.
- Error counting reads `grpc-status` from the response, because tonic delivers a failing unary
  call as `Ok(Response)` with the status in the headers: handler failures and rejections by inner
  layers (timeout, load shedding) both count.
- `active_requests` is released by a guard dropped with the response body *or* with the call's
  future, so a client that disconnects, times out, or is shed gives its slot back. Accounting for
  a request therefore finishes a moment after its status reaches the client.

#### Packaging and compatibility

- Requires `foxtive` 1.6 for the DI container, `App` and bootstrap. That dependency is also what
  sets `rust-version = "1.97"`: the tonic stack alone would have been satisfied by 1.88, and
  `foxtive` is not optional.
- Edition 2024, MIT (`LICENSE` ships), `default = []`, and exactly one feature, `tls`, which adds
  `tonic/tls-ring`. There is no `transport` feature, because the workspace dependency pin enables
  tonic's transport itself and a flag would gate nothing, and no `reflection` feature, because the
  framework never names `tonic_reflection`: a reflection service is built from *your* descriptors,
  so a crate that wants it depends on `tonic-reflection` directly.
- The published library carries no build script and no compiled protos, so installing it never
  requires `protoc`; you need it only for your own `.proto` files, in your own crate. `include`
  ships `src/`, the reference contract `.proto`, `README.md`, `CHANGELOG.md` and
  `LICENSE`.
- The crate root re-exports nothing from its dependencies: `App` and `AppBuilder` come from
  `foxtive`, `Arc` from `std::sync`.
- Examples, protos, codegen and integration tests live in the unpublished `foxtive-tonic-examples`
  workspace member, which owns the `tonic-prost-build` build script.

#### Tooling

- CI (`.github/workflows/ci.yml`): format, clippy under `-D warnings`, workspace tests with all
  features, `cargo doc` under `-D warnings`, an `msrv` job that reads `rust-version` out of the
  manifest instead of repeating it here, a protoc-free check with a positive and a negative
  control, and a publish dry run that asserts the package contents.
- Public items require doc comments (`missing_docs`), so one without a comment fails CI.
