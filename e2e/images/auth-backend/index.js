const express = require("express");
const app = express();

app.all("/auth/ok", (_req, res) => {
  res.set("X-Auth-User", "dorian");
  res.set("X-Auth-Roles", "admin,user");
  res.set("X-Auth-Email", "dorian@example.com");
  res.status(200).send("OK");
});

app.all("/auth/fail", (_req, res) => {
  res.status(401).send("Unauthorized");
});

app.all("/auth/forbidden", (_req, res) => {
  res.status(403).send("Forbidden");
});

app.all("/auth/500", (_req, res) => {
  res.status(500).send("Internal Error");
});

app.all("/auth/slow", (_req, res) => {
  setTimeout(() => res.status(200).send("OK"), 30000);
});

app.all("/auth/echo", (req, res) => {
  res.json({
    method: req.method,
    path: req.path,
    query: req.query,
    headers: req.headers,
  });
});

app.all("/auth/malformed", (_req, res) => {
  res.socket.write("HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n");
  res.socket.end();
});

app.all("/echo-headers", (req, res) => {
  res.json(req.headers);
});

app.get("/", (_req, res) => {
  res.send("Hello from auth backend!");
});

app.all("*", (_req, res) => {
  res.send("Hello from auth backend!");
});

app.listen(9090);
