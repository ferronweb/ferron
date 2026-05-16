#!/usr/bin/env python3
from flask import Flask, request, jsonify

app = Flask(__name__)

# Keep received payload metadata in memory for test inspection
_received = []

@app.route('/ready', methods=['GET'])
def ready():
    return "OK\n", 200

@app.route('/received', methods=['GET'])
def received():
    return jsonify({"count": len(_received)}), 200

@app.route('/v1/traces', methods=['POST'])
@app.route('/v1/metrics', methods=['POST'])
@app.route('/v1/logs', methods=['POST'])
def receive():
    try:
        data = request.get_data()
        # store minimal metadata to keep memory usage small
        _received.append({"len": len(data), "headers": dict(request.headers)})
        return "", 200
    except Exception as e:
        print("error receiving OTLP payload:", e)
        return "", 500

if __name__ == '__main__':
    # Bind to 0.0.0.0 so the container can be reached from other containers
    app.run(host='0.0.0.0', port=4318)
