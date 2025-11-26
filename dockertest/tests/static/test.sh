#!/bin/bash

TEST_FAILED=0

# Wait for the HTTP server to start
for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z ferron 80 >/dev/null 2>&1 && break || true
done

TEST_RESULTS="$(curl -fsSL http://ferron/basic.txt)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/basic.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Basic static file serving test passed!"
else
    echo "Basic static file serving test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://ferron/unicode.txt)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/unicode.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Unicode static file serving test passed!"
else
    echo "Unicode static file serving test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -H 'Accept-Encoding: gzip' http://ferron/basic.txt | gunzip)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/basic.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with gzip compression test passed!"
else
    echo "Static file serving with gzip compression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -H 'Accept-Encoding: deflate' http://ferron/basic.txt > /tmp/results.txt && \
                printf "\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\x00" | cat - /tmp/results.txt | (gzip -dc 2>/dev/null || true))"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/basic.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with Deflate compression test passed!"
else
    echo "Static file serving with Deflate compression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -H 'Accept-Encoding: br' http://ferron/basic.txt | brotli -d)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/basic.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with Brotli compression test passed!"
else
    echo "Static file serving with Brotli compression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -H 'Accept-Encoding: zstd' http://ferron/basic.txt | unzstd)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/basic.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with Zstandard compression test passed!"
else
    echo "Static file serving with Zstandard compression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -H 'Accept-Encoding: gzip' http://ferron/precompressed/basic.txt | hexdump -C)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/precompressed/basic.txt.gz | hexdump -C)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with precompression test passed!"
else
    echo "Static file serving with precompression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -H 'Range: bytes=0-11' http://ferron/basic.txt)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/basic.txt | head -c 12)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Partial static file serving test passed!"
else
    echo "Partial static file serving test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -w %{http_code} -H "If-None-Match: $(curl -fsSLI http://ferron/basic.txt | grep -i 'etag:' | cut -d ':' -f 2 | tr -d '[:space:]')" http://ferron/basic.txt)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="304"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "ETag test passed!"
else
    echo "ETag test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -w %{http_code} -H "Accept-Encoding: gzip" -H "If-None-Match: $(curl -fsSLI -H "Accept-Encoding: gzip" http://ferron/basic.txt | grep -i 'etag:' | cut -d ':' -f 2 | tr -d '[:space:]')" http://ferron/basic.txt)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="304"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "ETag with gzip compression test passed!"
else
    echo "ETag with gzip compression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsL -w %{http_code} --path-as-is http://ferron/../../../../../../../../etc/passwd || true)"
TEST_EXIT_CODE=$?
TEST_UNEXPECTED="200"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" != "$TEST_UNEXPECTED" ]; then
    echo "Path traversal prevention test passed!"
else
    echo "Path traversal prevention test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Unexpected: $TEST_UNEXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsL -w %{http_code} --path-as-is http://ferron/..\\..\\..\\..\\..\\..\\..\\..\\etc/passwd || true)"
TEST_EXIT_CODE=$?
TEST_UNEXPECTED="200"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" != "$TEST_UNEXPECTED" ]; then
    echo "Path traversal prevention test (with \\) passed!"
else
    echo "Path traversal prevention test (with \\) failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Unexpected: $TEST_UNEXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsL -w %{http_code} --path-as-is http://ferron/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/etc/passwd || true)"
TEST_EXIT_CODE=$?
TEST_UNEXPECTED="200"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" != "$TEST_UNEXPECTED" ]; then
    echo "Path traversal prevention test (with %2e%2e/) passed!"
else
    echo "Path traversal prevention test (with %2e%2e/) failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Unexpected: $TEST_UNEXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsL -w %{http_code} --path-as-is http://ferron/%2e%2e%c0%af%2e%2e%c0%af%2e%2e%c0%af%2e%2e%c0%af%2e%2e%c0%af%2e%2e%c0%af%2e%2e%c0%af%2e%2e%c0%afetc/passwd || true)"
TEST_EXIT_CODE=$?
TEST_UNEXPECTED="200"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" != "$TEST_UNEXPECTED" ]; then
    echo "Path traversal prevention test (with %c0%af) passed!"
else
    echo "Path traversal prevention test (with %c0%af) failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Unexpected: $TEST_UNEXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

curl -fsSLI http://ferron/basic.txt > /dev/null
TEST_EXIT_CODE=$?
if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo "HEAD request test passed!"
else
    echo "HEAD request test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsL -w %{http_code} http://ferron/doesntexist.txt || true)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="404"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "404 Not Found error test passed!"
else
    echo "404 Not Found error test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsL -o /dev/null -w %{http_code} http://ferron/dirlisting || true)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="200"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Directory listing test passed!"
else
    echo "Directory listing test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsL -w %{http_code} http://ferron/dirnolisting || true)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="403"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Directory listing forbidden test passed!"
else
    echo "Directory listing forbidden test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://ferron/)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/basic.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with custom index file test passed!"
else
    echo "Static file serving with custom index file test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
