# Rust RSS 追番服务

使用 Rust 独立设计实现的 RSS 订阅与下载自动化项目，采用 **GPL-2.0-only**。
功能方向参考 [ani-rss](https://github.com/wushuo894/ani-rss)，没有移植其 Java 业务代码或前端资源；本项目不是上游的官方 Rust 版本。

当前为后端 MVP：蜜柑搜索 / 手动 RSS → 订阅管理 → RSS/Atom 预览 → 持久化任务 → qBittorrent 提交与状态同步。
现有 Actix 示例接口继续保留。

## 运行

```sh
cargo run --locked
```

默认监听 `127.0.0.1:8080`，自动创建 `data/ani.db` 并运行数据库迁移。
当前 API 供本机使用，没有用户认证；监听地址固定为回环地址。
首次运行需要下载 Cargo 依赖。已在 Rust 1.93.1 上进行编译验证。

按需配置环境变量，参见 [.env.example](.env.example)。程序不自动读取 `.env`：

```sh
export QBIT_URL='http://127.0.0.1:8081'
export QBIT_USERNAME='admin'
export QBIT_PASSWORD='你的密码'
export QBIT_SAVE_PATH='/downloads'
cargo run --locked
```

qBittorrent 需要启用 Web UI。`QBIT_SAVE_PATH` 是下载器看到的路径。
`QBIT_*` 只在数据库尚无下载器实例时导入一次，并选为全局下载器。已有实例时，重启或修改这些环境变量不会覆盖保存的实例或选择。
全新安装不设置 `QBIT_URL` 时，仍可管理订阅、预览和生成未分配任务；首次通过接口选中下载器后绑定这些任务。清空环境变量不会停用数据库中已选中的下载器。
程序启动后立即执行一轮检查，之后默认每 300 秒检查一次；`POLL_INTERVAL_SECONDS` 最小为 10。

## 使用流程

### 从蜜柑发现订阅

```sh
curl -G 'http://127.0.0.1:8080/api/sources/mikan/search' --data-urlencode 'q=葬送'
curl 'http://127.0.0.1:8080/api/sources/mikan/series/3141'
curl 'http://127.0.0.1:8080/api/sources/mikan/resolve' \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://mikanani.me/Home/Bangumi/3141"}'
```

搜索返回各季度的独立候选及关键词 RSS。详情返回整部番剧 RSS、字幕组 RSS、站点信息和可用封面地址。
将详情中的字幕组 `rss_url` 传给 `resolve`，可以获得对应字幕组候选。
`resolve` 返回的 `subscription` 对象可直接作为下面预览、保存接口的请求体；默认禁用，`season=1` 只是初始值，需核对季度、偏移和筛选规则。
候选中的来源 ID、字幕组 ID 和范围用于发现流程；当前订阅保存标题、RSS 与规则，尚不单独持久化这些来源元数据。

关键词 RSS 可能跨多部作品，`resolve` 返回 `scope=keyword`、提示和 RSS 地址，`subscription=null`，不会生成单番剧草稿。
建议选择具体番剧；若手工使用关键词 RSS，需先用规则将内容限定为一部作品并预览。
搜索、查看详情、解析链接和预览均不创建下载任务。发布条目通过 RSS 预览获取，不从详情 HTML 重复解析。
这阶段提供后端 API，尚无搜索管理页面或蜜柑账号同步。

### 网站代理

```sh
export WEBSITE_PROXY_URL='http://127.0.0.1:7897'
# 或 socks5h://127.0.0.1:7897，让代理解析目标域名
export MIKAN_BASE_URL='https://mikanani.me'
cargo run --locked
```

网站客户端支持 HTTP、HTTPS、SOCKS5 和 SOCKS5H 代理 URL，可在 URL 内携带编码后的用户名和密码。
`socks5` 本地解析目标域名，`socks5h` 由代理解析。配置在启动时读取，修改后重启。
**不继承** `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY` 或 `NO_PROXY`；未设置 `WEBSITE_PROXY_URL` 时直连。
可用 `WEBSITE_DIRECT_HOSTS` 设置逗号分隔的显式直连域名/IP/CIDR，例如 `localhost,127.0.0.1,::1`，默认没有例外。
每次重定向重新应用代理及例外；代理失败返回错误，不自动直连。

蜜柑搜索、详情、封面、RSS 和种子元数据请求共用网站策略。封面返回本应用地址，由后端获取，浏览器无需直连蜜柑。
当前仅识别蜜柑同源 `/images/` 下的封面，支持 PNG/JPEG/WebP/GIF；其他图片布局会显示无封面。
qBittorrent 控制连接独立且始终直连；BT 传输由下载器控制。容器中的 `127.0.0.1` 指容器自身。
网站基址必须是无路径、查询参数或凭据的 HTTP(S) origin；粘贴解析仅接受该站点的公开搜索/番剧页面或 RSS。

### 预览、保存和启用

1. 预览真实的 RSS 地址，查看匹配结果。预览不会保存订阅或提交下载。

```sh
curl -s http://127.0.0.1:8080/api/preview \
  -H 'Content-Type: application/json' \
  -d '{
    "title": "示例番剧",
    "rss_url": "https://example.com/anime.xml",
    "season": 1,
    "episode_offset": 0,
    "include": ["1080p"],
    "exclude": ["合集"],
    "enabled": false
  }'
```

2. 将相同 JSON 提交到 `POST /api/subscriptions` 保存订阅。订阅默认禁用。
3. 确认预览结果后，向 `PUT /api/subscriptions/{id}` 提交完整 JSON，将 `enabled` 改为 `true`。
4. 等待自动检查，或使用 `POST /api/run` 立即执行一轮 RSS 检查和下载同步。
5. 使用 `GET /api/jobs` 查看状态、失败原因和重试时间。

首次启用会考虑 RSS 当前返回的全部匹配条目，尚未实现“仅从当前最新集开始”。
`include` 为不区分大小写的关键词“全部满足”，`exclude` 为“任意命中则排除”，不是正则表达式。
修改订阅规则只影响后续扫描；已入队任务保留创建时的季度、集数和资源地址。禁用订阅会暂停未提交任务。

## API

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/api/status` | 版本及下载器是否配置，不返回凭据 |
| GET / POST | `/api/downloaders` | 列出 / 创建下载器实例，响应不返回密码 |
| PUT | `/api/downloaders/active` | `{"downloader_id":"实例 ID"}` 选择接收新任务的全局下载器 |
| POST | `/api/downloaders/{id}/claim-legacy-jobs` | 将升级后尚无归属的旧任务明确关联到恢复的原实例 |
| GET / POST | `/api/destinations` | 列出 / 创建本地媒体目的地 |
| PUT | `/api/destinations/active` | `{"destination_id":"目的地 ID"}` 设置全局目的地 |
| GET / POST | `/api/destinations/{id}/mappings` | 查看历史映射 / 新增映射版本 |
| GET | `/api/destinations/{id}/entries?path=相对目录` | 只读浏览目录，省略 path 为根目录 |
| GET | `/api/sources/mikan/search?q=关键词` | 番剧搜索和关键词 RSS |
| GET | `/api/sources/mikan/series/{id}` | 番剧、字幕组及对应 RSS |
| GET | `/api/sources/mikan/series/{id}/cover` | 经网站网络策略获取封面 |
| POST | `/api/sources/mikan/resolve` | `{"url":"蜜柑地址"}` → 订阅候选 |
| POST | `/api/preview` | 临时预览，不写库、不下载 |
| GET / POST | `/api/subscriptions` | 列出 / 创建订阅 |
| GET / PUT / DELETE | `/api/subscriptions/{id}` | 查询 / 完整替换 / 删除订阅 |
| GET | `/api/subscriptions/{id}/preview` | 预览已保存订阅，包含去重判断 |
| POST | `/api/subscriptions/{id}/scan` | 扫描并入队，要求订阅已启用 |
| GET | `/api/subscriptions/{id}/releases` | 最近最多 500 条扫描记录 |
| GET | `/api/jobs` | 最近最多 500 个任务 |
| GET | `/api/jobs/{id}/files` | 读取已保存的文件清单与核对状态 |
| POST | `/api/jobs/{id}/files/check` | 发现文件清单（首次）并核对本地文件是否存在 |
| POST | `/api/jobs/{id}/retry` | 手动重试 failed / missing 任务 |
| POST | `/api/run` | 执行完整扫描及下载同步 |

后台处理与手动执行互斥；处理期间修改订阅或再次执行会返回 `409`，稍后重试即可。
删除订阅为软删除，保留去重与任务记录，不删除下载器任务或媒体文件。已提交任务仍会同步状态。

## 下载器实例与切换

```sh
curl 'http://127.0.0.1:8080/api/downloaders' \
  -H 'Content-Type: application/json' \
  -d '{"name":"NAS qBittorrent","kind":"qbittorrent","connection":{"url":"http://nas:8081","username":"admin","password":"你的密码","save_path":"/downloads"}}'

# 将返回的 id 填入下面的 downloader_id。
curl -X PUT 'http://127.0.0.1:8080/api/downloaders/active' \
  -H 'Content-Type: application/json' -d '{"downloader_id":"返回的实例 ID"}'
```

创建实例不会自动切换全局选择。任务在扫描入队的同一个事务中保存 `downloader_id`；已有归属的排队任务、重试和重启恢复都使用原实例。
切换后继续同步原实例；一个实例连接失败会报告该实例 ID，并继续处理其他实例，不把连接失败标成任务丢失。
原实例的地址与写入路径不随全局选择改变。本阶段只实现多个 qBittorrent 实例，不提供实例编辑/删除；第二种工具与凭据维护接口另行扩展。
实例凭据保存在 SQLite 数据库中，尚未加密；数据库及其备份应按含凭据的配置文件管理。列表和创建响应只返回 `password_configured`。

### 从旧版本升级

升级迁移为旧任务增加实例归属字段。首次导入时，应提供**旧任务实际使用的原 `QBIT_*` 配置**，应用会将它与旧任务关联；不能在升级时把这些值替换成另一台下载器。
若升级时没有提供原配置，历史任务保留 `legacy_unassigned=true`，不会自动绑定以后新选中的实例；`/api/status` 的 `legacy_unassigned_jobs` 及运行报告会提示数量。
恢复原实例后，显式调用 `POST /api/downloaders/{原实例ID}/claim-legacy-jobs` 才建立关联。该操作仅认领没有归属的旧任务，不迁移已有归属，也不立即提交下载；后续同步先核对原实例的任务。

## 全局媒体目的地与本地文件核对

先配置并选中下载器，再创建一个本地目的地，配置该下载器的写入路径映射，最后激活目的地：

```sh
curl 'http://127.0.0.1:8080/api/destinations' \
  -H 'Content-Type: application/json' \
  -d '{"name":"番剧目录","kind":"local","root_path":"/mnt/nas/anime"}'

curl 'http://127.0.0.1:8080/api/destinations/目的地ID/mappings' \
  -H 'Content-Type: application/json' \
  -d '{"downloader_id":"下载器ID","download_root":"/downloads/anime"}'

curl -X PUT 'http://127.0.0.1:8080/api/destinations/active' \
  -H 'Content-Type: application/json' -d '{"destination_id":"目的地ID"}'
```

`root_path` 是 Rust 应用所在环境可访问的绝对目录；`download_root` 是同一个位置在下载器环境中的绝对路径。
配置映射不会挂载、复制或上传文件，也不会创建这些目录。两个路径必须实际指向同一批文件。
同一时刻所有新任务统一使用全局目的地，订阅不能覆盖。切换目的地或下载器前，必须存在所需映射，否则返回 `400`，不会静默使用默认目录。

每次 `POST mappings` 都追加一个版本；新任务使用最新版本，旧任务保留 `destination_id`、`mapping_id` 和 `download_save_path`。
目的地读取根目录和已有映射版本保持不变；更改读取根需创建新目的地。即使任务尚未提交、后来重试或应用重启，也不重新解释其存储位置。
同一下载器里已存在相同 hash，但保存路径与任务快照不一致时，任务报告冲突，不复用错误位置的下载、不自动移动种子。

未启用目的地时继续使用原下载器 `save_path`，保持旧配置可用。旧任务不会自动绑定后来创建的目的地；没有可靠位置的任务核对为 `unchecked`。本阶段没有旧任务目的地补录或文件迁移接口。

```sh
curl -G 'http://127.0.0.1:8080/api/destinations/目的地ID/entries' --data-urlencode 'path=Series'
curl -X POST 'http://127.0.0.1:8080/api/jobs/任务ID/files/check'
curl 'http://127.0.0.1:8080/api/jobs/任务ID/files'
```

首次核对向原下载器读取文件清单，并核实其保存路径与快照一致。清单使用 [qBittorrent 文件接口](https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-%28qBittorrent-5.0%29#get-torrent-contents) 提供的相对文件名和大小，排除优先级为 0 的文件；不从发布标题猜文件名。
清单成功保存后，后续核对直接访问记录的本地路径，下载器离线也能执行。清单不会随下载器外部改名或移动而自动改写；本阶段不提供清单刷新或路径迁移。
核对由上述 POST 接口显式触发，后台下载同步暂不自动扫描文件系统。

| 状态 | 含义 |
| --- | --- |
| `unchecked` | 尚未核对，或未记录目的地、种子元数据/已选文件尚不可用 |
| `present` | 成功访问并找到所有记录的普通文件 |
| `absent` | 成功访问根目录，至少一个记录的文件不存在 |
| `unavailable` | 根目录不可访问、权限不足、路径越界、映射不匹配或首次读取下载器清单失败 |

汇总时 `unavailable` 优先于 `absent`；每个文件分别记录结果。响应含本次尝试时间 `checked_at` 和最后成功核对时间 `last_success_at`，失败不会抹去后者。
存在状态独立于下载状态；文件缺失不会触发重下载或删除。返回预期/观察大小供查看，`present` 不保证大小一致或内容完整，也不将下载标记为完成。
目录浏览最多返回 1000 项，达到上限时 `truncated=true`；单个任务清单上限 5000 项。不读取文件内容、不提供播放。

本地访问通过 [cap-std 的目录能力接口](https://docs.rs/cap-std/4.0.3/cap_std/fs/struct.Dir.html) 限定在配置根目录内。相对路径拒绝绝对地址、`..`、盘符和反斜杠；符号链接不能越出根目录。根目录若离线而仍保留可访问的空挂载点，本阶段不具备挂载状态识别能力。
P2 仅支持进程可访问的本地目录。SMB 应用直连属于 P3，尚未实现。

## 当前规则与恢复行为

- 支持常见 `[03]`、`- 03`、`EP03`、`第03话`、`S01E03` 以及 `03v2` 标题。
- 合集、分数集数、模糊或冲突的集数标记为 `needs_review`；目前可通过调整源或规则解决，尚无人工集数映射接口。
- RSS enclosure、Atom enclosure、磁力链接及 `.torrent` 链接可作为资源入口；尚未覆盖所有站点私有 XML 扩展。
- 支持 BitTorrent v1 磁力链接（十六进制或 Base32）及 v1/hybrid 种子；纯 v2 尚不支持。
- 同一订阅内按资源地址及“季度 + 集数”去重。相同集数有多个版本时采用 RSS 中先出现的条目，尚不自动洗版。
- 保存原始资源标题、地址、识别结果、过滤原因；种子文件按原始 info 字节计算 hash，并保存实际提交的种子数据。
- 下载前先持久化 hash 和提交意图，再调用下载器。超时或重启后先查询 qBittorrent，按 hash 核对，不盲目重复提交。
- 网络失败按 30 秒起、最多一小时的退避间隔重试；实际执行时间还受扫描周期影响。无效种子进入 `failed`，需手动重试。
- 已提交任务若从下载器消失，标记为 `missing`，不会自动重新下载。完成任务记录保留，不因再次扫描而重建。
- RSS/种子响应限制为 4 MiB，请求超时 30 秒。下载器 Cookie 与 RSS 请求使用不同客户端。
- 蜜柑 HTML 上限 8 MiB，封面上限 4 MiB；成功搜索/详情缓存 60 秒、最多 64 项，最多同时执行 4 个蜜柑请求，超限返回 `429`。HTTP 重定向最多 5 次。
- 无搜索结果返回空列表；页面无法识别、访问受限及上游失败返回 `502`，不会伪装成空列表。公开页面结构变化可能需要更新蜜柑适配器。

本地事务不能与下载器 API 组成原子事务。恢复依赖原下载器实例中的稳定 info hash 和下载器去重；同一数据库仅运行一个服务实例。使用全局选择接口切换不会改变恢复目标。

## 结构与验证

```text
src/api.rs          HTTP 接口
src/domain.rs       订阅、资源、任务模型与输入校验
src/parser.rs       标题与集数识别
src/feeds.rs        RSS/Atom 获取、解析、过滤
src/mikan.rs        蜜柑搜索、详情、订阅候选与封面
src/media.rs        本地文件访问、路径校验与媒体数据结构
src/media_store.rs  目的地、版本化映射、文件清单与核对记录持久化
src/storage.rs      SQLite 持久化与去重
src/downloader.rs   种子标识与 qBittorrent 适配
src/service.rs      扫描、任务恢复与后台调度
migrations/        数据库迁移
tests/fixtures/    人工构造的 RSS 测试样本
```

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

集成测试使用回环地址上的模拟蜜柑 / RSS / 代理 / qBittorrent 服务和临时 SQLite 数据库，不连接真实下载器、不下载媒体。
覆盖过滤、保守集数解析、重复扫描、数据库重开、提交成功但响应失败后的恢复、下载器不可用、禁用订阅、任务移除与 API CRUD。
新增覆盖中文搜索编码、不同季度、字幕组 ID、RSS 候选到保存、访问限制页面、封面与缓存、代理认证及跨域重定向、直连例外和代理失败处理。
下载器归属测试覆盖切换时已排队任务、同 hash 不同实例、单实例离线、重试、重启，以及真实旧版 SQLite 数据库升级后的显式归属恢复。
P2 使用临时本地文件与模拟下载器，覆盖目的地/映射版本快照、实际提交路径、同 hash 不同路径冲突、文件四态、成功核对时间保留、下载器离线核对、目录浏览与路径/符号链接限制。没有连接真实 NAS 或下载器。

已实现全局媒体目的地与本地文件存在核对。后续将支持 SMB 应用直连、第二种下载工具、MySQL、管理页面和部署。

## 许可证

本项目按 GNU General Public License version 2 only 发布，完整条款见 [LICENSE](LICENSE)。
新增 Rust 源文件使用 `SPDX-License-Identifier: GPL-2.0-only` 标识。第三方依赖遵循各自许可证。
