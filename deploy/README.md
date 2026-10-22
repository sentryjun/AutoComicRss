# 部署

所有命令都在仓库根目录执行。需要 Docker Engine / Docker Desktop 和 **Docker Compose 2.24.4 或更新版本**。NAS 上也可使用支持新版 Compose 的容器管理器。没有额外的前端服务；程序、页面和数据库迁移都包含在应用镜像内。

## 默认部署：SQLite

```sh
cp deploy/.env.example deploy/.env
chmod 600 deploy/.env
docker compose --env-file deploy/.env up -d --build --wait
```

在 Docker 所在机器打开 `http://127.0.0.1:8080`。远程机器可先用 SSH 转发 `ssh -L 8080:127.0.0.1:8080 用户@服务器`，再从本机访问。长期远程使用按下文配置 HTTPS 认证入口。

首次进入页面后，添加并选择全局下载器，再配置存储目的地和路径映射，最后添加、预览并启用订阅。`QBIT_*` 环境变量仅用于首次导入，也可完全在页面配置下载器。

- 默认项目名为 `autocomic-rss`，SQLite 存在命名卷 `autocomic-rss_app_data` 的 `/app/data/ani.db`。升级或重建容器保留该卷；`down -v` 会删除数据，请勿作为普通停止命令。
- 应用以 UID/GID **10001:10001** 运行，容器根文件系统只读。SQLite 写入持久卷，临时文件写入 `/tmp`。
- `HOST_PORT` 控制宿主机端口。容器内固定监听 `0.0.0.0:8080`，默认仅向宿主机 `127.0.0.1` 发布；这与直接运行二进制默认只监听本机不同。
- 镜像在本地构建，尚无预构建镜像发布。构建时需要访问 Docker 镜像仓库、Debian 软件源与 crates.io；运行时不需要 Rust 或 Node。

## NAS 本地目录

在 `deploy/.env` 设置已有媒体目录的绝对路径和其可读组 ID，例如：

```dotenv
MEDIA_PATH=/volume1/media/anime
MEDIA_GID=100
```

```sh
docker compose --env-file deploy/.env -f compose.yaml -f deploy/compose.media.yaml up -d --build --wait
```

容器只读挂载此目录到 `/media`。给组 `MEDIA_GID` 授予目录的读取/遍历权限；应用不需要媒体写权限。路径不存在时启动失败，不会自动创建一个空目录冒充媒体库。

在管理页创建本地目的地，根目录填 `/media`；路径映射填**下载器看到同一目录的路径**，例如 `/downloads/anime`。两个路径不要求一致，也不会自动搬移文件。

## 云端应用 + NAS SMB

使用基础 Compose 即可，不需要 `compose.media.yaml` 或系统 SMB 挂载。在页面选择 SMB，填写云端容器能够连接的 NAS/VPN/隧道地址、端口、共享名与只读账号；根目录填写共享内相对路径。下载器也使用容器可达的地址。

容器里的 `127.0.0.1` 指向容器自身。访问 Docker 宿主机服务可用 `host.docker.internal`（配置已包含 Linux 的 host-gateway 映射）；Linux 宿主机服务通常需要监听容器可达的网卡，而非仅监听宿主机回环地址。通过 VPN/隧道访问时，应先确认容器到入口的路由与端口可达；不需要将 NAS 的 SMB 端口公开到互联网。

网站代理可设置 `WEBSITE_PROXY_URL=http://host.docker.internal:7897` 或可达的 SOCKS5 地址。它只作用于蜜柑、RSS 与种子请求，不影响下载器、SMB 和数据库连接。

## MySQL（部署时选定）

**已有 SQLite 部署不能靠叠加 MySQL 文件迁移数据。** 换库必须停机并按下文执行离线迁移。

新部署可连接现有 MySQL 8.4：预建专用数据库，然后在 `deploy/.env` 将 `DATABASE_URL` 设为 `mysql://用户:密码@地址:3306/数据库`，使用基础 Compose 即可。URL 中的密码特殊字符需编码；远程数据库可附加 `ssl-mode=verify-full`、`ssl-ca`，证书需另行只读挂载进容器。

