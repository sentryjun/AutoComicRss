// SPDX-License-Identifier: GPL-2.0-only
// Real Chromium + Rust HTTP/SQLite. Only the external website/downloader is simulated.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const repo = path.resolve(__dirname, '..');
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function eventually(action) {
  let error;
  for (let i = 0; i < 100; i++) {
    try { return await action(); } catch (e) { error = e; await wait(100); }
  }
  throw error;
}
async function listen(server) { server.listen(0, '127.0.0.1'); await once(server, 'listening'); return server.address().port; }
(async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'autocomic-ui-'));
  let app, browser, logs = '';
  const tasks = new Map();
  let submissions = 0;
  const rss = await fs.readFile(path.join(__dirname, 'fixtures/releases.xml'), 'utf8');
  const external = http.createServer(async (req, res) => {
    const url = new URL(req.url, 'http://localhost');
    const json = value => { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify(value)); };
    if (url.pathname === '/Home/Search') {
      if (url.searchParams.get('searchstr') === 'empty') return res.end('<form action="/Home/Search"><input name="searchstr"></form><div class="central-container"><ul class="list-inline an-ul"></ul></div>');
      res.setHeader('Content-Type', 'text/html; charset=utf-8');
      return res.end('<form action="/Home/Search"><input name="searchstr"></form><div class="central-container"><ul class="list-inline an-ul"><li><a href="/Home/Bangumi/101"><div class="an-text" title="星空列车 &lt;img src=x onerror=alert(1)&gt;">星空列车</div></a></li></ul></div>');
    }
    if (url.pathname === '/Home/Bangumi/101') {
      res.setHeader('Content-Type', 'text/html; charset=utf-8');
      return res.end('<p class="bangumi-title">星空列车<a class="mikan-rss" href="/RSS/Bangumi?bangumiId=101"></a></p><p class="bangumi-info">放送日期：星期五</p><div class="subgroup-text" id="12"><a href="/Home/PublishGroup/99">测试字幕组</a><a class="mikan-rss" href="/RSS/Bangumi?bangumiId=101&amp;subgroupid=12"></a></div>');
    }
    if (url.pathname === '/RSS/Bangumi') { res.setHeader('Content-Type', 'application/rss+xml'); return res.end(rss); }
    if (url.pathname === '/api/v2/auth/login') return res.end('Ok.');
    if (url.pathname === '/api/v2/torrents/info') return json([...tasks.values()]);
    if (url.pathname === '/api/v2/torrents/files') return json([{ name: 'episode.mkv', size: 4, priority: 1 }]);
    if (url.pathname === '/api/v2/torrents/add') {
      let body = ''; for await (const chunk of req) body += chunk;
      const hash = body.match(/urn:btih:([a-f0-9]{40})/)[1];
      const save_path = body.match(/name="savepath"\r\n\r\n([^\r]+)/)[1];
      tasks.set(hash, { hash, save_path, progress: 1, state: 'uploading' }); submissions++;
      return res.end('Ok.');
    }
    res.statusCode = 404; res.end('missing');
  });
  try {
    const externalPort = await listen(external);
    const reservation = http.createServer(); const port = await listen(reservation);
    await new Promise(resolve => reservation.close(resolve));
    const base = `http://127.0.0.1:${port}`;
    const website = `http://127.0.0.1:${externalPort}`;
    const media = path.join(temp, 'media'); await fs.mkdir(media); await fs.mkdir(path.join(media, 'Season 01'));
    await fs.writeFile(path.join(media, 'episode.mkv'), 'test');
    const env = { ...process.env, DATABASE_URL: `sqlite://${temp}/test.db`, APP_PORT: String(port), POLL_INTERVAL_SECONDS: '3600', MIKAN_BASE_URL: website, QBIT_URL: '', WEBSITE_PROXY_URL: '', WEBSITE_DIRECT_HOSTS: '' };
    app = spawn(path.join(repo, 'target/debug/autocomic-rss'), [], { cwd: temp, env, stdio: ['ignore', 'pipe', 'pipe'] });
    app.stdout.on('data', chunk => { logs += chunk; }); app.stderr.on('data', chunk => { logs += chunk; });
    await eventually(async () => assert.equal((await fetch(`${base}/health`)).status, 200));
    const api = async route => (await fetch(`${base}/api${route}`)).json();
    browser = await chromium.launch({ headless: true, ...(process.env.AUTOCOMIC_CHROMIUM ? { executablePath: process.env.AUTOCOMIC_CHROMIUM } : {}) });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    page.setDefaultTimeout(10000);
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    page.on('dialog', async prompt => { assert.equal(prompt.type(), 'confirm'); await prompt.accept(); });
    const button = name => page.getByRole('button', { name, exact: true });
    const modal = page.getByRole('dialog');
    const click = async name => (await modal.isVisible() ? modal : page).getByRole('button', { name, exact: true }).click();
    async function close() { await click('关闭弹窗'); await modal.waitFor({ state: 'hidden' }); }
    async function nav(name) { await page.getByRole('link', { name, exact: false }).click(); await page.getByRole('heading', { name: name === '发现番剧' ? '下一部，想追什么？' : name, exact: true }).waitFor(); }
    async function notice(text) { await eventually(async () => assert.ok((await page.locator('#notice').innerText()).includes(text))); }
    await page.goto(base);
    await page.getByRole('heading', { name: '下一部，想追什么？' }).waitFor();
    await page.getByLabel('番剧搜索关键词').fill('星空'); await click('搜索番剧'); await click('查看字幕组 →');
    await click('使用此字幕组');
    assert.equal(await page.getByLabel('启用自动追番').isChecked(), false);
    assert.equal(await button('保存订阅').isDisabled(), true);
    await page.getByLabel('包含关键词').fill('1080p');
    await page.route('**/api/preview', route => route.fulfill({ status: 502, json: { error: 'fixture temporarily unavailable' } }));
    await click('预览资源'); await page.locator('#dialog-error').waitFor({ state: 'visible' });
    assert.equal(await button('保存订阅').isDisabled(), true);
    await page.unroute('**/api/preview'); await click('预览资源');
    await eventually(async () => assert.equal(await button('保存订阅').isDisabled(), false));
    assert.equal(await page.locator('#dialog-error').isVisible(), false);
    await page.getByLabel('番剧名称').fill('星空列车 <img src=x onerror=alert(1)>');
    assert.equal(await button('保存订阅').isDisabled(), true); // edits invalidate preview
    await click('预览资源'); await eventually(async () => assert.equal(await button('保存订阅').isDisabled(), false));
    assert.equal((await api('/jobs')).length, 0); assert.equal(submissions, 0);
    await click('保存订阅'); await modal.waitFor({ state: 'hidden' });
    assert.equal((await api('/jobs')).length, 0);
    await nav('我的订阅'); assert.equal(await page.locator('#content img').count(), 0); // untrusted title remains text
    assert.ok((await page.locator('#content').innerText()).includes('<img src=x onerror=alert(1)>'));

    await nav('下载器'); await click('添加下载器');
    await page.getByLabel('实例名称').fill('NAS qBittorrent'); await page.getByLabel('服务地址').fill(website);
    await page.getByLabel('用户名', { exact: true }).fill('demo'); await page.getByLabel('密码', { exact: true }).fill('test-secret-never-echo');
    await click('添加实例'); await modal.waitFor({ state: 'hidden' });
    await click('设为全局下载器'); await notice('已切换');
    assert.ok(!JSON.stringify(await api('/downloaders')).includes('test-secret-never-echo'));
    assert.ok(!(await page.locator('body').innerText()).includes('test-secret-never-echo'));
    await click('添加下载器'); assert.equal(await page.getByLabel('密码', { exact: true }).inputValue(), ''); await close();

    await nav('存储目的地'); await click('添加目的地');
    await page.getByLabel('目的地名称').fill('本地媒体'); await page.getByLabel('访问根目录').fill(media);
    await click('添加目的地'); await modal.waitFor({ state: 'hidden' });
    await click('设为全局目的地'); await notice('失败'); // mapping required, selection unchanged
    assert.equal((await api('/status')).active_destination_id, null);
    await click('路径映射'); await page.getByRole('heading', { name: '映射历史' }).waitFor();
    await click('保存新映射版本'); await eventually(async () => assert.ok((await modal.innerText()).includes('版本 1'))); await close();
    await click('设为全局目的地'); await notice('已切换');
    await click('浏览目录'); await click('▸ Season 01'); await page.getByText('此目录为空。').waitFor();
    await click('↑ 上一级'); await page.getByText('episode.mkv', { exact: true }).waitFor(); await close();

    await nav('我的订阅'); await click('启用'); await notice('订阅已启用'); await click('扫描入队'); await notice('1 个新任务');
    await nav('下载任务'); await click('立即扫描并同步'); await notice('扫描');
    await eventually(async () => assert.equal((await api('/jobs'))[0].state, 'submitted'));
    await click('立即扫描并同步'); // completion is observed on the next downloader sync
    await eventually(async () => assert.equal((await api('/jobs'))[0].state, 'completed'));
    assert.equal(submissions, 1);
    await click('核对文件'); await modal.getByText('文件存在', { exact: true }).first().waitFor();
    assert.equal((await api('/jobs'))[0].file_status, 'present'); await close();
    const frozen = (await api('/jobs'))[0];
    await fs.unlink(path.join(media, 'episode.mkv')); await click('核对文件'); await modal.getByText('文件缺失', { exact: true }).first().waitFor(); await close();
    await fs.rename(media, `${media}-offline`); await click('核对文件'); await modal.getByText('无法访问', { exact: true }).first().waitFor(); await close();
    assert.equal((await api('/jobs'))[0].state, 'completed'); assert.equal(submissions, 1);
    await fs.rename(`${media}-offline`, media);
    await page.screenshot({ path: path.join(os.tmpdir(), 'autocomic-ui-desktop.png'), fullPage: true });

    await nav('下载器'); await click('添加下载器');
    await page.getByLabel('实例名称').fill('备用 Transmission'); await page.getByLabel('下载工具').selectOption('transmission');
    await page.getByLabel('服务地址').fill(`${website}/transmission/rpc`); await click('添加实例'); await modal.waitFor({ state: 'hidden' });
    await click('设为全局下载器'); await notice('失败'); // new instance must map current storage
    await nav('存储目的地'); await click('路径映射');
    const other = (await api('/downloaders')).find(d => d.kind === 'transmission');
    await page.getByLabel('下载器实例').selectOption(other.id); await click('保存新映射版本');
    await eventually(async () => assert.ok((await modal.innerText()).includes('备用 Transmission · 版本'))); await close();
    await nav('下载器'); await click('设为全局下载器'); await notice('已切换');
    assert.equal((await api('/status')).active_downloader_id, other.id);
    assert.equal((await api('/jobs'))[0].downloader_id, frozen.downloader_id);

    await nav('存储目的地'); await click('添加目的地'); await page.getByLabel('目的地名称').fill('远程 NAS');
    await page.getByLabel('访问方式').selectOption('smb');
    await page.getByLabel('服务器地址').fill('127.0.0.1'); await page.getByLabel('共享名称').fill('media');
    await page.getByLabel('用户名', { exact: true }).fill('readonly'); await page.getByLabel('密码', { exact: true }).fill('smb-secret');
    await click('添加目的地'); await modal.waitFor({ state: 'hidden' });
    const remoteCard = page.locator('.card').filter({ has: page.getByRole('heading', { name: '远程 NAS' }) });
    await remoteCard.getByRole('button', { name: '路径映射', exact: true }).click();
    await page.getByRole('heading', { name: '映射历史' }).waitFor(); await click('保存新映射版本');
    await eventually(async () => assert.ok((await modal.innerText()).includes('版本 1'))); await close();
    await remoteCard.getByRole('button', { name: '设为全局目的地', exact: true }).click(); await notice('已切换');
    assert.notEqual((await api('/status')).active_destination_id, frozen.destination_id);
    assert.equal((await api('/jobs'))[0].destination_id, frozen.destination_id);
    assert.ok(!JSON.stringify(await api('/destinations')).includes('smb-secret'));

    // A delayed detail response must never replace a newly opened editor.
    await nav('发现番剧'); await page.getByLabel('番剧搜索关键词').fill('星空'); await click('搜索番剧');
    await page.route('**/api/sources/mikan/series/101', async route => { await wait(400); await route.continue(); });
    await click('查看字幕组 →'); await close(); await click('手动添加 RSS'); await wait(600);
    assert.equal(await page.locator('#dialog-title').innerText(), '创建订阅'); await page.getByLabel('番剧名称').fill('保留中的输入');
    assert.equal(await page.getByLabel('番剧名称').inputValue(), '保留中的输入'); await close();
    await page.unroute('**/api/sources/mikan/series/101');
    await click('解析蜜柑链接'); await page.getByLabel('番剧页、字幕组或 RSS 链接').fill(`${website}/Home/Bangumi/101`);
    await click('解析链接'); await click('使用这个 RSS'); await page.getByLabel('番剧名称').waitFor(); await close();

    // Server failure is recoverable and shown as text; no automatic mutation retry.
    await nav('我的订阅'); await page.route('**/api/subscriptions/*', route => route.fulfill({ status: 409, json: { error: 'busy' } }));
    await click('暂停'); await notice('稍后重试'); await page.unroute('**/api/subscriptions/*'); await click('暂停'); await notice('已暂停');
    await page.setViewportSize({ width: 390, height: 844 }); await nav('发现番剧');
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);
    await page.screenshot({ path: path.join(os.tmpdir(), 'autocomic-ui-mobile.png'), fullPage: true });
    await click('手动添加 RSS'); await page.keyboard.press('Escape'); await modal.waitFor({ state: 'hidden' });
    await eventually(async () => assert.equal(await button('手动添加 RSS').evaluate(node => node === document.activeElement), true));
    assert.equal(await page.locator('video,audio').count(), 0);
    assert.deepEqual(errors, []);
    console.log('PASS: discovery, preview, subscription, downloaders, local/SMB configuration, mappings, file states, frozen ownership, errors, XSS text, stale dialogs, mobile and keyboard.');
    console.log(`Screenshots: ${path.join(os.tmpdir(), 'autocomic-ui-desktop.png')}, ${path.join(os.tmpdir(), 'autocomic-ui-mobile.png')}`);
  } catch (error) { console.error(logs.slice(-4000)); throw error; }
  finally {
    if (browser) await browser.close();
    if (app && app.exitCode === null) {
      const exited = once(app, 'exit'); app.kill('SIGTERM');
      const force = setTimeout(() => app.kill('SIGKILL'), 5000);
      await exited; clearTimeout(force);
    }
    external.closeAllConnections(); await new Promise(resolve => external.close(resolve));
    await fs.rm(temp, { recursive: true, force: true });
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
