const http = require("http");

const PORT = 9090;
const received = [];

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);

  if (req.method === "POST" && url.pathname === "/cache/purge") {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      try {
        const data = JSON.parse(body);
        received.push(data);
        console.log(
          `[${new Date().toISOString()}] Received purge:`,
          JSON.stringify(data),
        );
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ status: "ok" }));
      } catch (_e) {
        res.writeHead(400);
        res.end("Invalid JSON");
      }
    });
  } else if (req.method === "GET" && url.pathname === "/received") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify(received));
  } else if (req.method === "DELETE" && url.pathname === "/received") {
    received.length = 0;
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ status: "cleared" }));
  } else {
    res.writeHead(404);
    res.end("Not Found");
  }
});

server.listen(PORT, () => {
  console.log(`Mock control-plane listening on port ${PORT}`);
});
