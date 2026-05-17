#!/usr/bin/env python3
import signal
import socketserver
import sys
import time
import urllib.parse


class ChaoticHandler(socketserver.StreamRequestHandler):
    def handle(self):
        try:
            # Read request line
            request_line = self.rfile.readline().decode("latin-1")
            if not request_line:
                return
            parts = request_line.split()
            if len(parts) < 2:
                return
            path = parts[1]
            # Drain headers
            while True:
                h = self.rfile.readline().decode("latin-1")
                if h in ("\r\n", "\n", ""):
                    break
            parsed = urllib.parse.urlparse(path)
            if parsed.path == "/slow":
                q = urllib.parse.parse_qs(parsed.query)
                delay = int(q.get("delay", ["10"])[0])
                print(f"Simulating slow backend: sleep {delay}s", flush=True)
                time.sleep(delay)
                body = b"OK SLOW\n"
                self.request.sendall(
                    b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n" % len(body) + body
                )
            elif parsed.path == "/close_mid":
                print("Simulating close mid-response", flush=True)
                # Send headers and partial body, then close
                self.request.sendall(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nPartialData"
                )
                try:
                    self.request.shutdown(1)
                except Exception:
                    pass
                self.request.close()
            elif parsed.path == "/partial_headers":
                print("Simulating partial headers (slowloris-like)", flush=True)
                headers = b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nContent-Type: text/plain\r\n\r\n"
                for b in headers:
                    self.request.sendall(bytes([b]))
                    time.sleep(0.05)
                self.request.sendall(b"hello world\n")
            elif parsed.path == "/malformed_chunked":
                print("Simulating malformed chunked encoding", flush=True)
                self.request.sendall(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
                )
                self.request.sendall(b"ZZ\r\nnot-chunked\r\n0\r\n\r\n")
            else:
                body = b"OK\n"
                self.request.sendall(
                    b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n" % len(body) + body
                )
        except Exception as e:
            print("Bad backend handler error:", e, flush=True)
            return


def run(host="0.0.0.0", port=8000):
    server = socketserver.ThreadingTCPServer((host, port), ChaoticHandler)
    server.allow_reuse_address = True

    def shutdown(signum, frame):
        print("Shutting down bad backend", flush=True)
        server.shutdown()
        sys.exit(0)

    signal.signal(signal.SIGINT, shutdown)
    signal.signal(signal.SIGTERM, shutdown)
    print(f"Bad backend listening on {host}:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    import argparse

    p = argparse.ArgumentParser()
    p.add_argument("--port", type=int, default=8000)
    args = p.parse_args()
    run(port=args.port)
