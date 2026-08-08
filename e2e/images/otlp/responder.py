#!/usr/bin/env python3
"""OTLP mock collector for e2e tests.

Serves:
- HTTP/1.1 OTLP endpoints (/v1/traces, /v1/metrics, /v1/logs) on port 4318,
  accepting both protobuf and OTLP JSON bodies, with optional gzip encoding.
- gRPC OTLP services (TraceService, MetricsService, LogsService) on port 4317.

Decoded payloads and request metadata are exposed through /received as JSON
so the Rust tests can assert on decoded telemetry (see
e2e/tests/observability/*.rs).
"""
import gzip
import json
import threading
from concurrent import futures

import grpc
from flask import Flask, jsonify, request
from opentelemetry.proto.collector.logs.v1 import (
    logs_service_pb2,
    logs_service_pb2_grpc,
)
from opentelemetry.proto.collector.metrics.v1 import (
    metrics_service_pb2,
    metrics_service_pb2_grpc,
)
from opentelemetry.proto.collector.trace.v1 import (
    trace_service_pb2,
    trace_service_pb2_grpc,
)

app = Flask(__name__)

_received = []
_decoded_spans = []
_decoded_metrics = []
_decoded_logs = []
_json_payloads = []
_lock = threading.Lock()


@app.route("/ready", methods=["GET"])
def ready():
    return "OK\n", 200


@app.route("/received", methods=["GET"])
def received():
    with _lock:
        return (
            jsonify(
                {
                    "count": len(_received),
                    "items": list(_received),
                    "spans": list(_decoded_spans),
                    "metrics": list(_decoded_metrics),
                    "logs": list(_decoded_logs),
                    "json_payloads": list(_json_payloads),
                }
            ),
            200,
        )


def _attribute(value):
    if value.HasField("string_value"):
        return value.string_value
    if value.HasField("int_value"):
        return str(value.int_value)
    if value.HasField("bool_value"):
        return str(value.bool_value)
    if value.HasField("double_value"):
        return str(value.double_value)
    if value.HasField("array_value"):
        return [_attribute(v) for v in value.array_value.values]
    return None


def _exemplars(items):
    out = []
    for ex in items:
        value = None
        if ex.HasField("as_int"):
            value = ex.as_int
        elif ex.HasField("as_double"):
            value = ex.as_double
        out.append(
            {
                "trace_id": ex.trace_id.hex(),
                "span_id": ex.span_id.hex(),
                "time_unix_nano": ex.time_unix_nano,
                "value": value,
            }
        )
    return out


def _number_point(point):
    value = None
    if point.HasField("as_int"):
        value = point.as_int
    elif point.HasField("as_double"):
        value = point.as_double
    return {"value": value, "time_unix_nano": point.time_unix_nano}


def _bucket_set(buckets):
    return {
        "offset": buckets.offset,
        "counts": list(buckets.bucket_counts),
    }


