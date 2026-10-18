# AutoComicRss

使用 Rust 独立实现的 RSS 自动追番服务，功能方向参考 [ani-rss](https://github.com/wushuo894/ani-rss)。

## 功能

- 搜索蜜柑计划番剧、字幕组和 RSS，也支持手动添加 RSS/Atom 订阅。
- 预览订阅、筛选资源、识别集数和去重，定时提交下载并同步任务状态。
- 接入 qBittorrent 和 Transmission，支持多个实例和全局切换，旧任务继续由原实例管理。
- 设置全局存储目的地与路径映射，浏览本地目录或直连 SMB 共享、手动核对文件是否存在。
- 访问蜜柑、RSS 和种子资源时可使用 HTTP / SOCKS5 代理。

内置中文管理页面，支持手机访问；部署时可选择 SQLite 或 MySQL 8.4，不提供视频播放。

## 部署

准备 Rust 工具链（已在 Rust 1.93.1 验证），以及开启 Web UI 的 qBittorrent 或开启 RPC 的 Transmission。

```sh
git clone https://github.com/sentryjun/AutoComicRss.git
cd AutoComicRss
cargo build --release --locked
cp .env.example .env
```

编辑 `.env`，取消注释并填写下载器配置：

```sh
QBIT_URL='http://127.0.0.1:8081'
QBIT_USERNAME='admin'
QBIT_PASSWORD='你的密码'
QBIT_SAVE_PATH='/downloads'

# 按需启用网站代理，也可使用 socks5h://127.0.0.1:7897
# WEBSITE_PROXY_URL='http://127.0.0.1:7897'
```

也可以跳过 `QBIT_*`，启动后在管理页添加下载器。`QBIT_SAVE_PATH` 是下载器看到的保存路径；环境变量仅在首次配置时导入，后续通过页面或 API 管理。网站代理需显式配置，不继承系统代理，也不影响下载器连接。

程序不会自动读取 `.env`，在项目目录加载配置后启动：

```sh
set -a
. ./.env
set +a
./target/release/autocomic-rss
```

默认监听 `127.0.0.1:8080`，每 300 秒检查订阅，数据保存在 `data/ani.db`，启动时自动建库和迁移。更多配置见 [.env.example](.env.example)。同一数据库只运行一个服务实例。备份前停止服务：SQLite 保存 `data/`，MySQL 使用数据库备份工具；备份中包含下载器和 SMB 凭据。

浏览器打开 [管理页面](http://127.0.0.1:8080)。首次使用先添加并选择全局下载器，再添加存储目的地、配置下载器写入路径映射并选为全局。然后搜索番剧、选择字幕组，预览规则并保存订阅；勾选“启用自动追番”后才会自动下载。任务页面可手动核对文件是否存在。健康检查地址为 `/health`。

当前服务没有用户认证，仅监听本机。需要远程访问时，请使用带认证的反向代理或受控隧道。

### MySQL 与换库

默认使用 SQLite。使用 MySQL 8.4 时，预先创建专用数据库，为账号授予该库的建表和读写权限，再设置 `DATABASE_URL='mysql://用户:密码@地址:3306/数据库名'`。凭据中的特殊字符需 URL 编码；远程连接可配置 `ssl-mode=verify-full` 和 `ssl-ca`。启动时自动执行对应数据库的迁移。

换库需停止源、目标服务并备份，使用当前版本升级源库后，执行离线迁移：

```sh
export MIGRATION_SOURCE_URL='sqlite://data/ani.db'
export MIGRATION_TARGET_URL='mysql://用户:密码@地址:3306/空数据库名'
./target/release/autocomic-rss migrate --offline
```

支持双向迁移，只接受空目标库；逐字段验证通过后才提交业务数据。源库保持只读，不生成含凭据的中间文件。成功后手动修改 `DATABASE_URL`，只启动目标服务；失败可继续使用源库。当前工具限 256 MiB 业务数据、10 万行，不包含媒体文件。

### Transmission

在“下载器”页面添加 Transmission，填写完整 RPC 地址（例如 `http://127.0.0.1:9091/transmission/rpc`）、凭据和保存路径，再选为全局下载器。支持 3.x/4.0 旧 RPC 与 4.1 新 RPC；API 配置示例见 [实例配置](examples/transmission-downloader.json)。

如已配置全局存储目的地，需先为新实例添加写入路径映射。切换不迁移旧任务；Transmission 连接直达 RPC 地址，不使用网站代理。新旧 RPC 已通过模拟服务回归，真实下载器仍需联调。

### SMB 存储

无需系统挂载。在“存储目的地”页面选择 SMB 直连，填写连接信息，再配置下载器写入路径映射并选为全局目的地。API 示例见 [SMB 目的地配置](examples/smb-destination.json)。`root_path` 是共享内的相对目录，空字符串表示共享根；端口可按隧道入口设置。

请使用仅可读取媒体共享的账号。SMB 直连不使用网站代理，凭据保存在数据库中，接口不回显密码。已通过 SMB2 测试共享联调，具体 NAS、SMB3 签名与加密仍需实测；暂不支持 DFS 和客户端可识别的链接路径，服务端需限制共享范围及链接跳转。

## 许可证

[GPL-2.0-only](LICENSE)。本项目独立设计实现，并非 ani-rss 的官方 Rust 版本。
