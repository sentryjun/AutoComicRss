# AutoComicRss

使用 Rust 独立实现的 RSS 自动追番服务，功能方向参考 [ani-rss](https://github.com/wushuo894/ani-rss)。

## 功能

- 搜索蜜柑计划番剧、字幕组和 RSS，也支持手动添加 RSS/Atom 订阅。
- 预览订阅、筛选资源、识别集数和去重，定时提交下载并同步任务状态。
- 接入 qBittorrent，支持多个实例和全局切换，旧任务继续由原实例管理。
- 设置全局存储目的地与路径映射，浏览本地目录、手动核对文件是否存在。
- 访问蜜柑、RSS 和种子资源时可使用 HTTP / SOCKS5 代理。

目前提供后端 API，使用 SQLite 存储数据，尚无管理页面。SMB 直连、MySQL 和其他下载工具尚未实现；不提供视频播放。

## 部署

准备 Rust 工具链（已在 Rust 1.93.1 验证）和开启 Web UI 的 qBittorrent。

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

`QBIT_SAVE_PATH` 是下载器看到的保存路径。`QBIT_*` 仅在首次配置下载器时导入，后续通过 API 管理。网站代理需显式配置，不继承系统代理，也不影响下载器连接。

程序不会自动读取 `.env`，在项目目录加载配置后启动：

```sh
set -a
. ./.env
set +a
./target/release/autocomic-rss
```

默认监听 `127.0.0.1:8080`，每 300 秒检查订阅，数据保存在 `data/ani.db`，启动时自动建库和迁移。更多配置见 [.env.example](.env.example)。同一数据库只运行一个服务实例；备份时停止服务并保存 `data/`，其中包含下载器凭据。

在另一终端检查服务是否正常：

```sh
curl http://127.0.0.1:8080/api/status
```

当前服务没有用户认证，仅监听本机。需要远程访问时，请使用带认证的反向代理或受控隧道。

## 许可证

[GPL-2.0-only](LICENSE)。本项目独立设计实现，并非 ani-rss 的官方 Rust 版本。