也可随应用启动专用 MySQL。在 `deploy/.env` 设置不同的 `MYSQL_PASSWORD` 和 `MYSQL_ROOT_PASSWORD`，各用 `openssl rand -hex 24` 生成。此示例将应用密码放入连接 URL，请保持为十六进制字符串。

```sh
docker compose --env-file deploy/.env -f compose.yaml -f deploy/compose.mysql.yaml up -d --build --wait
```

该配置选择 MySQL 8.4.11、创建 `autocomic` 库及应用账号，等待数据库就绪后启动应用。数据库使用独立命名卷，不向宿主机发布 3306 端口。`MYSQL_*` 只在空数据库首次初始化时生效，修改环境变量不会替已有账号修改密码。

## HTTPS 和访问认证

准备指向部署机器的域名，允许公网访问 TCP 80/443。使用其他反向代理的 NAS，可继续用自己的 HTTPS 与认证入口代理宿主机 `127.0.0.1:8080`；必须覆盖整个站点及 `/api`，不要只保护首页。

附带的 Caddy 配置提供 HTTPS 与浏览器密码认证。交互生成密码哈希，避免将明文密码写进命令历史：

```sh
docker run --rm -it caddy:2.10.2-alpine caddy hash-password
```

把结果填入 `deploy/.env`，保留哈希外的单引号：

```dotenv
SITE_ADDRESS=anime.example.com
ADMIN_USER=admin
ADMIN_PASSWORD_HASH='$2a$...粘贴完整哈希...'
```

```sh
docker compose --env-file deploy/.env -f compose.yaml -f deploy/compose.https.yaml up -d --build --wait
```

访问 `https://anime.example.com`。此配置会**移除应用的宿主机端口**，只发布 Caddy 的 80/443，页面、静态资源、API 和健康端点全部要求认证。浏览器密码不会转发给应用；来自其他站点的写入请求会被拒绝。证书与 Caddy 状态保存在独立持久卷中。不要给 `SITE_ADDRESS` 加 `http://` 来省略 TLS。

Caddy 直接终止 HTTPS；若前面还有 CDN/隧道网关，请保留原始主机名并让它以 HTTPS 连接 Caddy。仅有内网域名、无法开放 80/443 时，使用已有的带认证 HTTPS 网关或 SSH/VPN 入口；当前示例没有包含 DNS challenge 插件或替你部署穿透服务。

三种附加配置可以组合，按同样的 `-f` 顺序执行后续命令：

```sh
docker compose --env-file deploy/.env \
  -f compose.yaml -f deploy/compose.media.yaml \
  -f deploy/compose.mysql.yaml -f deploy/compose.https.yaml up -d --build --wait
```

## 检查、升级和停止

以下定义一个本地 shell 函数，后续命令可复用。**若用了附加配置，请把相同的 `-f` 参数加入函数**，不要在升级时漏掉 HTTPS 或 MySQL 文件。

```sh
dc() { docker compose --env-file deploy/.env -f compose.yaml "$@"; }
dc config --quiet
dc ps
dc logs --tail 100 app
dc exec -T app curl --noproxy '*' -fsS http://127.0.0.1:8080/ready
```

`/health` 只表示 HTTP 进程存活；`/ready` 在最多两秒内检查数据库连接，返回 200 或 503，是镜像的健康检查入口。它不会扫描订阅或探测下载器/SMB。Compose 的 `unhealthy` 不会自动触发容器重启；退出的进程由 `unless-stopped` 策略重启。

升级：记录当前 Git 提交与 `IMAGE_TAG`，完成停机备份，再切换到目标版本。建议每个版本使用不同的 `IMAGE_TAG`，保留原镜像用于回退。

```sh
dc stop app
# 此时按下一节备份；随后更新代码和 deploy/.env 中的 IMAGE_TAG。
dc build app
dc up -d --wait
```

启动会自动执行数据库迁移。回退不能仅换回旧二进制：若 schema 已升级，应使用**升级前对应的备份与旧镜像**。正常停止用 `dc stop`，重启用 `dc start`。同一数据库只运行一个应用实例。

## SQLite 备份与恢复

使用上节的 `dc` 函数。停止应用后将整个数据卷打包，保留 WAL 等辅助文件；媒体文件不在此备份范围。备份包含下载器/SMB 凭据，限制文件权限。

