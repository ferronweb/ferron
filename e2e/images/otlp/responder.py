#!/usr/bin/env python3
from flask import Flask, jsonify, request
from opentelemetry.proto.collector.trace.v1.trace_service_pb2 import (
    ExportTraceServiceRequest,
)

app = Flask(__name__)

# Keep received payload metadata and decoded spans in memory
_received = []
_decoded_spans = []


@app.route("/ready", methods=["GET"])
def ready():
    return "OK\n", 200


@app.route("/received", methods=["GET"])
def received():
    return jsonify(
        {
            "count": len(_received),
            "items": _received,
            "spans": _decoded_spans,
        }
    ), 200


def _decode_spans(data):
    """Decode an ExportTraceServiceRequest protobuf and extract span info."""
    try:
        req = ExportTraceServiceRequest()
        req.ParseFromString(data)
        for resource_spans in req.resource_spans:
            # Extract resource attributes
            resource_attrs = {}
            for attr in resource_spans.resource.attributes:
                key = attr.key
                val = attr.value
                if val.string_value:
                    resource_attrs[key] = val.string_value
                elif val.int_value:
                    resource_attrs[key] = str(val.int_value)
                elif val.bool_value:
                    resource_attrs[key] = str(val.bool_value)
                elif val.double_value:
                    resource_attrs[key] = str(val.double_value)
            for scope_spans in resource_spans.scope_spans:
                for span in scope_spans.spans:
                    attrs = {}
                    for attr in span.attributes:
                        key = attr.key
                        val = attr.value
                        if val.string_value:
                            attrs[key] = val.string_value
                        elif val.int_value:
                            attrs[key] = str(val.int_value)
                        elif val.bool_value:
                            attrs[key] = str(val.bool_value)
                        elif val.double_value:
                            attrs[key] = str(val.double_value)
                    _decoded_spans.append(
                        {
                            "name": span.name,
                            "attributes": attrs,
                            "resource": resource_attrs,
                        }
                    )
    except Exception as e:
        print("error decoding trace protobuf:", e)


@app.route("/v1/traces", methods=["POST"])
@app.route("/v1/metrics", methods=["POST"])
@app.route("/v1/logs", methods=["POST"])
def receive():
    try:
        data = request.get_data()
        _received.append(
            {
                "len": len(data),
                "headers": dict(request.headers),
                "path": request.path,
            }
        )
        if request.path == "/v1/traces":
            _decode_spans(data)
        return "", 200
    except Exception as e:
        print("error receiving OTLP payload:", e)
        return "", 500


if __name__ == "__main__":
    app.run(host="0.0.0.0", port=4318)
