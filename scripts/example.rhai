// 添加自定义响应头
response.set_header("X-Script-Source", "file");
response.set_header("X-Script-Version", "1.0");

// 覆盖响应内容
response.set_status(200);
response.set_body("Hello from file script123!");
