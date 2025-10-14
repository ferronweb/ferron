const express = require("express");
const app = express();

app.enable("trust proxy");

app.get("/", (_req, res, _next) => {
  res.send("Hello, World!");
});

app.listen(3000);
