-- wrk Lua script for small static file benchmarking
-- Requests small (1KB) static files with path randomization

request = function()
    local paths = { "/static/1k.txt" }
    local path = paths[math.random(#paths)]

    -- Add cache-busting query parameter
    local cache_bust = math.random(100000)

    return wrk.format("GET", path .. "?cb=" .. cache_bust, { ["Connection"] = "keep-alive" }, nil)
end
