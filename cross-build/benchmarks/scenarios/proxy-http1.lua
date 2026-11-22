request = function()
    local paths = { "/proxy/static/1k.txt", "/proxy/static/1m.txt" }
    local path = paths[math.random(#paths)]

    -- Add cache-busting query parameter
    local cache_bust = math.random(100000)

    return wrk.format("GET", path .. "?cb=" .. cache_bust, { ["Connection"] = "keep-alive" }, nil)
end
