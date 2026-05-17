#!/usr/bin/env python3
import signal
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer


class HangHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        # Read a small prefix (if Content-Length present) then hang to simulate sink that never responds
        try:
            length = int(self.headers.get("Content-Length", 0) or 0)
            if length:
                _ = self.rfile.read(min(length, 1024))
        except Exception:
            pass
        print(f"Hanging on {self.path} from {self.client_address}", flush=True)
        try:
            while True:
                time.sleep(3600)
        except KeyboardInterrupt:
            pass

    def log_message(self, format, *args):
        # Minimal logging to stdout
        sys.stdout.write(
            "%s - - [%s] %s\n"
            % (self.client_address[0], self.log_date_time_string(), format % args)
        )
        sys.stdout.flush()


def run(port=4318):
    server = HTTPServer(("0.0.0.0", port), HangHandler)

    def shutdown(signum, frame):
        print("Shutting down OTLP hang server", flush=True)
        server.shutdown()

    signal.signal(signal.SIGINT, shutdown)
    signal.signal(signal.SIGTERM, shutdown)
    print(f"Starting OTLP hang server on port {port}")
    server.serve_forever()


if __name__ == "__main__":
    import argparse

    p = argparse.ArgumentParser()
    p.add_argument("--port", "-p", type=int, default=4318)
    args = p.parse_args()
    run(port=args.port)
