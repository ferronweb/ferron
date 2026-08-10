#!/usr/bin/env python3
"""StatsD mock receiver for e2e tests.

Listens for UDP StatsD datagrams on port 8125 and serves the collected
datagrams through the /received HTTP endpoint on port 8080 so the Rust tests
can assert on the emitted metrics (see e2e/tests/observability/statsd.rs).
"""
import socket
import threading

from flask import Flask, jsonify

app = Flask(__name__)

_received = []
_lock = threading.Lock()

UDP_PORT = 8125


def udp_listener():
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("0.0.0.0", UDP_PORT))
    while True:
        data, _ = sock.recvfrom(65535)
        with _lock:
            _received.append(data.decode("utf-8", errors="replace"))


@app.route("/ready", methods=["GET"])
def ready():
    return "OK\n", 200


@app.route("/received", methods=["GET"])
def received():
    with _lock:
        items = list(_received)
    return jsonify({"count": len(items), "items": items}), 200


@app.route("/clear", methods=["POST"])
def clear():
    with _lock:
        _received.clear()
    return "OK\n", 200


if __name__ == "__main__":
    threading.Thread(target=udp_listener, daemon=True).start()
    app.run(host="0.0.0.0", port=8080)
