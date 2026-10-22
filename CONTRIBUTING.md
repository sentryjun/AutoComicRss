# 开发说明

项目使用单个 Cargo package：`src/main.rs` 处理进程入口，`src/lib.rs` 暴露供应用和集成测试共用的模块。默认采用 `cargo fmt` 格式，不需要前端构建工具。

## 目录职责

| 路径 | 内容 |
| --- | --- |
| `src/app.rs`、`src/config.rs` | 服务启动、停机与部署配置 |
| `src/domain/` | 共享模型、输入校验和标题解析结果，不持有网络或数据库客户端 |
| `src/service/` | 订阅扫描、下载同步、文件核对和后台调度 |
| `src/http/` | 按资源组织的路由、请求转换和内嵌页面 |
| `src/sources/` | RSS、蜜柑和 Bangumi 数据源 |
| `src/scraping/` | 刮削配置、解析调度、正则与大模型适配 |
| `src/downloaders/` | 下载工具适配和种子元数据处理 |
| `src/media/` | 本地/SMB 存储访问与路径校验 |
| `src/storage/` | SQLite/MySQL 连接和按业务组织的持久化操作 |
| `migrations/sqlite/`、`migrations/mysql/` | 分数据库保存的版本迁移 |
| `web/` | 编译时内嵌的 HTML、CSS 和 JavaScript |
| `tests/` | Rust 集成测试；较大套件以 `<套件>/main.rs` 为入口 |
| `tests/browser/`、`tests/deployment/` | 浏览器回归与部署检查脚本 |
| `tests/fixtures/`、`tests/support/` | 通用测试数据及外部协议模拟服务 |
| `deploy/` | 部署配置与操作说明 |

`mod.rs` 定义模块入口与公共导出，内部实现默认私有。跨模块共享模型放在 `domain`，业务流程通过 `Service` 和 `Store` 组合现有能力。仅供一个测试套件使用的模拟器与该套件放在一起。

已有迁移不修改内容、编号或名称；数据库变更新增迁移文件，并同步维护两个数据库的迁移及离线复制字段。移动迁移目录时须保留原文件字节，避免已有迁移账本校验失败。

## 本地验证

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
```

Rust 测试使用临时 SQLite 和回环模拟服务。`pipeline`、`metadata` 仍可用 `cargo test --test pipeline`、`cargo test --test metadata` 单独运行。浏览器、Compose、Caddy、Docker 和可选 MySQL/SMB 测试的依赖与命令见 [测试说明](tests/README.md)。

不要提交本地凭据、数据库或构建产物。README 保持功能概述和部署入口；操作细节写入 `deploy/README.md`，测试细节写入 `tests/README.md`。
