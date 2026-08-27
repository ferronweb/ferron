request = function()
    local paths = { "/static/1k.txt" }
    local path = paths[math.random(#paths)]

    return wrk.format("GET", path .. "?cache=1", { ["Connection"] = "keep-alive" }, nil)
end