def _decode_metrics(data):
    try:
        req = metrics_service_pb2.ExportMetricsServiceRequest()
        req.ParseFromString(data)
    except Exception as e:
        print("error parsing metrics protobuf:", type(e).__name__, e)
        return
    for resource_metrics in req.resource_metrics:
        for scope_metrics in resource_metrics.scope_metrics:
            for metric in scope_metrics.metrics:
                entry = {
                    "name": metric.name,
                    "unit": metric.unit,
                    "kind": None,
                    "points": [],
                }
                if metric.HasField("sum"):
                    entry["kind"] = "sum"
                    entry["is_monotonic"] = metric.sum.is_monotonic
                    entry["aggregation_temporality"] = metric.sum.aggregation_temporality
                    for point in metric.sum.data_points:
                        entry["points"].append(
                            {
                                **_number_point(point),
                                "exemplars": _exemplars(point.exemplars),
                            }
                        )
                elif metric.HasField("gauge"):
                    entry["kind"] = "gauge"
                    for point in metric.gauge.data_points:
                        entry["points"].append(
                            {
                                **_number_point(point),
                                "exemplars": _exemplars(point.exemplars),
                            }
                        )
                elif metric.HasField("exponential_histogram"):
                    entry["kind"] = "exponential_histogram"
                    entry["aggregation_temporality"] = (
                        metric.exponential_histogram.aggregation_temporality
                    )
                    for point in metric.exponential_histogram.data_points:
                        entry["points"].append(
                            {
                                "count": point.count,
                                "sum": point.sum,
                                "min": point.min,
                                "max": point.max,
                                "scale": point.scale,
                                "zero_count": point.zero_count,
                                "positive": _bucket_set(point.positive),
                                "negative": _bucket_set(point.negative),
                                "exemplars": _exemplars(point.exemplars),
                            }
                        )
                elif metric.HasField("histogram"):
                    entry["kind"] = "histogram"
                    entry["aggregation_temporality"] = metric.histogram.aggregation_temporality
                    for point in metric.histogram.data_points:
                        entry["points"].append(
                            {
                                "count": point.count,
                                "sum": point.sum,
                                "min": point.min,
                                "max": point.max,
                                "explicit_bounds": list(point.explicit_bounds),
                                "bucket_counts": list(point.bucket_counts),
                                "exemplars": _exemplars(point.exemplars),
                            }
                        )
                with _lock:
                    _decoded_metrics.append(entry)


def _decode_logs(data):
    try:
        req = logs_service_pb2.ExportLogsServiceRequest()
        req.ParseFromString(data)
    except Exception as e:
        print("error parsing logs protobuf:", type(e).__name__, e)
        return
    for resource_logs in req.resource_logs:
        for scope_logs in resource_logs.scope_logs:
            scope = scope_logs.scope.name if scope_logs.HasField("scope") else ""
            for record in scope_logs.log_records:
                attrs = {}
                for kv in record.attributes:
                    attrs[kv.key] = _attribute(kv.value)
                with _lock:
                    _decoded_logs.append(
                        {
                            "scope": scope,
                            "body": record.body.string_value if record.HasField("body") else None,
                            "severity_text": record.severity_text,
                            "severity_number": record.severity_number,
                            "trace_id": record.trace_id.hex(),
                            "span_id": record.span_id.hex(),
                            "time_unix_nano": record.time_unix_nano,
                            "attributes": attrs,
                        }
                    )


def _decode_spans(data):
    try:
        req = trace_service_pb2.ExportTraceServiceRequest()
        req.ParseFromString(data)
    except Exception as e:
        print("error parsing trace protobuf:", e)
        return
    for resource_spans in req.resource_spans:
        resource_attrs = {
            attr.key: _attribute(attr.value) for attr in resource_spans.resource.attributes
        }
        for scope_spans in resource_spans.scope_spans:
            for span in scope_spans.spans:
                attrs = {
                    attr.key: _attribute(attr.value) for attr in span.attributes
                }
                with _lock:
                    _decoded_spans.append(
                        {
                            "name": span.name,
                            "attributes": attrs,
                            "resource": resource_attrs,
                            "trace_id": span.trace_id.hex(),
                            "span_id": span.span_id.hex(),
                            "parent_span_id": span.parent_span_id.hex(),
                        }
                    )


def _decode_json_spans(payload):
    for resource_spans in payload.get("resourceSpans", []):
        resource_attrs = {}
        for attr in resource_spans.get("resource", {}).get("attributes", []):
            resource_attrs[attr["key"]] = attr["value"].get("stringValue")
        for scope_spans in resource_spans.get("scopeSpans", []):
            for span in scope_spans.get("spans", []):
                with _lock:
                    _decoded_spans.append(
                        {
                            "name": span.get("name"),
                            "attributes": {},
                            "resource": resource_attrs,
                            "trace_id": span.get("traceId", "").lower(),
                            "span_id": span.get("spanId", "").lower(),
                            "parent_span_id": span.get("parentSpanId", "").lower(),
                            "json": True,
                        }
                    )


