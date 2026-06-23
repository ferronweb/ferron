const express = require("express");
const http2 = require("http2");
const http2Express = require("http2-express");
const fs = require("fs");

const app = http2Express(express);
require("express-ws")(app);

app.enable("trust proxy");

const backendName = process.env.BACKEND_NAME || "backend";
let unstableFailuresRemaining = Number.parseInt(
  process.env.UNSTABLE_FAILS || "0",
  10,
);

app.get("/version", (req, res, _next) => {
  res.send(req.httpVersion);
});

app.get("/", (_req, res, _next) => {
  res.send("Hello, World!");
});

app.get("/whoami", (_req, res, _next) => {
  res.send(backendName);
});

app.get("/unstable", (req, res, _next) => {
  const sleep = Number.parseInt(req.query.sleep || "0", 10);
  if (sleep > 0) {
    setTimeout(() => {
      res.send(backendName);
    }, sleep);
    return;
  }

  if (unstableFailuresRemaining > 0) {
    unstableFailuresRemaining -= 1;
    res.status(503).send(`unstable:${backendName}`);
    return;
  }

  res.send(backendName);
});

app.get("/ip", (req, res, _next) => {
  res.send(req.ip);
});

app.get("/hostname", (req, res, _next) => {
  res.send(req.headers.host);
});

app.get("/header", (req, res, _next) => {
  res.send(req.headers["x-some-header"]);
});

app.get("/unsafe", (req, _res, _next) => {
  req.socket.destroy();
});

app.get("/tls", (req, res, _next) => {
  if (req.socket.encrypted) {
    res.send("Hello, World!");
  } else {
    res.status(403).send("Not TLS");
  }
});

// Cache revalidation test endpoint — tracks version state per resource id
const cacheVersions = {};

app.get("/cache-etag", (req, res) => {
  const id = req.query.id || "default";
  const version = cacheVersions[id] || 1;
  const etag = `W/"v${version}"`;
  const lastModified = new Date(Date.UTC(2024, 0, version)).toUTCString();

  if (req.headers["if-none-match"] === etag) {
    return res.status(304).set("ETag", etag).set("Date", new Date().toUTCString()).end();
  }

  res
    .status(200)
    .set("ETag", etag)
    .set("Last-Modified", lastModified)
    .set("Cache-Control", "public, max-age=300")
    .send(`v${version}`);
});

app.post("/cache-etag/update", express.json(), (req, res) => {
  const id = req.query.id || "default";
  cacheVersions[id] = (cacheVersions[id] || 1) + 1;
  const newVersion = cacheVersions[id];
  res.send(`updated to v${newVersion}`);
});

app.ws("/echo", (ws, _req) => {
  ws.on("message", (msg) => {
    ws.send(msg);
  });
});

try {
  // NOTE: No WebSocket support when using `https.createServer({...}, app).listen(<port>)`...
  // This isn't a problem, as TLS backend tests are just basic reverse proxying tests for now...
  http2
    .createSecureServer(
      {
        key: fs.readFileSync("/etc/certs/server.key"),
        cert: fs.readFileSync("/etc/certs/server.crt"),
        allowHTTP1: true,
      },
      app,
    )
    .listen(3001);
} catch (error) {
  // Probably the certificate didn't load...
  console.error("Failed to start HTTPS server:", error);
}

app.listen(3000);
