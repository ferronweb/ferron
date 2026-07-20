-- wrk Lua script for large static file benchmarking
-- Requests large (1MB) static files with path randomization

request = function()
    local paths = { "/static/1m.txt" }
    local path = paths[math.random(#paths)]

    -- Add cache-busting query parameter
    local cache_bust = math.random(100000)

    return wrk.format("GET", path .. "?cb=" .. cache_bust, { ["Connection"] = "keep-alive" }, nil)
end