@app.route("/v1/traces", methods=["POST"])
@app.route("/v1/metrics", methods=["POST"])
@app.route("/v1/logs", methods=["POST"])
def receive():
    try:
        data = request.get_data()
        if request.headers.get("Content-Encoding", "").lower() == "gzip":
            data = gzip.decompress(data)
        with _lock:
            _received.append(
                {
                    "len": len(data),
                    "headers": dict(request.headers),
                    "path": request.path,
                }
            )
        if request.headers.get("Content-Type", "").startswith("application/json"):
            _handle_json(request.path, data)
        elif request.path == "/v1/traces":
            _decode_spans(data)
        elif request.path == "/v1/metrics":
            _decode_metrics(data)
        elif request.path == "/v1/logs":
            _decode_logs(data)
        return "", 200
    except Exception as e:
        import traceback

        traceback.print_exc()
        print("error receiving OTLP payload:", request.path, e)
        return "", 500


def _handle_json(path, data):
    payload = json.loads(data)
    with _lock:
        _json_payloads.append({"path": path, "payload": payload})
    if path == "/v1/traces":
        _decode_json_spans(payload)
    elif path == "/v1/metrics":
        _decode_json_metrics(payload)
    elif path == "/v1/logs":
        _decode_json_logs(payload)


def _decode_json_metrics(payload):
    for resource_metrics in payload.get("resourceMetrics", []):
        for scope_metrics in resource_metrics.get("scopeMetrics", []):
            for metric in scope_metrics.get("metrics", []):
                with _lock:
                    _decoded_metrics.append({"name": metric.get("name"), "kind": "json", "points": []})


def _decode_json_logs(payload):
    for resource_logs in payload.get("resourceLogs", []):
        for scope_logs in resource_logs.get("scopeLogs", []):
            scope = scope_logs.get("scope", {}).get("name")
            for record in scope_logs.get("logRecords", []):
                with _lock:
                    _decoded_logs.append(
                        {
                            "scope": scope,
                            "body": record.get("body", {}).get("stringValue"),
                            "severity_text": record.get("severityText"),
                            "severity_number": record.get("severityNumber"),
                            "trace_id": record.get("traceId", "").lower(),
                            "span_id": record.get("spanId", "").lower(),
                            "attributes": {},
                            "json": True,
                        }
                    )


class _TraceServicer(trace_service_pb2_grpc.TraceServiceServicer):
    def Export(self, request, context):
        with _lock:
            _received.append({"len": request.ByteSize(), "path": "grpc/v1/traces"})
        _decode_spans(request.SerializeToString())
        return trace_service_pb2.ExportTraceServiceResponse()


class _MetricsServicer(metrics_service_pb2_grpc.MetricsServiceServicer):
    def Export(self, request, context):
        with _lock:
            _received.append({"len": request.ByteSize(), "path": "grpc/v1/metrics"})
        _decode_metrics(request.SerializeToString())
        return metrics_service_pb2.ExportMetricsServiceResponse()


class _LogsServicer(logs_service_pb2_grpc.LogsServiceServicer):
    def Export(self, request, context):
        with _lock:
            _received.append({"len": request.ByteSize(), "path": "grpc/v1/logs"})
        _decode_logs(request.SerializeToString())
        return logs_service_pb2.ExportLogsServiceResponse()


def serve_grpc():
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=8))
    trace_service_pb2_grpc.add_TraceServiceServicer_to_server(_TraceServicer(), server)
    metrics_service_pb2_grpc.add_MetricsServiceServicer_to_server(_MetricsServicer(), server)
    logs_service_pb2_grpc.add_LogsServiceServicer_to_server(_LogsServicer(), server)
    server.add_insecure_port("0.0.0.0:4317")
    server.start()
    server.wait_for_termination()


if __name__ == "__main__":
    threading.Thread(target=serve_grpc, daemon=True).start()
    app.run(host="0.0.0.0", port=4318, threaded=True)