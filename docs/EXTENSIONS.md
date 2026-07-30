# 进程外扩展服务

RUPORA 的扩展不会作为动态库载入编辑器进程。扩展是由用户明确配置的外部可执行程序，通过
stdin/stdout 上的一次性 JSON 协议接收活动文档并返回结果。扩展默认全局关闭。

## 配置

从“扩展 → 打开扩展配置”创建并打开 `extensions.json`。示例：

```json
{
  "enabled": true,
  "services": [
    {
      "name": "Uppercase example",
      "program": "C:\\absolute\\path\\extension_uppercase.exe",
      "args": [],
      "permissions": ["read_document", "replace_document"],
      "timeout_ms": 5000,
      "max_output_bytes": 1048576
    }
  ]
}
```

保存后选择“重新加载扩展配置”。程序路径必须是绝对路径；RUPORA 直接启动该程序，不调用
shell。服务名称不区分大小写且不能重复，最多配置 32 个。

权限：

- `read_document`：接收活动文档正文；运行服务必须有此权限。
- `read_document_path`：额外接收文档的本地路径。
- `replace_document`：允许响应替换整篇文档；替换会成为一个可撤销事务。

只读扩展可以仅返回 `message`。文档在服务运行期间发生变化时，RUPORA 会丢弃过期结果。

## 协议

请求是一行 UTF-8 JSON：

```json
{
  "protocol": 1,
  "requestId": 42,
  "method": "transformDocument",
  "document": {
    "text": "# Markdown",
    "path": "D:\\notes\\example.md"
  }
}
```

没有 `read_document_path` 权限时省略 `path`。成功响应：

```json
{
  "protocol": 1,
  "requestId": 42,
  "result": {
    "replacement": "# Updated Markdown",
    "message": "updated"
  }
}
```

失败响应可以使用 `"error": "message"` 代替 `result`。未知字段、错误协议版本、错误请求 ID、
无权限的 `replacement` 或无效 JSON 都会被拒绝。

## 资源与安全边界

- 文档输入上限 8 MiB。
- 默认响应上限 1 MiB，配置不能超过 4 MiB。
- 默认超时 5 秒，配置范围 100 ms–30 秒；超时进程会被终止。
- 子进程不继承编辑器环境变量，只获得少量操作系统目录变量和
  `RUPORA_EXTENSION_PROTOCOL=1`。
- stdout 只允许协议响应，stderr 被丢弃；诊断信息应由扩展自行记录。

进程外协议可保护编辑器内存和限制 RUPORA 交给服务的数据/可接受的操作，但它不是操作系统
沙箱。扩展程序仍以当前用户身份运行，可能自行访问用户有权访问的文件和网络。只配置自己
信任的可执行程序；需要更强隔离时应在容器、低权限账户或平台沙箱中运行服务。

## Rust 示例

仓库提供 [`examples/extension_uppercase.rs`](../examples/extension_uppercase.rs)：

```bash
cargo build --locked --example extension_uppercase
```

把生成的绝对可执行路径写入配置即可验证完整调用、权限、超时和可撤销替换流程。
