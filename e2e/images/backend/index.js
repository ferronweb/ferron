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
    return res
      .status(304)
      .set("ETag", etag)
      .set("Date", new Date().toUTCString())
      .end();
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

// Cache stale-while-revalidate test endpoints
const swrVersions = {};
const swrFetchCounts = {};

app.get("/cache-swr", (req, res) => {
  const id = req.query.id || "default";
  const version = swrVersions[id] || 1;
  const fetchCount = (swrFetchCounts[id] || 0) + 1;
  swrFetchCounts[id] = fetchCount;

  // First fetch: set error flag so subsequent requests fail until version bumped
  if (fetchCount === 1) {
    swrFetchCounts[id] = 1;
  }

  res
    .status(200)
    .set("Cache-Control", "public, max-age=1, stale-while-revalidate=60")
    .set("X-Backend-Version", String(version))
    .set("X-Backend-Fetch-Count", String(fetchCount))
    .send(`swr-v${version}`);
});

app.post("/cache-swr/update", express.json(), (req, res) => {
  const id = req.query.id || "default";
  swrVersions[id] = (swrVersions[id] || 1) + 1;
  swrFetchCounts[id] = 0;
  res.send(`swr updated to v${swrVersions[id]}`);
});

// Cache stale-if-error test endpoints
const sieErrorMode = {};
const sieVersions = {};

app.get("/cache-sie", (req, res) => {
  const id = req.query.id || "default";
  const version = sieVersions[id] || 1;

  if (sieErrorMode[id]) {
    return res.status(503).send(`sie-error:${id}`);
  }

  res
    .status(200)
    .set("Cache-Control", "public, max-age=300, stale-if-error=60")
    .set("X-Backend-Version", String(version))
    .send(`sie-v${version}`);
});

app.post("/cache-sie/error", express.json(), (req, res) => {
  const id = req.query.id || "default";
  sieErrorMode[id] = true;
  res.send(`sie error mode enabled for ${id}`);
});

app.post("/cache-sie/recover", express.json(), (req, res) => {
  const id = req.query.id || "default";
  sieErrorMode[id] = false;
  res.send(`sie error mode disabled for ${id}`);
});

app.post("/cache-sie/update", express.json(), (req, res) => {
  const id = req.query.id || "default";
  sieVersions[id] = (sieVersions[id] || 1) + 1;
  sieErrorMode[id] = false;
  res.send(`sie updated to v${sieVersions[id]}`);
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
