# AutoComicRss

使用 Rust 独立实现的 RSS 自动追番服务，功能方向参考 [ani-rss](https://github.com/wushuo894/ani-rss)。

## 功能

- 搜索蜜柑计划番剧、字幕组和 RSS，也支持手动添加 RSS/Atom 订阅。
- 预览订阅、筛选资源、识别集数和去重，定时提交下载并同步任务状态。
- 接入 qBittorrent 和 Transmission，支持多个实例和全局切换，旧任务继续由原实例管理。
- 设置全局存储目的地与路径映射，浏览本地目录或直连 SMB 共享、手动核对文件是否存在。
- 访问蜜柑、RSS 和种子资源时可使用 HTTP / SOCKS5 代理。

内置中文管理页面，支持手机访问；部署时可选择 SQLite 或 MySQL 8.4，不提供视频播放。

## Docker 部署

需要 Docker 和 Compose 2.24.4+：

```sh
git clone https://github.com/sentryjun/AutoComicRss.git
cd AutoComicRss
cp deploy/.env.example deploy/.env
chmod 600 deploy/.env
docker compose --env-file deploy/.env up -d --build --wait
```

默认使用 SQLite 持久卷，访问 [管理页面](http://127.0.0.1:8080)。先配置全局下载器、存储目的地和路径映射，再搜索番剧、预览规则并启用订阅。

NAS 本地目录、SMB、MySQL、HTTPS 密码认证、升级与备份恢复见 [部署说明](deploy/README.md)。默认端口仅向宿主机本地开放；远程使用请配置说明中的认证入口。网站代理也在部署配置中设置。

镜像在本地构建，运行时无需 Rust 或 Node。本轮已验证 Compose 配置与原生服务/反向代理；实际 Docker 镜像构建、运行需按部署说明中的冒烟测试补验。

## 源码运行

```sh
cargo build --release --locked
cp .env.example .env
# 编辑 .env 后加载并启动
set -a
. ./.env
set +a
./target/release/autocomic-rss
```

已在 Rust 1.93.1 验证。默认监听 `127.0.0.1:8080`，数据库为 `data/ani.db`；可通过 `APP_HOST`、`APP_PORT` 和 `DATABASE_URL` 调整。程序不会自动读取 `.env`，同一数据库只运行一个实例。`/health` 检查进程，`/ready` 检查数据库连接。完整配置见 [.env.example](.env.example)。

## 许可证

[GPL-2.0-only](LICENSE)。本项目独立设计实现，并非 ani-rss 的官方 Rust 版本。
