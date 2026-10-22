# 测试

```sh
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

默认测试使用临时 SQLite、文件目录和回环 HTTP/TCP 服务，不连接真实下载器或 NAS。

## 下载器协议回归

`pipeline` 的同一组业务契约分别运行 qBittorrent、Transmission 旧 RPC、Transmission 新 JSON-RPC 2.0 模拟服务，覆盖去重、原始种子提交、提交结果未知后的恢复、重启、缺失任务显式重试、目的地快照和文件核对。跨工具切换验证原归属不变及单个实例故障隔离。

Transmission 专项覆盖 Basic 认证、409 会话更新与重试上限、RPC 错误脱敏、重复种子路径保护、元数据未就绪、Tracker 错误与本地错误区别、代理隔离及禁止认领旧 qBittorrent 任务。使用哈希查询，不依赖可变的数字任务 ID。这些是官方协议模拟测试，不代表真实 Transmission 实例联调。

## MySQL 与跨库迁移

默认测试不依赖 MySQL。准备专用 MySQL 8.4 测试服务，提供有创建数据库权限的连接；测试只创建和删除自身随机命名的 `autocomic_test_*` 库：

```sh
export AUTOCOMIC_TEST_MYSQL_URL='mysql://测试账号:密码@127.0.0.1:3306/mysql'
cargo test --locked --lib storage::tests -- --include-ignored
cargo test --locked --test migration_cli -- --include-ignored
cargo test --locked --test pipeline -- --include-ignored
```

已在官方 MySQL Community Server 8.4.11（macOS ARM64）实测。相同存储契约覆盖并发去重、中文/长资源标识、大小写及尾随空格区别、事务回滚、全局选择与历史快照、重试、文件状态和凭据脱敏；另有真实 MySQL 下载流程及 CLI 验证。SQLite → MySQL → SQLite 逐字段保留种子字节、凭据、删除标记、任务状态与历史映射；目标非空拒绝写入，注入约束失败后验证业务数据全部回滚。

运行包括 SMB 在内的所有可选测试时，同时设置 `AUTOCOMIC_TEST_MYSQL_URL` 和下述 `AUTOCOMIC_SMB_TEST_PYTHON`，再执行 `cargo test --locked -- --include-ignored`。测试异常中断可能留下随机测试库，勿指向生产服务器。

## SMB 协议联调

SMB 端到端测试默认忽略，因为需要额外的 Python 测试服务。使用 uv 创建临时环境：

```sh
uv venv /tmp/autocomic-smb-venv
uv pip install --python /tmp/autocomic-smb-venv/bin/python 'impacket==0.13.0'
AUTOCOMIC_SMB_TEST_PYTHON=/tmp/autocomic-smb-venv/bin/python \
  cargo test --locked --test smb_media -- --include-ignored
```

测试自行启动只监听 `127.0.0.1` 随机端口的只读 SMB2 共享，使用固定的测试凭据及临时文件，结束后关闭并清理。`denied` 路径由测试服务注入 SMB `STATUS_ACCESS_DENIED` 响应，以稳定验证权限错误映射，不依赖宿主机 UID。

覆盖目录列举、1000 项截断、文件存在/缺失、原目的地持久化与重启、切换全局目的地、下载器离线核对、错误密码、共享/根目录丢失和断线。另有无需 Python 的配置脱敏、P2 数据库升级和无响应 TCP 对端超时测试。

此测试证明 SMB2 协议互通，不代表已验证特定 NAS、SMB3 签名/加密或服务器隐藏的符号链接行为。服务端必须限制账户及共享范围。

## 管理页面

`cargo test --test ui` 验证内嵌资源、内容类型、浏览器安全策略和未知 API 路由不会被页面兜底。

浏览器回归需要 Node.js 和 Playwright（仅开发测试需要，部署不需要 Node）：

```sh
cargo build --locked
npm install --prefix /tmp/autocomic-ui-tools playwright@1.62.1
/tmp/autocomic-ui-tools/node_modules/.bin/playwright install chromium
NODE_PATH=/tmp/autocomic-ui-tools/node_modules node tests/ui.cjs
```

如已有 Chromium，可通过 `AUTOCOMIC_CHROMIUM` 指定其可执行文件。测试启动真实 Rust HTTP 服务、临时 SQLite 和目录，模拟外部蜜柑网站及 qBittorrent；使用随机回环端口，不访问真实网站、下载器或 NAS。结束后关闭服务并删除临时数据库，桌面和手机截图保存在系统临时目录。

覆盖搜索→字幕组→规则预览→保存、默认暂停且不入队、编辑后重新预览、订阅启停、下载器/本地/SMB 表单、路径映射前置条件、目录浏览、文件存在/缺失/不可访问、切换后旧任务归属保留、密码不回显、错误恢复、不可信标题作为文本、异步弹窗隔离、手机布局及 Escape 焦点恢复。SMB 在此只验证表单和持久化，其协议另由上述 SMB 测试覆盖。

## 容器与部署

```sh
cargo build --locked
python3 tests/deployment_config.py
python3 tests/deployment_proxy.py
docker build -t autocomic-rss:smoke .
python3 tests/container_smoke.py autocomic-rss:smoke
```

配置测试使用 Docker Compose，无需 daemon，覆盖八种组合、HTTPS 模式移除后端端口、只读挂载、MySQL 启动顺序、密码哈希插值及缺少认证参数时拒绝启动。可设置 `COMPOSE_BIN` 为独立 Compose 可执行文件。

代理测试需要 Caddy（可通过 `CADDY_BIN` 指定），启动真实 Rust 与 Caddy 回环 HTTP 服务，验证全站认证、跨站写入拒绝、凭据不转发、停机与 SQLite 恢复。它不申请公网证书或修改系统信任库。

镜像测试需要运行中的 Docker，检查新建/重建/恢复、健康检查、非 root、只读媒体与持久卷，并清理自身随机命名资源。本轮本机没有 Docker daemon，此项尚未执行；Compose 2.39.4 和原生 Caddy 2.10.2 测试已通过。
