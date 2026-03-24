const express = require("express");
const app = express();
const expressWs = require("express-ws")(app);
const https = require("https");
const http = require("http");
const fs = require("fs");

app.enable("trust proxy");

app.get("/", (_req, res, _next) => {
  res.send("Hello, World!");
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

app.ws("/echo", (ws, _req) => {
  ws.on("message", (msg) => {
    ws.send(msg);
  });
});

try {
  // NOTE: No WebSocket support when using `https.createServer({...}, app).listen(<port>)`...
  // This isn't a problem, as TLS backend tests are just basic reverse proxying tests for now...
  https
    .createServer(
      {
        key: fs.readFileSync("/etc/certs/server.key"),
        cert: fs.readFileSync("/etc/certs/server.crt"),
      },
      app,
    )
    .listen(3001);
} catch (error) {
  // Probably the certificate didn't load...
  console.error("Failed to start HTTPS server:", error);
}

app.listen(3000);
