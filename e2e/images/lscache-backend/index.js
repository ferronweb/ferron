const http = require("http");

const PORT = 3000;

const server = http.createServer((req, res) => {
  //const url = new URL(req.url, `http://localhost:${PORT}`);
  const headers = req.headers;

  console.log(`[${new Date().toISOString()}] ${req.method} ${req.url}`);
  console.log(`  Request headers:`, headers);

  // Default response body
  let body = "OK";

  const lsCacheControl = headers["x-test-cache-control"];
  if (lsCacheControl) {
    res.setHeader("X-LiteSpeed-Cache-Control", lsCacheControl);
  }

  const lsTag = headers["x-test-tag"];
  if (lsTag) {
    res.setHeader("X-LiteSpeed-Tag", lsTag);
  }

  const lsVary = headers["x-test-vary"];
  if (lsVary) {
    res.setHeader("X-LiteSpeed-Vary", lsVary);
  }

  const lsPurge = headers["x-test-purge"];
  if (lsPurge) {
    res.setHeader("X-LiteSpeed-Purge", lsPurge);
  }

  const lscCookie = headers["x-test-lsc-cookie"];
  if (lscCookie) {
    res.setHeader("LSC-Cookie", lscCookie);
  }

  const cacheControl = headers["x-test-upstream-cache-control"];
  if (cacheControl) {
    res.setHeader("Cache-Control", cacheControl);
  }

  // Arbitrary response headers (from X-Test-Headers, pipe-separated "Name: Value" pairs)
  const testHeaders = headers["x-test-headers"];
  if (testHeaders) {
    for (const pair of testHeaders.split("|")) {
      const colon = pair.indexOf(":");
      if (colon !== -1) {
        res.setHeader(
          pair.slice(0, colon).trim(),
          pair.slice(colon + 1).trim(),
        );
      }
    }
  }

  const testBody = headers["x-test-body"];
  if (testBody) {
    body = testBody;
  }

  const statusCode = parseInt(headers["x-test-status"] || "200", 10);

  res.writeHead(statusCode);
  res.end(body);
});

server.listen(PORT, () => {
  console.log(`LSCache backend listening on port ${PORT}`);
});