```sh
umask 077
mkdir -p deploy/backups
dc stop app
dc run --rm --no-deps -T --entrypoint tar app -C /app/data -czf - . > deploy/backups/sqlite.tar.gz
# 确认上一条命令成功，且归档可读，再启动。
tar -tzf deploy/backups/sqlite.tar.gz
dc start app
```

恢复到**全新项目的数据卷**，避免覆盖原库。下面复用本地已构建镜像；若 `IMAGE_TAG` 已改动，确保对应旧镜像仍存在。原部署需停止，恢复后的部署继续使用相同下载器与路径配置。

```sh
dc stop app
# 新项目名会创建新的空命名卷。使用基础 SQLite 配置，恢复完成前不启动应用。
docker compose --env-file deploy/.env -p autocomic-restored -f compose.yaml \
  run --rm --no-deps -T --entrypoint sh app -ec \
  'test -z "$(ls -A /app/data)"; tar -C /app/data -xzf -' < deploy/backups/sqlite.tar.gz
```

确认新卷中的备份数据后，再使用 `-p autocomic-restored` 和原来的存储/HTTPS附加配置启动。保留旧项目停止状态。若恢复卷已经有数据，上述命令会拒绝解包；不要删除原库来绕过检查。新项目名必须一直保留，否则会连接回其他卷。

MySQL 使用官方 `mysqldump`/`mysql` 工具。停止 `app` 后进行逻辑备份，恢复到全新空库；不要在 MySQL 运行时直接复制 `/var/lib/mysql`。随应用部署 MySQL 时可执行：

```sh
umask 077
dc stop app
dc exec -T mysql sh -c 'MYSQL_PWD="$MYSQL_PASSWORD" exec mysqldump -u autocomic --single-transaction --no-tablespaces --set-gtid-purged=OFF autocomic' > deploy/backups/mysql.sql
# 检查备份命令成功后再启动应用。
dc start app
```

还应单独备份 `deploy/.env`，HTTPS 部署备份 Caddy 的数据卷；不要把凭据或证书目录加入 Git。

## SQLite / MySQL 离线换库

停止所有连接源、目标库的应用并备份；目标必须是空库。源库需已由当前程序升级到当前 schema。保持数据库服务运行，在当前 Compose 网络中执行：

```sh
dc stop app
# 在当前终端设置 MIGRATION_SOURCE_URL 与 MIGRATION_TARGET_URL。
# SQLite URL 使用容器路径 sqlite:///app/data/ani.db；MySQL URL 使用容器可达地址。
dc run --rm --no-deps -T \
  -e MIGRATION_SOURCE_URL -e MIGRATION_TARGET_URL app migrate --offline
```

迁移验证成功后修改部署配置，只启动目标服务。迁移不会自动改变 `DATABASE_URL`；若启用了 MySQL 附加文件，它会覆盖基础文件中的 `DATABASE_URL`，切回 SQLite 时需要移除此附加文件。失败时源库仍可用。当前迁移限 256 MiB 业务字段/10 万行，不包含媒体文件；已验证 SQLite 与 MySQL 的双向迁移。

## 验证范围

本轮已通过 Compose 2.39.4 的全部八种配置组合解析，以及真实 Caddy 2.10.2 + Rust 回环服务的认证、跨站写入限制、凭据移除、数据库就绪与 SQLite 停机恢复测试。Docker 官方镜像标签的 amd64/arm64 清单已核对。

本机没有 Docker daemon，**镜像实际构建和容器运行尚待验证**。已提供容器验收脚本，具备 Docker 的机器可执行：

```sh
docker build -t autocomic-rss:smoke .
python3 tests/container_smoke.py autocomic-rss:smoke
```

脚本创建随机命名的临时容器/卷，验证健康检查、非 root、只读挂载、重建持久化、退出和卷备份恢复，最后清理自身资源。实际 NAS、SMB3、真实下载器及公网域名证书签发仍需目标环境联调。

配置依据：[Compose 合并规则](https://docs.docker.com/compose/how-tos/multiple-compose-files/merge/)、[持久卷](https://docs.docker.com/engine/storage/volumes/)、[环境变量插值](https://docs.docker.com/compose/how-tos/environment-variables/variable-interpolation/)、[Caddy 密码认证](https://caddyserver.com/docs/caddyfile/directives/basic_auth)。
