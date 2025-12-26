---
title: Rhai script module
---

The **script-exec** module lets you attach [Rhai](https://rhai.rs) programs to Ferron’s request pipeline. A script can inspect or mutate the inbound request, adjust the generated response, maintain shared state across workers, or even schedule background work, making it possible to express lots of “edge logic” without recompiling Ferron.

This page describes how script execution is wired into the server, how to configure scripts in `ferron.kdl`, and which APIs are available from Rhai.

## When scripts run

Every script declares one or more *triggers*. Triggers map to server phases:

| Trigger                | Script phase        | When it runs                                                                                                   |
|------------------------|--------------------|-----------------------------------------------------------------------------------------------------------------|
| `on_request_start`     | `RequestStart`     | Before Ferron passes the request to the rest of the module stack. You can rewrite method/URI/headers/body here. |
| `on_request_body`      | `RequestBody`      | After the body is buffered. Only scripts that request this trigger force buffering.                             |
| `on_response_ready`    | `Response`         | After Ferron has received the response from the downstream module but before bytes are sent to the client.      |
| `on_tick`              | `Tick`             | Fired periodically (default `1s`, configurable per script via `tick_interval`).                                 |
| `spawn_task(...)`      | `BackgroundTask`   | Runs asynchronously when a script explicitly spawns a task.                                                    |

Each trigger produces a [`ScriptPhase`](../ferron-script/src/context.rs) under the hood, so host helpers such as `set_header` automatically act on either the request or the response depending on where the script is running.

## Quick start configuration

Enable the module inside a host block and add one or more `script` entries:

```kdl
:8080 {
  module "script-exec" {
    script "auth-check" {
      file "scripts/auth.rhai"
      trigger "on_request_start"
      trigger "on_response_ready"
      env {
        shared_secret "super-secret"
      }
      allow "spawn_task"
      limits {
        max_operations 300000     // default 200_000
        max_exec_time "75ms"      // default 50ms
      }
      tick_interval "5s"
      failure_policy "block"
    }
  }

  root "wwwroot"
}
```

You may list multiple `script` blocks; each block’s first value (`"auth-check"` above) is the script identifier that appears in logs.

### Script block reference

| Directive            | Description |
|----------------------|-------------|
| `file "<path>"`      | Required. Path to the Rhai file. File-based scripts automatically reload when the source file changes unless you set `reload_on_change false`. |
| `trigger "<name>"`   | Required at least once. One of `on_request_start`, `on_request_body`, `on_response_ready`, or `on_tick`. Multiple triggers are allowed. |
| `tick_interval "<duration>"` | Optional. Overrides the default `1s` tick cadence when `on_tick` is enabled. Uses `humantime` durations (`"250ms"`, `"5s"`, etc.). |
| `env { ... }`        | Optional key/value map injected into the script as the `env` object. |
| `allow "spawn_task"` | Grants access to `spawn_task`. Omit to disable background work for this script. |
| `reload_on_change <bool>` | Optional. Defaults to `true` for file-backed scripts; set to `false` to pin to the compiled AST until Ferron restarts. |
| `limits { ... }`     | Optional guardrails. Supports `max_operations` (default `200_000`), `max_call_depth` (default `32`), and `max_exec_time` (default `50ms`). |
| `failure_policy "<mode>"` | `block` (default) propagates an error to the HTTP stack, `skip` converts failures into “continue” decisions. |
| `allow { ... }`      | Helper block to group permission entries (currently only `spawn_task`). |

> **Note**  
> Scripts that register `on_request_body` force Ferron to buffer the entire request body so Rhai can read or modify it. Avoid adding that trigger unless the script truly needs the body—streaming throughput drops if every request must be buffered.

## Runtime APIs available to Rhai

Each script sees a pre-populated scope:

| Name        | Type                   | Purpose |
|-------------|------------------------|---------|
| `request`   | `Request` handle       | Allows you to read or mutate `method`, `uri`, `body`, and individual headers during request-side phases. |
| `response`  | `Response` handle      | When running during `on_response_ready`, represents the downstream response. Scripts can edit `status`, `headers`, or `body`. |
| `env`       | `Map`                  | The immutable key/value map from the configuration’s `env` block. |
| `state`     | `StateStore`           | A synchronized map shared across all scripts and workers (`get`, `set`, `remove`, `clear`, `keys`). Useful for counters or caches. |

Helpers registered via `rhai::Engine` provide the rest of the integration surface:

- `log(level, message)` queues log lines that Ferron writes once the script finishes (`level` is an arbitrary string such as `"info"` or `"debug"`).
- `set_header(name, value)` and `remove_header(name)` adjust headers on the *current* phase (request vs. response) without manually switching handles.
- `deny(status, body)` stops the pipeline and instructs Ferron to return `status`/`body` immediately. The script’s `failure_policy` isn’t consulted because this is an explicit decision, not an error.
- `spawn_task(name, fn_ptr)` schedules background work. The function pointer must reference another Rhai function; the background worker inherits the script’s `env`, `state`, and failure policy. Tasks obey the same execution limits and log forwarding as the main script.

In addition, the `request` and `response` handles expose idiomatic getters/setters for headers and bodies, so you can write expressive Rhai like:

```rhai
if request.header("x-api-key").is_none() {
  deny(401, "missing key");
}

response.set_header("x-script-version", "1.0");
log("debug", `handled ${request.uri}`);
```

## Background work and scheduled tasks

`on_tick` triggers are tied to Ferron’s scheduler: when configured, the script executes on every tick interval even if no requests arrive. This is helpful for cache warm-up or periodic housekeeping. The tick handler uses the same Rhai file and scope setup as other triggers, but the `request` handle is absent because no HTTP exchange caused the invocation.

`spawn_task` allows scripts to dispatch extra async work (for example, to refresh an external ACL) without blocking the request thread. Tasks run through the same sandbox:

- They observe the script’s `limits`.
- Timeouts or panics count toward the script’s failure counter.
- Logs emitted inside the task flow into the main error logger along with the task name.

Because background work also runs on the Tokio runtime via `block_in_place`, you should keep those functions short and avoid long sleeps. Prefer scheduling periodic logic via `on_tick` if you don’t need ad-hoc spawns.

## Failure handling, throttling, and logging

Each script instance tracks its own `failure_state`. Consecutive runtime errors (panics, exceeded limits, compile failures) trip a breaker after **five** failures. Once tripped, the script is temporarily disabled; Ferron logs a message and continues to skip that script until it successfully runs again.

- `failure_policy "block"` propagates the error back to the module chain (usually returning a 500 unless another module overrides the response).
- `failure_policy "skip"` treats failures as “continue” decisions so the HTTP request proceeds, but the incident is still logged and the breaker counter increments.

Use the `limits` block to prevent runaway scripts: `max_operations` guards Rhai’s internal operation counter, `max_call_depth` protects the call stack, and `max_exec_time` wraps the whole script in a Tokio timeout. Choose tighter values in production to minimize blast radius.

Ferron buffers all log messages produced via `log()` and flushes them even if the script eventually fails. Logs are labeled with the script ID so you can correlate them with `script_module_wrk.sh` benchmarks or regular server logs.

## Hot reload and testing

File-backed scripts watch their source files by default. Whenever the file’s mtime changes, Ferron recompiles the script and swaps in the new AST on the next execution. Set `reload_on_change false` only when you need deterministic behavior (for example, in production with a read-only deploy).

The repository ships with `test-script.kdl`, two sample scripts under `scripts/`, and the helper `./script_module_wrk.sh` that runs a battery of `wrk` scenarios. That script is the quickest way to validate new handlers locally:

```bash
bash ./script_module_wrk.sh -t8 -c64 -d30s http://127.0.0.1:8080/
```

For a complete walkthrough of the benchmark scenarios and tuning tips, refer to `README.md` (section “script_module_wrk.sh”).
