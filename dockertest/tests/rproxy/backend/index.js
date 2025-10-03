const express = require("express");
const app = express();
const expressWs = require("express-ws")(app);

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

app.ws("/echo", (ws, _req) => {
  ws.on("message", (msg) => {
    ws.send(msg);
  });
});

app.listen(3000);
