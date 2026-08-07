# Custom OTLP exporter rewrite — implementation plan

**Status:** Steps 0-4 done — pbjson JSON integration, transport layer, Event→proto conversion, batch trace exporter, and batch log exporter all implemented and committed; Step 5 (metrics pipeline) pending
**Branch:** `feat/custom-otlp-exporter`
**Module:** `modules/observability-otlp` (`ferron-observability-otlp`)

This document is the step-by-step plan for replacing the OpenTelemetry Rust SDK
(`opentelemetry`, `opentelemetry-sdk`, `opentelemetry-otlp`, `opentelemetry-http`)
inside `modules/observability-otlp` with a custom OTLP exporter built on top of
the prost-generated protobuf types already available in
`src/proto.rs` (the `proto` module at the crate root).

The exporter must support:

- OTLP/gRPC (tonic, generated clients, port 4317)
- OTLP/HTTP with binary protobuf (`application/x-protobuf`)
- OTLP/HTTP with JSON protobuf (`application/json`, `pbjson` or equivalent)
- a **batch log exporter** (replaces `BatchLogProcessor`)
- a **batch trace exporter** (replaces `BatchSpanProcessor`)
- a **periodic metric reader** (replaces `PeriodicReader` + `SdkMeterProvider`)
- metrics: **native (Base2 exponential) histograms**, **metric exemplars**, and
  the follow-ups listed in [Metrics scope](#metrics-scope)

---

## 1. Current state (summary)

### 1.1 What exists today

| File | Role |
| --- | --- |
| `src/lib.rs` | `OtlpEventSink` (bounded `async_channel` of 131072 events), `OtlpObservabilityModule` event loop, per-config provider cache (`OtlpProviderCache`) keyed by `config_cache_key`, shutdown via `spawn_blocking` + `provider.shutdown()`, `OtlpObservabilityModuleLoader` with directives + validator registration. |
| `src/config.rs` | `OtlpBackendConfig` (service_name, no_verify, per-signal `SignalConfig`, baggage promotions, `LogStyle`, global authorization). Protocol default: `grpc` if port is 4317, else `http/protobuf`. |
| `src/validator.rs` | Directive validation + best-practice checks. |
| `src/client.rs` | `HyperOtelClient` (hyper-util + hyper-rustls, native certs + webpki-roots fallback, `no_verification` support) implementing `opentelemetry_http::HttpClient`; `build_tonic_channel` (tonic + same TLS). **Reusable as-is for the custom exporter.** |
| `src/providers/` | `emit_log`, `emit_metric`, `emit_trace`, `emit_access_log` — convert `Event`s into the OTel SDK API; `cache.rs` builds the three `Sdk*Provider`s per config; `context.rs` has `CorrelationContext`, `RequestedIdGenerator` (thread-local requested trace/span IDs), `build_resource`, `build_parent_context`; `metrics.rs` has `CachedInstrument` + `sanitize_label_value`. |
| `src/proto.rs` | `mod proto` with `tonic::include_proto!` for `opentelemetry.proto.{common,resource,logs,metrics,trace,collector.{logs,metrics,trace}}.v1`. |
| `build.rs` | Compiles `opentelemetry-proto` submodule (pinned near v1.11.0) via `protox` + `tonic_prost_build` (`build_server(false)`, `build_client(true)`); auto-updates the submodule. |

### 1.2 Current SDK wiring being replaced (parity baseline)

From `src/providers/cache.rs` — the new implementation must reproduce these
behaviors:

| Signal | Current SDK config | Notes |
| --- | --- | --- |
| Logs | `SdkLoggerProvider` + `BatchLogProcessor` (SDK defaults) | queue 2048, batch 512, delay 5 s, export timeout 30 s |
| Traces | `SdkTracerProvider` + `BatchSpanProcessor` + `AlwaysOn` sampler + `RequestedIdGenerator` | span IDs are forced from the HTTP trace context via thread-local (`providers/context.rs`) |
| Metrics | `SdkMeterProvider` + `PeriodicReader` with **30 s interval**, Base2 exponential histogram view (`max_size 160`, `max_scale 20`, `record_min_max true`) for all histograms | cumulative temporality; histogram boundaries from events are currently discarded by the view |

Protocols per signal: `grpc`, `http/protobuf`, `http/json` (all three already
exercised today via `opentelemetry-otlp`). Authorization: per-signal
`authorization` with global fallback; HTTP header vs. gRPC metadata.

### 1.3 Behavior fixed by the old implementation that must survive the rewrite

- `sanitize_label_value` (128-char cap, control chars → `?`, long values →
  `hash_<hex>`) for metric string attributes (`providers/metrics.rs:26`).
- `EventTraceContext` carries trace/span IDs as **hex-ASCII bytes**
  (`[u8; 32]` / `[u8; 16]`); the exporter must decode them to 16/8 raw bytes
  for the protobuf fields, and re-encode to hex for OTLP/HTTP JSON.
- Baggage key promotion with `DistinctValueTracker` cardinality control
  (`ferron_observability::baggage`).
- `ferron.control_plane.*` attributes on all signals.
- Resource attributes: `service.name`, `process.pid`, `process.start_time`
  (`build_resource` in `providers/context.rs:121`).
- `log_style legacy|modern` semantics for log records
  (`providers/logs.rs`).
- `ferron.request` spans get `SpanKind::Server`; everything else `INTERNAL`.
- Span links (with sampled flags) and `Parent::{ByKey,ById}` correlation
  (`providers/context.rs` `build_parent_context`).
- Shutdown must flush pending data (currently via `spawn_blocking` to avoid a
  deadlock in `BatchSpanProcessor::shutdown`, see `lib.rs:224-239`).
- The docs currently state *"Ferron does not currently support OTLP metric
  exemplars, due to OpenTelemetry SDK limitations"* (`docs/configuration/observability/otlp.md:47`) —
  this rewrite removes that limitation; the note must be deleted in the
  exemplars step.

---

## 2. Target architecture

```
                     EventSink (async_channel, unchanged)
                              │
            ┌─────────────────┼──────────────────┐
            ▼                 ▼                  ▼
     emit_log            emit_metric         emit_trace
            │                 │                  │
            ▼                 ▼                  ▼
   ┌────────────────┐  ┌────────────────┐  ┌────────────────┐
   │ LogBuffer       │  │ MetricStore    │  │ TraceBuffer    │
   │ (proto LogRecord│  │ (instruments → │  │ (proto Span +  │
   │ queue)          │  │  data points)  │  │ correlation)   │
   └───────┬─────────┘  └───────┬────────┘  └───────┬────────┘
           │                   │  PeriodicMetric    │
           │  BatchLogExporter │  Reader (30 s)     │  BatchTraceExporter
           │                   ▼                    │
           └─────────────►  OTLP client  ◄─────────┘
                   gRPC (tonic) │ http/protobuf (hyper)
                                │ http/json (hyper + pbjson)
                                ▼
                       Collector / backend
```

### 2.1 Proposed module layout (all under `src/`)

```
src/
├── proto.rs            # already exists; make `pub`; add pbjson serde includes
├── pipeline/           # NEW: signal buffers + batching + scheduling
│   ├── mod.rs
│   ├── logs.rs         # LogBuffer + BatchLogExporter (flush interval, batch size)
│   ├── traces.rs       # TraceBuffer + BatchTraceExporter
│   └── metrics.rs      # MetricStore (series accumulation) + PeriodicMetricReader
├── transport/          # NEW: encoding + wire protocols
│   ├── mod.rs
│   ├── client.rs       # OtlpTransport trait + shared retry/backoff logic
│   ├── grpc.rs         # tonic unary clients, metadata auth, code→retryable map
│   ├── http.rs         # hyper POST, content types, status→retryable map, Retry-After
│   └── json.rs         # JSON (pbjson) serialization + OTLP deviations (hex IDs)
├── encode.rs           # NEW: Event → proto conversions (attributes, AnyValue, etc.)
├── lib.rs              # event loop rewired: cache → pipeline; shutdown path
├── config.rs           # maybe: new directives (see §5.6)
└── providers/          # DELETED after migration (emit_* move into pipeline/encode)
```

Rationale: `providers/` was an adapter onto the SDK API; its job (turn
`Event`s into telemetry) survives as `encode.rs`, while the SDK role
(accumulate, batch, schedule, export) is taken over by `pipeline/`.

### 2.2 Dependencies

**Add**

| Crate | Why |
| --- | --- |
| `pbjson = "0.9"` | serde JSON serialization for prost types (verified compatible with prost 0.14; enums serialize as integers, 64-bit ints as strings, lowerCamelCase — matches OTLP/HTTP JSON deviations). |
| `pbjson-types = "0.9"` | if well-known types are needed (probably not; add only if required). |
| `serde` | `Serialize` impls come from pbjson, but the JSON body still needs `serde_json`. |
| `serde_json` | JSON payload construction. |

**Build deps:** `pbjson-build = "0.9"` (generates `.serde.rs` includes from the
same `protox` file descriptor set used today; `Builder::register_descriptors`
accepts the encoded `FileDescriptorSet` returned by `protox::compile`).

**Remove after migration** (todo comment already marks them, `Cargo.toml:29`):
`opentelemetry`, `opentelemetry-sdk`, `opentelemetry-otlp`,
`opentelemetry-http`. Keep `prost`, `tonic`, `hyper`/`hyper-util`/
`hyper-rustls`, `rustls`, `rustls-native-certs`, `webpki-roots`, `bytes`,
`http`, `http-body-util`, `async-trait` (only if still needed), `tokio-util`
(cancellation tokens), `lru` (correlation cache), `urlencoding`,
`async-channel`.

Run `cargo shear` at the end to catch leftover unused deps.

---

## 3. Design decisions (locked for this plan)

| # | Decision | Detail |
| --- | --- | --- |
| D1 | Internal accumulation format = prost proto messages | Buffers store `proto::logs::v1::LogRecord`, `proto::trace::v1::Span`, metric data points directly; no intermediate model layer. |
| D2 | Batcher parameters mirror SDK defaults | queue 2048, batch 512, scheduled delay 5 s, export timeout 30 s. Constants in `pipeline/mod.rs`; config directives are a follow-up (§5.6). |
| D3 | Metric reader interval 30 s, cumulative temporality | Parity with `providers/cache.rs:252`. New reader instance on config change ⇒ fresh `start_time_unix_nano` per series. |
| D4 | Histograms: Base2 exponential by default | Parity with the current view (`max_size 160`, `max_scale 20`, min/max recorded). If the event carries explicit boundaries **and** the `native_histograms false` directive (new, §5.6) is set, export an explicit-bucket histogram instead. |
| D5 | Exemplars supported on counters and histograms (both explicit and exponential) | Unlike the Prometheus backend, OTLP exponential histograms accept exemplars, so no mutual exclusion. Keep a small per-series exemplar ring buffer (default capacity 1; store trace_id, span_id, value, timestamp, `filtered_attributes` = empty). |
| D6 | JSON encoding via pbjson + hex-ID handling | pbjson base64-encodes `bytes`; OTLP requires **hex** for `traceId`/`spanId`/`parentSpanId` and exemplar IDs. Handle by serializing the request envelope with pbjson and post-replacing only the base64-encoded ID strings at the four known keys (verified in Step 2 with golden tests; fallback: hand-rolled `Serialize` for the ID-bearing messages only — see §6 risk R3). |
| D7 | No timestamps on events ⇒ exporter assigns them | `time_unix_nano` = ingestion time; `start_time_unix_nano` = first observation per series (D3). |
| D8 | One OTLP client per signal config | Transport selected per `SignalConfig.protocol`; gRPC uses generated tonic clients (`build_client(true)` already enabled in `build.rs`). |
| D9 | Retry/backoff per OTLP spec | gRPC: retryable codes table (§spec: CANCELLED, DEADLINE_EXCEEDED, ABORTED, OUT_OF_RANGE, UNAVAILABLE, DATA_LOSS; RESOURCE_EXHAUSTED only with `RetryInfo`); respect `RetryInfo.retry_delay`. HTTP: 429/502/503/504 retryable, respect `Retry-After`. Exponential backoff with jitter, capped (e.g. 5 s max delay, ~3 attempts), then drop + count. |
| D10 | Partial success = drop + log, never retry | Per spec: `partial_success` with `rejected_*` counts ⇒ log warn with counts; do not resend. |
| D11 | Empty envelopes are not exported | Batch only sends if at least one item (spec: "Empty Telemetry Envelopes"). |
| D12 | Resource & scope | Resource from `build_resource` (moved to `encode.rs`); scope name `"ferron"` (parity with `meter("ferron")` / `tracer("ferron")`). |
| D13 | Request size cap 64 MiB, response cap 4 MiB | On exceed: drop request and count (spec recommendation). |
| D14 | Drop accounting | All drops/invalid inputs increment `ferron.admin.observability_events_dropped` where applicable, plus warn-once `log_warn!` (parity with current sink behavior, `lib.rs:71-77`). |

---

## 4. Step-by-step implementation checklist

Each step has a **Definition of done** and the verification commands:

```sh
cargo test -p ferron-observability-otlp
cargo clippy -p ferron-observability-otlp --all-targets -- -D warnings
cargo fmt --all --check
```

### Step 0 — Groundwork: proto module and pbjson build integration

- [x] Rename/extend `src/proto.rs`: make the module `pub` (`pub mod proto;` in
      `lib.rs`), keep the existing `tonic::include_proto!` structure.
- [x] Extend `build.rs`:
  - [x] keep `protox::compile` (existing 3 collector service protos);
  - [x] serialize the returned `FileDescriptorSet` and feed it to
    `pbjson_build::Builder::new().register_descriptors(&fds)?.build(&[".opentelemetry"])`;
  - [x] add `cargo:rerun-if-changed` for the proto files if not already present.
- [x] Include the generated serde code inside each module in `src/proto.rs`
      (`include!(concat!(env!("OUT_DIR"), "/opentelemetry.proto.common.v1.serde.rs"))`
      etc., mirroring the pbjson-build usage example).
- [x] Add `pbjson`, `pbjson-build` (build), `serde`, `serde_json` to
      `Cargo.toml`.
- [x] Verify JSON output for `Span`, `LogRecord`, `Exemplar`, metric data
      points against the official examples in the submodule
      `opentelemetry-proto/examples/{trace,metrics,logs}.json` (golden
      fixture tests; see §7).

**Definition of done:** `cargo test -p ferron-observability-otlp` passes with
a test that round-trips a `Span` with `traceId`/`spanId` through
`serde_json` and asserts the hex (not base64) representation. No SDK usage in
this step.

- [x] `client.rs`: `ExportResult { Success, PartialSuccess{rejected, message},
      Failure{retryable, retry_after, message} }`, `RetryConfig`, shared
      `retry_with_backoff` (jitter 0.5-1.5x, `Retry-After` cap 60 s), and size
      caps 64 MiB request / 4 MiB response.
- [x] `grpc.rs`: wrap the generated clients (`LogsServiceClient`,
      `MetricsServiceClient`, `TraceServiceClient`) over the tonic channel
      (reuse `src/client.rs` as-is). Apply authorization as gRPC metadata
      (`authorization` key). Map `tonic::Status::code()` to retryable per the
      spec table (D9); decode `RetryInfo` from `grpc-status-details-bin` for
      `RESOURCE_EXHAUSTED`.
- [x] `http.rs`: reuse `HyperOtelClient` (the hyper-util client). POST to the
      configured endpoint (already contains `/v1/...`). Set
      `Content-Type: application/x-protobuf` or `application/json`. Handle
      status → retryable mapping, `Retry-After`, partial-success body
      parsing, 4 MiB response cap.
- [x] `json.rs`: pbjson serialization + D6 hex-ID handling (moved from
      `src/json.rs`, now `src/transport/json.rs`).
- [x] Unit tests: each protocol path against `127.0.0.1` test servers
      (hyper server for HTTP; `tonic::transport::Server` with the generated
      `*_service_server` traits — `build_server(true)` added to `build.rs`)
      verifying: payload bytes decode back to identical proto; content types;
      retryable vs. non-retryable classification; `Retry-After`/`RetryInfo`
      respect; partial success reporting.
**Definition of done:** all three transports send a hand-built request and
the test server receives and decodes it correctly (protobuf bytes + JSON
parse). `client.rs` no longer depends on `opentelemetry_http`.

### Step 2 — Encoding: Event → proto (`src/convert/`)

Move and adapt the conversion logic from `providers/`:

- [x] `LogEvent` → `proto::logs::v1::LogRecord`: severity number/text mapping
      (Error=17, Warn=13, Info=9, Debug=5), body per `log_style` (legacy
      `message` / modern `summary` + typed attributes), `log.target`
      attribute, trace/span ID hex-decode + sampled flags, baggage
      promotion, control-plane attributes.
- [x] Access events → log records (port `emit_access_log` from
      `providers/access_log.rs`, incl. the `log_style` remap via
      `OtelAccessAttributeVisitor`; legacy mode renders via the formatter
      registry with `<unknown access log>` fallback).
- [x] `TraceEvent::{StartSpan,EndSpan}` → `proto::trace::v1::Span`: ID
      handling (requested IDs via `EventTraceContext` hex-decode, random
      fallback with `rand`, never zero), parent resolution
      (`Parent::ByKey` via correlation context / `ById`), kind mapping
      (`ferron.request` → SERVER), links (malformed dropped), status
      (error message → `STATUS_CODE_ERROR`), attributes (builder + semantic
      + control plane + baggage promotions), start = `StartSpan` ingestion
      time, end = `EndSpan` time (clamped to start). LRU overflow finishes
      the evicted span with an error status instead of dropping it.
- [x] `MetricEvent` attribute conversion helpers: typed `metric_key_values`
      from `MetricAttributeValue`, `sanitize_label_value` (128-char cap,
      control-char replacement). Series accumulation is Step 5.
- [x] Resource (`build_resource` with `service.name`, `process.pid`,
      `process.start_time`), scope (`build_scope`), shared
      `kv`/`any_*`/`nanos` helpers.
- [x] Port existing unit tests from `src/tests.rs` to assert against the
      produced proto messages instead of SDK providers; un-comment the
      `correlation_context_tracks_active_spans` test marked
      `TODO: migrate to future custom exporter`.

**Definition of done:** `cargo test -p ferron-observability-otlp` green with
proto-level assertions (e.g. `span.attributes[0].key == "http.request.method"`,
`log_record.body.string_value == summary`).

### Step 3 — Batch trace exporter (`src/pipeline/traces.rs`)

- [x] `TraceBuffer`: bounded queue of finished `Span`s (from
      `EndSpan`; evict-and-drop with LRU or drop-newest when full), sharing
      the existing `CorrelationContext` so spans stay correlated.
- [x] `BatchTraceExporter`: background task (tokio, spawned from the module
      event loop with the module's `CancellationToken`) that flushes when
      batch size (512) or interval (5 s) is hit, wraps items in
      `ExportTraceServiceRequest` (resource + scope), calls the transport,
      applies retry (D9), drains on shutdown with export timeout (30 s).
- [x] Wire into `lib.rs`: replace `traces_provider` with
      `pipeline.traces`; delete `providers/traces.rs` and
      `providers/context.rs` request-ID handling (moved in Step 2).
- [x] Unit tests: batching thresholds (flush at N items, flush at interval),
      queue-full drop behavior, shutdown flush (all buffered spans exported
      before return), retry then drop on persistent failure.

**Definition of done:** with a mock transport, `N` end-span events produce
`ceil(N/512)` export calls; shutdown flushes the remainder; failing exports
retry ≤3 times then drop and increment the dropped counter.

### Step 4 — Batch log exporter (`src/pipeline/logs.rs`)

- [x] `LogBuffer` + `BatchLogExporter`, mirroring Step 3 (same defaults,
      same retry wrapper). Logs and traces may share the generic batcher
      implementation (`pipeline/mod.rs` generic over `ResourceSpans` /
      `ResourceLogs` if convenient — keep it simple; two small structs are
      fine).
- [x] Wire into `lib.rs`; delete `providers/logs.rs`, `providers/access_log.rs`.
- [x] Unit tests: same suite as Step 3 plus `log_style` body/attribute
      assertions at the wire level.

**Definition of done:** identical to Step 3 for the logs signal.

### Step 5 — Metrics pipeline: accumulation, native histograms, exemplars, periodic reader (`src/pipeline/metrics.rs`)

- [ ] `MetricStore`: instrument series registry keyed by
      `(metric_name, attributes)`; created lazily per metric event (mirrors
      today's `CachedInstrument` + `Family::get_or_create` behavior). Records
      unit, description, instrument type.
- [ ] Accumulators per series:
  - Counter (monotonic) / UpDownCounter (non-monotonic): running sum,
    `start_time_unix_nano` (first observation), exemplar ring buffer.
  - Gauge: last value + timestamp.
  - Histogram: count, sum, min/max, and either
    - **exponential** buckets (default, D4): base-2, scale adaptation
      (grow/shrink like `max_scale 20`, `max_size 160`), zero count,
      per-bucket `exemplars`; or
    - **explicit** buckets (only with `native_histograms false` + event
      boundaries): bucket counts over the event's boundaries, min/max.
- [ ] Exemplars (D5): on `observe`/`add` with `trace_context` present, push
      `{trace_id, span_id, value, time_unix_nano}` (hex → raw bytes) into the
      series' ring buffer; attach to the exported data point.
- [ ] `PeriodicMetricReader`: tokio task, 30 s interval, collects all series
      → `ExportMetricsServiceRequest` (`ResourceMetrics`), exports via
      transport with the shared retry path; on config-change restart, fresh
      start times.
- [ ] Wire into `lib.rs`; delete `providers/metrics.rs`, `providers/cache.rs`.
- [ ] Unit tests:
  - monotonic counter never decreases (negative delta dropped);
  - start/time stamps are ordered and correct;
  - exponential histogram bucket math (known values, scale/offset
    correctness, zero count, min/max);
  - explicit-bucket path honors event boundaries;
  - exemplar ring buffer overwrite semantics and proto field values;
  - `sanitize_label_value` still applied to metric string attributes;
  - reader interval test with a mock clock (inject `Instant` source or use
    short intervals in tests) — flush-on-interval, no empty exports (D11).

**Definition of done:** `cargo test -p ferron-observability-otlp` green with
proto-level assertions on sums, histograms, and exemplars; a manual
`cargo run -p ferron -- validate -c ferron.conf` still passes.

### Step 6 — Integration, teardown, and cleanup

- [ ] `lib.rs`: replace `OtlpProviderCache` (SDK providers) with the three
      pipeline components; keep the event loop + `config_cache_key` +
      sink. Ensure shutdown: cancel token → flush batch exporters and
      reader (reuse the `spawn_blocking` pattern if the drop path deadlocks;
      with a custom implementation this can be a plain async flush + timeout).
- [ ] Delete `src/providers/` (all files) and `src/client.rs`'s
      `opentelemetry_http` impl; keep `HyperOtelClient`/`build_tonic_channel`
      logic (moved to `transport/`).
- [ ] Remove SDK crates from `Cargo.toml`; run `cargo shear`, `cargo clippy
      --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
      `cargo fmt --all --check`.
- [ ] Optional config additions (only if you want user-facing control; each
      needs a directive + validator + docs + changelog):
  - `native_histograms [bool]` (per-signal or global; default `true` for
    parity with the current exponential behavior);
  - `exemplars [bool]` (default `true`);
  - `export_interval`/`export_batch_size` for logs/traces; `read_interval`
    for metrics;
  - `gzip [bool]` (default `false`) for HTTP/gRPC compression.

**Definition of done:** no `opentelemetry*` dependency remains; full
workspace builds, tests, clippy, fmt, and shear pass; a real collector
receives all three signals (manual smoke or e2e).

### Step 7 — E2E tests (`e2e/`) and docs

See §7 for the full test matrix. E2E additions:

- [ ] Extend `e2e/images/otlp/responder.py` (Flask mock, port 4318) to also
      decode metrics (`ExportMetricsServiceRequest`, incl. exponential
      histograms and exemplars) and logs, mirroring the existing
      `/received` JSON shape used by `tests/observability/traces.rs`.
- [ ] Add a gRPC receiver (port 4317) to the mock container (Python
      `grpcio` + generated OTLP services) exposing the same `/received`
      payloads, so gRPC e2e can assert decoded data.
- [ ] New tests (registered in `e2e/Cargo.toml` or under
      `tests/observability/mod.rs`):
  - `test_otlp_metrics_exported` (counters, gauges, histograms; assert
    sum/bucket counts/exemplar trace IDs in the decoded payload);
  - `test_otlp_logs_exported` (modern + legacy style);
  - `test_otlp_http_json_exported` (protocol `http/json`, assert decoded
    via the JSON path);
  - `test_otlp_grpc_exported` (protocol `grpc` on 4317);
  - `test_otlp_exemplars` (assert trace_id/span_id on the metric data point
    match the triggering request);
  - `test_otlp_native_histograms` (assert exponential histogram fields).
- [ ] Docs: `docs/configuration/observability/otlp.md` — remove the exemplar
      limitation note (§1.3), document native histograms + exemplars +
      any new directives; `CHANGELOG.md` entries under **UNRELEASED** for
      each user-visible change (one bullet per commit, Conventional Commits,
      per AGENTS.md).

**Definition of done:** `cd e2e && cargo test` passes with the new tests
(Docker + protoc available), docs reflect reality, changelog updated.

---

## 5. Key correctness details (read before implementing)

### 5.1 IDs and timestamps

- `EventTraceContext.trace_id: [u8; 32]`, `span_id: [u8; 16]` are
  **hex-ASCII** (see `types/observability/src/event.rs:151-158`). Decode to
  16/8 bytes for protobuf; encode to hex for JSON. Validate with `from_hex`-
  style checks and drop events with malformed IDs (never emit zero IDs).
- Events carry no timestamps (D7): assign at ingestion. Span start = time of
  `StartSpan`, end = time of `EndSpan` (or start if missing). Log
  `observed_time_unix_nano = time_unix_nano` = ingestion.
- Gauge data points must carry `time_unix_nano`; sums/histograms also
  `start_time_unix_nano`.

### 5.2 OTLP protocol requirements (from `opentelemetry-proto/docs/specification.md`)

- **gRPC:** unary `Export*ServiceRequest`/`Response`; partial success
  (`rejected_spans`/`rejected_data_points`/`rejected_log_records` +
  `error_message`) → never retry; retryable code table at §4 (line ~314);
  throttling via `RetryInfo.retry_delay`; default port 4317; request ≤ 64 MiB
  recommended.
- **HTTP:** POST; `Content-Type: application/x-protobuf` (binary) or
  `application/json` (JSON); default paths `/v1/traces`, `/v1/metrics`,
  `/v1/logs`; gzip via `Content-Encoding: gzip` (optional, D-later); success
  = 200 with empty/partial-success body; retryable = 429/502/503/504 with
  `Retry-After` honored; other non-2xx = drop; response body cap 4 MiB.
- **JSON deviations** (§5, line ~445): `traceId`/`spanId`/`parentSpanId` hex
  strings (not base64); enums as integers; lowerCamelCase keys; 64-bit ints
  as decimal strings; unknown fields ignored on read (we only write).
- **Empty envelopes** must not be exported.

### 5.3 Metric semantics (from the OTel metrics data model)

- Counter → `Sum` (is_monotonic = true); UpDownCounter → `Sum`
  (is_monotonic = false); Gauge → `Gauge`; Histogram → `ExponentialHistogram`
  (default) or `Histogram` (explicit, opt-in). `AggregationTemporality` =
  `AGGREGATION_TEMPORALITY_CUMULATIVE`.
- Exemplars attach to data points; `filtered_attributes` empty; values match
  the point's numeric type (`as_double`/`as_int`).
- Exponential histogram encoding: `scale`, `base` (2), `positive`/`negative`
  buckets with `offset` + `bucket_counts`, `zero_count`, `min`, `max`, `sum`,
  `count`, `zero_threshold` (0), plus exemplars. See
  `opentelemetry.proto.metrics.v1.ExponentialHistogram` in the submodule.

### 5.4 Concurrency and lifecycle

- The module event loop runs on the secondary (tokio) runtime
  (`runtime.spawn_secondary_task`, `lib.rs:152`); batchers/reader tasks must
  also be tokio tasks tied to the module's `CancellationToken` (recreated on
  every `register_modules` call, see the Prometheus module's pattern in
  `modules/observability-prometheus/src/lib.rs:949-964`).
- Shutdown: cancel token → flush everything with the export timeout (30 s) →
  drop. Do not block the event loop on network I/O.
- `spawn_blocking` was needed only because of SDK shutdown semantics
  (`lib.rs:224`); with the custom pipeline an async flush is expected, but
  keep a fallback note if drop-order issues appear.

### 5.5 Config reload

`config_cache_key` (`lib.rs:253`) already differentiates configs per signal
endpoint/protocol/authorization + service_name + log_style. The pipeline
registry keyed the same way; a changed key creates a fresh pipeline (new
start times, new transport), old one is flushed on shutdown.

### 5.6 Optional directives (only if user-facing control is wanted)

Each requires: `register_directives` entry, validator rules, docs table,
changelog bullet, and — per AGENTS.md — an e2e/unit test. Do not add them all
in one commit.

---

## 6. Risks and open questions

| # | Risk | Mitigation |
| --- | --- | --- |
| R1 | pbjson base64-encodes `bytes`; OTLP JSON needs hex trace/span IDs (D6) | Golden-fixture test in Step 0 using `opentelemetry-proto/examples/*.json`; if post-replacement is too fragile, hand-write `Serialize` for the ~4 ID-bearing messages (`Span`, `Link`, `LogRecord`, `Exemplar`), keep pbjson for everything else. |
| R2 | Building gRPC **server** for tests requires `build_server(true)` | Add a separate test-only build (e.g. second `tonic_prost_build::configure()` invocation gated by `cargo:rerun-if-env-changed` / cfg, or generate server code behind a `ferron-observability-otlp/test-utils` dev feature) — do not ship server code in release. |
| R3 | Exponential histogram scale adaptation complexity | Reuse the math from the Prometheus module's `NativeHistogramConfig(1.1)` and the OTel spec formulas; fuzz with `cargo +nightly fuzz run fuzz_otlp_histogram` if added (see §7). |
| R4 | Backpressure: bounded buffers + drop policy choices | Mirror SDK defaults (drop-newest when queue full, count drops); document the choice in the code (`TODO`/`FIXME` markers per AGENTS.md if partial). |
| R5 | gRPC `authorization` metadata vs. HTTP header | Both already exist today (`providers/cache.rs`); preserve exact behavior (metadata key `authorization`). |
| R6 | The mock collector (`responder.py`) currently decodes traces only | Extend for metrics/logs + gRPC (Step 7); keeps e2e assertions at the decoded-payload level. |
| R7 | Removing SDK crates may shift panic/error surface | New exporter must log provider errors as WARN structured logs with `error.message` (parity: `providers/cache.rs:44-54` and `docs/configuration/observability/otlp.md` Observability section). |

---

## 7. Testing strategy

### 7.1 Unit tests (`modules/observability-otlp/src/`, `#[cfg(test)]`)

| Area | Cases |
| --- | --- |
| Encoding (Step 0/2) | attribute value typing (all variants), hex-ID decode/encode, `log_style` bodies, severity mapping, span kind/status/links, control-plane + baggage promotion, access-log remap table, sanitization. |
| JSON (Step 0) | golden fixtures from `opentelemetry-proto/examples/{trace,metrics,logs}.json` — serialize matching payloads and assert semantic equality (parse fixture, compare); hex traceId/spanId; enums as integers. |
| Transport (Step 1) | protobuf payload roundtrip per protocol; content-type headers; retryable classification (gRPC codes, HTTP statuses); `Retry-After`/`RetryInfo`; partial success; response-size caps; auth metadata/header. |
| Batchers (Step 3/4) | batch-size flush, interval flush, queue-full drop + counter, shutdown flush with timeout, retry-then-drop. |
| Metrics (Step 5) | counters monotonicity, gauge last-value, updown sum, explicit + exponential histogram math (scale/offset/counts/min/max/zero), exemplars (values, ring overwrite, IDs), reader interval/empty-export suppression, start-time reset on new reader. |
| Integration (Step 6) | end-to-end without network: events in → mock transport records exported protos; config-reload keying; shutdown ordering. |

Port every test from `src/tests.rs` that still compiles only against the SDK
(they were written for `SdkMeterProvider`/`SdkTracerProvider`); un-comment the
TODO-marked ones.

### 7.2 E2E (`e2e/`, requires Docker + protoc)

- Mock receiver: extend `e2e/images/otlp/responder.py` (HTTP 4318: logs +
  metrics decode; gRPC 4317 via `grpcio`). `/received` returns decoded spans,
  metrics (sums, exponential/explicit histograms, exemplars), and logs.
- Scenarios (register under `e2e/tests/observability/`):
  1. traces exported (existing, must keep passing)
  2. metrics exported: counters/gauges/histograms + exemplar trace IDs
  3. logs exported (modern and legacy `log_style`)
  4. `http/json` protocol path
  5. `grpc` protocol path (port 4317)
  6. native histograms flag off → explicit buckets honored
  7. cross-plane metadata still present on all signals (existing
     `cross_plane.rs` must keep passing)
- Follow the existing pattern in `tests/observability/traces.rs`
  (container startup retry loop, polling `/received`).

### 7.3 Fuzzing (optional follow-up, `fuzz/`)

- `fuzz_otlp_http_request` (JSON + protobuf encode fuzz — assert no panic,
  output decodes), `fuzz_otlp_histogram` (exponential bucket math
  invariants). Run from `fuzz/` with `cargo +nightly fuzz run <target>`.

### 7.4 Full verification commands (after each step)

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo shear
cd e2e && cargo test
```

---

## 8. Commit plan (Conventional Commits, per AGENTS.md)

Each commit on `develop-3.x`-targeted branch `feat/custom-otlp-exporter`:

1. `chore(observability-otlp): add pbjson build integration and JSON golden tests`
2. `feat(observability-otlp): custom OTLP transport layer (gRPC, HTTP/protobuf, HTTP/JSON)`
3. `feat(observability-otlp): encode events to OTLP proto messages`
4. `feat(observability-otlp): batch trace exporter`
5. `feat(observability-otlp): batch log exporter`
6. `feat(observability-otlp): periodic metric reader with native histograms and exemplars`
7. `refactor(observability-otlp): remove OpenTelemetry SDK dependencies`
8. `test(e2e): OTLP metrics, logs, JSON and gRPC export tests`
9. `docs: OTLP native histograms and exemplars` + `CHANGELOG.md` updates

Steps 4-6 and 8-9 are user-visible features ⇒ each must ship with docs,
changelog, and tests per the repository's mandatory-updates rule. Footer:
`Assisted-by: <agent>:<model>`.

---

## 9. Out of scope (deliberately deferred)

- OTLP profiles signal (`ExportProfilesServiceRequest`) — not in `proto.rs`.
- Delta temporality option.
- Multi-destination exporting (the OTLP spec's Implementation Recommendations)
  — out of scope for the current single-endpoint model.
- `events.json` example parity (Event API is development status).
