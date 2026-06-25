const http = require("http");

const PORT = 3000;

const server = http.createServer((req, res) => {
  //const url = new URL(req.url, `http://localhost:${PORT}`);
  const headers = req.headers;

  console.log(`[${new Date().toISOString()}] ${req.method} ${req.url}`);
  console.log(`  Request headers:`, headers);

  // Default response body
  let body = "OK";

  // X-LiteSpeed-Cache-Control (from X-Test-Cache-Control header)
  const lsCacheControl = headers["x-test-cache-control"];
  if (lsCacheControl) {
    res.setHeader("X-LiteSpeed-Cache-Control", lsCacheControl);
  }

  // X-LiteSpeed-Tag (from X-Test-Tag header)
  const lsTag = headers["x-test-tag"];
  if (lsTag) {
    res.setHeader("X-LiteSpeed-Tag", lsTag);
  }

  // X-LiteSpeed-Vary (from X-Test-Vary header)
  const lsVary = headers["x-test-vary"];
  if (lsVary) {
    res.setHeader("X-LiteSpeed-Vary", lsVary);
  }

  // X-LiteSpeed-Purge (from X-Test-Purge header)
  const lsPurge = headers["x-test-purge"];
  if (lsPurge) {
    res.setHeader("X-LiteSpeed-Purge", lsPurge);
  }

  // LSC-Cookie (from X-Test-LSC-Cookie header)
  const lscCookie = headers["x-test-lsc-cookie"];
  if (lscCookie) {
    res.setHeader("LSC-Cookie", lscCookie);
  }

  // Standard Cache-Control (from X-Test-Upstream-Cache-Control header)
  const cacheControl = headers["x-test-upstream-cache-control"];
  if (cacheControl) {
    res.setHeader("Cache-Control", cacheControl);
  }

  // Custom response body (from X-Test-Body header)
  const testBody = headers["x-test-body"];
  if (testBody) {
    body = testBody;
  }

  // Status code (from X-Test-Status header)
  const statusCode = parseInt(headers["x-test-status"] || "200", 10);

  res.writeHead(statusCode);
  res.end(body);
});

server.listen(PORT, () => {
  console.log(`LSCache backend listening on port ${PORT}`);
});
