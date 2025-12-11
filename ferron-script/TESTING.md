# Script 模块测试指南

目标：提供可直接运行的用例，覆盖请求拦截、请求体修改、响应改写、定时任务、后台任务、状态存储、环境注入与热重载。

## 快速验证：文件脚本改写响应

1) 配置 `test-script.kdl`（已随仓库提供，示例放在 `on_response_ready` 阶段）：
```kdl
globals {
  log "access.log"
  error_log "error.log"
}

:8080 {
  module "script-exec" {
    script "test-script" {
      file "scripts/example.rhai"
      trigger "on_response_ready"
      reload_on_change #true
      limits {
        max_operations 200_000
        max_call_depth 32
        max_exec_time "50ms"
      }
      failure_policy "skip"
    }
  }

  root "wwwroot"
}
```

2) 脚本 `scripts/example.rhai` 内容（会覆盖响应并加头）：
```rhai
log("info", "Executing script from file");
log("info", "Request method: " + request.method);
log("info", "Request URI: " + request.uri);

response.set_header("X-Script-Source", "file");
response.set_header("X-Script-Version", "1.0");
response.set_status(200);
response.set_body("Hello from file script!");
```

3) 运行并验证：
```bash
./target/debug/ferron --config test-script.kdl
curl -i http://localhost:8080/
# 预期：响应体为 Hello from file script!，头含 X-Script-Source/X-Script-Version
```

> 注意：修改响应体后，运行时会自动重算 Content-Length。

## 组合测试场景

可将以下脚本块逐个加入 `module "script-exec"`，或拆分为多个 script。

### 1. 请求拦截 deny
```kdl
script "auth-check" {
  inline "
    if request.header('Authorization') == () {
      deny(403, 'Access denied');
    }
  "
  trigger "on_request_start"
}
```
验证：
```bash
curl -i http://localhost:8080/               # 403
curl -i -H "Authorization: Bearer token" http://localhost:8080/  # 继续
```

### 2. 修改请求头/方法/URI（请求阶段）
```kdl
script "rewrite-request" {
  inline "
    request.set_header('X-Debug', '1');
    request.method = 'POST';
    request.uri = '/rewritten';
  "
  trigger "on_request_start"
}
```

### 3. 修改请求体（需要 on_request_body，会自动缓冲请求体）
```kdl
script "body-rewrite" {
  inline "
    let body = request.body;
    let new_body = body + '#patched';
    request.body = new_body;
  "
  trigger "on_request_body"
}
```

### 4. 响应改写（头 + 体）
```kdl
script "resp-rewrite" {
  inline "
    response.set_header('X-Custom', 'yes');
    response.set_body('patched by script');
    response.set_status(202);
  "
  trigger "on_response_ready"
}
```

### 5. 定时任务 on_tick
```kdl
script "ticker" {
  inline "
    log('info', 'tick fired');
  "
  trigger "on_tick"
  tick_interval "2s"
}
```

### 6. 后台任务（需允许 spawn_task）
```kdl
script "bg-task" {
  inline "
    spawn_task('cleanup', || {
      log('info', 'background task executed');
    });
  "
  trigger "on_request_start"
  allow ["spawn_task"]
}
```

### 7. 模块级状态存储
```kdl
script "counter" {
  inline "
    let count = state.get('count');
    if count == () { count = 0; }
    count = count + 1;
    state.set('count', count);
    response.set_header('X-Count', count.to_string());
  "
  trigger "on_response_ready"
}
```

### 8. 环境变量注入
```kdl
script "env-test" {
  inline "
    log('info', 'API key = ' + env.api_key);
  "
  trigger "on_request_start"
  env { api_key "secret-key-123" }
}
```

### 9. 文件脚本热重载
```kdl
script "file-reload" {
  file "scripts/hot.rhai"
  trigger "on_response_ready"
  reload_on_change true
}
```
修改 `scripts/hot.rhai` 后再次请求即可看到生效，无需重启。

## 运行与调试提示

- 运行：`./target/debug/ferron --config test-script.kdl` 或 `make run-dev`（确保复制配置为 `ferron.kdl`）。
- 日志：`tail -f access.log`、`tail -f error.log`，脚本内 `log(level, msg)` 会落在错误日志。
- 失败策略：`failure_policy "block"` 会中断请求；`"skip"` 仅跳过脚本。
- 超时与限额：`limits.max_exec_time`/`max_operations`/`max_call_depth` 可按需调高测试。

## 自动化测试建议

- 单元测试：`cargo test -p ferron-script`。
- Smoketest：在 `smoketest/` 复制 `ferron.kdl`，添加 `module "script-exec"` 配置，运行 `smoketest/smoketest.sh`，验证请求/响应改写和 deny 场景。

### 1. 查看日志

```bash
# 查看访问日志
tail -f access.log

# 查看错误日志
tail -f error.log
```

### 2. 脚本错误处理

如果脚本有错误，根据 `failure_policy` 设置：
- `"block"`: 请求会被阻止
- `"skip"`: 请求会继续，但脚本不会执行

### 3. 检查脚本编译

脚本会在首次加载时编译。如果编译失败，会在错误日志中显示。

### 4. 热重载测试

对于文件脚本，设置 `reload_on_change true`：

```kdl
script "reload-test" {
  file "scripts/test.rhai"
  trigger ["on_request_start"]
  reload_on_change true
}
```

修改文件后，脚本会自动重新加载。

## 单元测试

运行单元测试：

```bash
cargo test -p ferron-script
```

## 集成测试

在 `smoketest` 目录创建测试：

```bash
# 创建测试配置
cp smoketest/ferron.kdl smoketest/ferron-script.kdl

# 添加 script 模块配置到 smoketest/ferron-script.kdl

# 运行测试
cd smoketest
./smoketest.sh
```

## 常见问题

### Q: 脚本没有执行？

- 检查 `trigger` 配置是否正确
- 检查 `failure_policy` 是否为 `"block"` 且脚本有错误
- 查看错误日志

### Q: 脚本执行超时？

- 增加 `max_exec_time` 限制
- 检查脚本逻辑是否有死循环

### Q: 如何调试脚本？

- 使用 `log()` 函数输出调试信息
- 查看错误日志
- 使用 `failure_policy "skip"` 避免阻塞请求
