const http = require("http");

const PORT = 3000;

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);
  const params = url.searchParams;

  console.log(`[${new Date().toISOString()}] ${req.method} ${req.url}`);
  console.log(`  Parsed params:`, Object.fromEntries(params.entries()));

  // Default response body
  let body = "OK";

  // X-LiteSpeed-Cache-Control
  const lsCacheControl = params.get("ls-cache-control");
  if (lsCacheControl) {
    res.setHeader("X-LiteSpeed-Cache-Control", lsCacheControl);
  }

  // X-LiteSpeed-Tag
  const lsTag = params.get("ls-tag");
  if (lsTag) {
    res.setHeader("X-LiteSpeed-Tag", lsTag);
  }

  // X-LiteSpeed-Vary
  const lsVary = params.get("ls-vary");
  if (lsVary) {
    res.setHeader("X-LiteSpeed-Vary", lsVary);
  }

  // X-LiteSpeed-Purge
  const lsPurge = params.get("ls-purge");
  if (lsPurge) {
    res.setHeader("X-LiteSpeed-Purge", lsPurge);
  }

  // LSC-Cookie
  const lscCookie = params.get("lsc-cookie");
  if (lscCookie) {
    res.setHeader("LSC-Cookie", lscCookie);
  }

  // Standard Cache-Control (can be used alongside or overridden by LS headers)
  const cacheControl = params.get("cache-control");
  if (cacheControl) {
    res.setHeader("Cache-Control", cacheControl);
  }

  // Custom response body
  const responseBody = params.get("body");
  if (responseBody) {
    body = responseBody;
  }

  // Status code
  const statusCode = parseInt(params.get("status") || "200", 10);

  res.writeHead(statusCode);
  res.end(body);
});

server.listen(PORT, () => {
  console.log(`LSCache backend listening on port ${PORT}`);
});
