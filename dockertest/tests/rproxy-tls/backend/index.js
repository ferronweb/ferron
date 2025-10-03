const express = require("express");
const https = require("https");
const fs = require("fs");
const app = express();

app.get("/", (_req, res, _next) => {
  res.send("Hello, World!");
});

https
  .createServer(
    {
      key: fs.readFileSync("/etc/certs/server.key"),
      cert: fs.readFileSync("/etc/certs/server.crt"),
    },
    app,
  )
  .listen(3000);
