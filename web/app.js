// SPDX-License-Identifier: GPL-2.0-only
const $ = (selector, root = document) => root.querySelector(selector);
const content = $('#content');
const dialog = $('#dialog');
let dialogVersion = 0;
let viewVersion = 0;
let lastActionControl;
let dialogOpener;
let data = { status: {}, downloaders: [], destinations: [] };
const pages = { discover: '发现番剧', subscriptions: '我的订阅', jobs: '下载任务', downloaders: '下载器', storage: '存储目的地' };
const labels = { queued: '等待提交', submitting: '正在提交', submitted: '已提交', downloading: '下载中', completed: '下载完成', missing: '远端任务缺失', failed: '下载失败', unchecked: '尚未核对', present: '文件存在', absent: '文件缺失', unavailable: '无法访问', ready: '可下载', filtered: '已过滤', needs_review: '待确认', already_queued: '已入队' };

// All source titles, paths and errors are text nodes, never HTML.
function el(tag, text, className) {
  const node = document.createElement(tag);
  if (text != null) node.textContent = String(text);
  if (className) node.className = className;
  return node;
}
function box(className, ...children) {
  const node = el('div', null, className);
  node.append(...children.filter(Boolean));
  return node;
}
function note(text) { return el('p', text, 'hint'); }
function empty(text) { return el('p', text, 'empty'); }
function badge(state) {
  return el('span', labels[state] || state, `badge ${['failed', 'missing', 'absent', 'unavailable'].includes(state) ? 'bad' : ['present', 'completed'].includes(state) ? 'good' : ''}`);
}
function notify(message, error = false) {
  const node = $('#notice');
  node.textContent = message;
  node.className = error ? 'error' : '';
  node.hidden = false;
}
function showError(error, version) {
  if (dialog.open && version === dialogVersion) {
    $('#dialog-error').textContent = error.message;
    $('#dialog-error').hidden = false;
  } else notify(error.message, true);
}
async function perform(control, action) {
  if (control.disabled) return;
  lastActionControl = control;
  const version = dialog.open ? dialogVersion : -1;
  if (dialog.open) $('#dialog-error').hidden = true;
  control.disabled = true;
  try { await action(); } catch (error) { showError(error, version); }
  finally { control.disabled = false; }
}
function button(text, action, className = '') {
  const node = el('button', text, className);
  node.type = 'button';
  node.addEventListener('click', () => perform(node, action));
  return node;
}
async function api(path, method = 'GET', body) {
  let response;
  try {
    response = await fetch(`/api${path}`, {
      method, credentials: 'same-origin', cache: 'no-store',
      headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch { throw new Error('连接中断。请检查服务状态；写入结果可能尚未确认，请先刷新列表，避免重复操作。'); }
  if (response.status === 204) return null;
  const text = await response.text();
  let result;
  try { result = JSON.parse(text); } catch { result = null; }
  if (!response.ok) {
    const prefix = response.status === 409 ? '后台正在处理任务，请稍后重试。' : `请求失败（${response.status}）。`;
    throw new Error(`${prefix}${result?.error || '请检查输入或服务日志。'}`);
  }
  if (result === null) throw new Error('服务返回了无法识别的响应，请刷新后重试。');
  return result;
}
async function modalApi(version, ...args) {
  try { return await api(...args); }
  catch (error) {
    if (version === dialogVersion && dialog.open) {
      $('#dialog-content').replaceChildren(note('读取未完成，请关闭后重试。'));
      showError(error, version);
    }
    return null;
  }
}
function field(form, label, name, value = '', type = 'text', required = true) {
  const wrap = el('label', null, 'field');
  wrap.append(el('span', label));
  const input = el(type === 'textarea' ? 'textarea' : 'input');
  if (type !== 'textarea') input.type = type;
  input.name = name;
  input.value = value ?? '';
  input.required = required;
  if (type === 'password') input.autocomplete = 'new-password';
  wrap.append(input);
  form.append(wrap);
  return input;
}
function select(form, label, name, choices, value) {
  const wrap = el('label', null, 'field');
  wrap.append(el('span', label));
  const input = el('select');
  input.name = name;
  for (const [key, title] of choices) {
    const option = el('option', title);
    option.value = key;
    input.append(option);
  }
  if (value != null) input.value = value;
  wrap.append(input);
  form.append(wrap);
  return input;
}
function submit(form, text, action) {
  const control = el('button', text, 'primary');
  control.type = 'submit';
  form.addEventListener('submit', event => { event.preventDefault(); perform(control, action); });
  return control;
}
function values(form) { return Object.fromEntries(new FormData(form)); }
function modal(title) {
  if (!dialog.open) dialogOpener = lastActionControl || document.activeElement;
  dialogVersion++;
  $('#dialog-title').textContent = title;
  $('#dialog-error').hidden = true;
  $('#dialog-content').replaceChildren();
  if (!dialog.open) dialog.showModal();
  return $('#dialog-content');
}
function closeModal() { dialog.close(); }
dialog.addEventListener('close', () => {
  if (!dialog.open) {
    dialogVersion++;
    $('#dialog-content').replaceChildren();
    (dialogOpener?.isConnected ? dialogOpener : $('#main')).focus();
  }
});
async function finishForm(form, message) {
  form.reset();
  if (dialog.contains(form)) closeModal();
  notify(message);
  await navigate();
}
$('#close-dialog').addEventListener('click', closeModal);
function time(value) { return value == null ? '尚无记录' : new Date(value * 1000).toLocaleString('zh-CN'); }
function owner(id) { return data.downloaders.find(item => item.id === id)?.name || id || '尚未分配'; }
function destination(id) { return data.destinations.find(item => item.id === id)?.name || id || '未设置'; }
function intro(title, description, eyebrow = 'AUTOCOMIC / YOUR COLLECTION') {
  return box('intro', el('span', eyebrow, 'eyebrow'), el('h1', title), el('p', description));
}
async function loadSettings() {
  const [status, downloaders, destinations] = await Promise.all([api('/status'), api('/downloaders'), api('/destinations')]);
  data = { status, downloaders, destinations };
  $('#version').textContent = `AutoComicRss · v${status.version}`;
}
async function navigate() {
  const version = ++viewVersion;
  const page = location.hash.slice(1) in pages ? location.hash.slice(1) : 'discover';
  for (const link of document.querySelectorAll('nav a')) {
    if (link.hash === `#${page}`) link.setAttribute('aria-current', 'page');
    else link.removeAttribute('aria-current');
  }
  $('#section-label').textContent = pages[page];
  content.replaceChildren(empty('正在加载…'));
  try {
    await loadSettings();
    const node = await views[page]();
    if (version === viewVersion) content.replaceChildren(node);
  } catch (error) {
    if (version === viewVersion) content.replaceChildren(empty(error.message), button('重新连接', navigate));
  }
}

function releaseList(releases) {
  const list = box('result-list');
  if (!releases.length) list.append(empty('订阅源中暂时没有资源。'));
  for (const release of releases) {
    list.append(box('row', box('row-title', badge(release.decision), el('span', release.title)),
      el('div', `集数：${release.episode ?? '无法识别'} · ${release.reason || ''}`, 'meta')));
  }
  return list;
}
function subscriptionEditor(initial = {}, id) {
  const body = modal(id ? '编辑订阅' : '创建订阅');
  const form = el('form');
  field(form, '番剧名称', 'title', initial.title).maxLength = 200;
  field(form, 'RSS 地址', 'rss_url', initial.rss_url, 'url');
  const pair = box('fields');
  const season = field(pair, '季数', 'season', initial.season ?? 1, 'number');
  season.min = 1; season.max = 999;
  const offset = field(pair, '集数偏移', 'episode_offset', initial.episode_offset ?? 0, 'number');
  offset.min = -10000; offset.max = 10000;
  form.append(pair);
  field(form, '包含关键词（每行一个，留空表示不限）', 'include', (initial.include || []).join('\n'), 'textarea', false);
  field(form, '排除关键词（每行一个）', 'exclude', (initial.exclude || []).join('\n'), 'textarea', false);
  const check = el('input'); check.type = 'checkbox'; check.name = 'enabled'; check.checked = initial.enabled ?? false;
  const checkLabel = el('label', null, 'check'); checkLabel.append(check, el('span', '启用自动追番'));
  form.append(checkLabel, note('先预览筛选结果再保存。启用后，后台会定时扫描并提交下载；暂停状态只保存订阅。预览不会创建任务。'));
  const previewArea = box('');
  let previewed = '';
  const payload = () => {
    const v = values(form);
    return { title: v.title, rss_url: v.rss_url, season: Number(v.season), episode_offset: Number(v.episode_offset), include: v.include.split('\n').map(s => s.trim()).filter(Boolean), exclude: v.exclude.split('\n').map(s => s.trim()).filter(Boolean), enabled: check.checked };
  };
  const signature = () => JSON.stringify({ ...payload(), enabled: false });
  const save = submit(form, id ? '保存修改' : '保存订阅', async () => {
    if (!previewed || previewed !== signature()) throw new Error('规则已变化，请先重新预览。');
    await api(id ? `/subscriptions/${id}` : '/subscriptions', id ? 'PUT' : 'POST', payload());
    await finishForm(form, '订阅已保存。');
  });
  save.disabled = true;
  form.addEventListener('input', () => { save.disabled = !previewed || previewed !== signature(); });
  const preview = button('预览资源', async () => {
    if (!form.reportValidity()) return;
    const current = signature();
    previewArea.replaceChildren(note('正在读取 RSS…'));
    let releases;
    try { releases = await api('/preview', 'POST', payload()); }
    catch (error) { previewArea.replaceChildren(note('预览失败，请检查 RSS 地址后重试。')); throw error; }
    if (current !== signature()) { previewArea.replaceChildren(note('规则已变化，请重新预览。')); return; }
    previewed = current; save.disabled = false;
    previewArea.replaceChildren(note(`读取到 ${releases.length} 项资源；此操作没有创建下载任务。`), releaseList(releases));
  });
  form.append(box('actions', preview, save), previewArea);
  body.append(form);
}
async function discover() {
  const root = box('', intro('下一部，想追什么？', '从蜜柑计划发现番剧，选择字幕组，把更新交给订阅。', 'DISCOVER / 发现与订阅'));
  const form = el('form', null, 'search');
  const input = el('input'); input.type = 'search'; input.required = true;
  input.placeholder = '输入番剧名称，搜索蜜柑计划'; input.setAttribute('aria-label', '番剧搜索关键词');
  const results = box('result-list');
  let searchVersion = 0;
  const search = submit(form, '搜索番剧', async () => {
    const version = ++searchVersion;
    results.replaceChildren(empty('正在搜索蜜柑计划…'));
    try {
      const result = await api(`/sources/mikan/search?q=${encodeURIComponent(input.value.trim())}`);
      if (version !== searchVersion) return;
      results.replaceChildren(box('toolbar', el('h2', '搜索结果'), el('span', `${result.series.length} 部番剧`, 'count')));
      if (!result.series.length) results.append(empty('没有找到番剧，试试其他名称。'));
      const cards = box('grid');
      for (const series of result.series) {
        const card = box('card');
        if (series.cover_url) {
          const image = el('img'); image.className = 'poster'; image.alt = ''; image.loading = 'lazy'; image.src = series.cover_url;
          card.append(image);
        }
        card.append(el('h3', series.title), note('蜜柑计划'), button('查看字幕组 →', () => seriesDetail(series.series_id)));
        cards.append(card);
      }
      results.append(cards);
      if (result.keyword_rss_url) results.append(box('panel soft', el('h3', '订阅这个关键词？'), note('关键词 RSS 可能混入其他番剧，建议优先选择具体番剧和字幕组，并预览筛选结果。'),
        button('使用关键词 RSS', () => subscriptionEditor({ title: result.query, rss_url: result.keyword_rss_url }))));
    } catch (error) { results.replaceChildren(empty(error.message)); }
  });
  form.append(input, search);
  root.append(form, box('actions', button('手动添加 RSS', () => subscriptionEditor()), button('解析蜜柑链接', resolveLink)),
    box('stats', el('span', `全局下载器：${owner(data.status.active_downloader_id)}`), el('span', `存储目的地：${destination(data.status.active_destination_id)}`)),
    box('panel soft', el('h3', '从发现到收藏，只需三步'), note('搜索番剧 → 选择字幕组和筛选规则 → 预览并保存订阅。\n首次使用请先配置下载器，再为存储目的地添加对应的写入路径映射。')), results);
  return root;
}
async function seriesDetail(id) {
  const body = modal('番剧与字幕组');
  const version = dialogVersion;
  body.append(note('正在读取详情…'));
  const detail = await modalApi(version, `/sources/mikan/series/${encodeURIComponent(id)}`);
  if (!detail || version !== dialogVersion) return;
  body.replaceChildren(el('h3', detail.title), note(detail.information.join('\n')),
    button('订阅全部字幕组', () => subscriptionEditor({ title: detail.title, rss_url: detail.rss_url })));
  body.append(note('选择一个字幕组，减少同一集的重复候选资源。'));
  for (const group of detail.groups) body.append(box('row', el('h3', group.name), button('使用此字幕组', () => subscriptionEditor({ title: detail.title, rss_url: group.rss_url }))));
  if (!detail.groups.length) body.append(empty('此番剧暂未发现字幕组，可使用番剧 RSS。'));
}
function resolveLink() {
  const body = modal('解析蜜柑链接'); const form = el('form');
  field(form, '番剧页、字幕组或 RSS 链接', 'url', '', 'url');
  const result = box('');
  form.append(submit(form, '解析链接', async () => {
    const candidate = await api('/sources/mikan/resolve', 'POST', values(form));
    result.replaceChildren(note(candidate.rss_url), ...candidate.warnings.map(note),
      button('使用这个 RSS', () => subscriptionEditor(candidate.subscription || { rss_url: candidate.rss_url })));
  }), result); body.append(form);
}
async function subscriptions() {
  const subscriptions = await api('/subscriptions');
  const root = box('', intro('我的订阅', '管理追番规则与更新节奏。暂停或删除订阅会保留已有下载任务。'),
    box('toolbar', el('span', `${subscriptions.length} 个订阅`, 'count'), button('添加 RSS 订阅', () => subscriptionEditor(), 'primary')));
  const grid = box('grid');
  for (const sub of subscriptions) {
    grid.append(box('card', box('card-head', el('h3', sub.title), badge(sub.enabled ? '自动追番' : '已暂停')),
      note(`第 ${sub.season} 季 · 集数偏移 ${sub.episode_offset}\n最后扫描：${time(sub.last_checked_at)}`),
      sub.last_error ? el('p', sub.last_error, 'meta') : null,
      box('actions', button('编辑规则', () => subscriptionEditor(sub, sub.id)),
        button(sub.enabled ? '暂停' : '启用', async () => {
          const { title, rss_url, season, episode_offset, include, exclude } = sub;
          await api(`/subscriptions/${sub.id}`, 'PUT', { title, rss_url, season, episode_offset, include, exclude, enabled: !sub.enabled });
          notify(sub.enabled ? '订阅已暂停。' : '订阅已启用，后台将自动扫描并提交下载。'); await navigate();
        }), button('预览', async () => {
          const body = modal('订阅资源预览'); const version = dialogVersion;
          body.append(note('正在读取…'));
          const items = await modalApi(version, `/subscriptions/${sub.id}/preview`);
          if (items && version === dialogVersion) body.replaceChildren(releaseList(items));
        }), sub.enabled ? button('扫描入队', async () => {
          const result = await api(`/subscriptions/${sub.id}/scan`, 'POST'); notify(`已将 ${result.queued} 个新任务入队，后台会继续提交下载。`); await navigate();
        }) : null, button('删除', async () => {
          if (!confirm(`删除订阅“${sub.title}”？已有下载任务和文件将保留。`)) return;
          await api(`/subscriptions/${sub.id}`, 'DELETE'); notify('订阅已删除，已有任务保留。'); await navigate();
        }, 'danger'))));
  }
  root.append(subscriptions.length ? grid : empty('还没有订阅。去「发现番剧」搜索，或直接添加 RSS。'));
  return root;
}
async function showFiles(job, check = false) {
  const body = modal('任务文件核对'); const version = dialogVersion;
  body.append(note(check ? '正在读取下载器文件清单并核对存储…' : '正在读取最近的核对记录…'));
  const report = await modalApi(version, `/jobs/${job.id}/files${check ? '/check' : ''}`, check ? 'POST' : 'GET');
  if (!report || version !== dialogVersion) return;
  body.replaceChildren(el('h3', job.title), badge(report.status), note(`目的地：${destination(report.destination_id)}\n本次核对：${time(report.checked_at)}\n最后成功访问：${time(report.last_success_at)}`));
  if (report.error) body.append(note(report.error));
  body.append(note('文件缺失或无法访问不会触发重下载，也不会改变下载完成状态。'), button('重新核对文件', () => showFiles(job, true)));
  for (const file of report.files) body.append(box('row', badge(file.status), el('p', file.relative_path, 'file-path'),
    note(`预期大小：${file.expected_size} 字节 · 当前大小：${file.observed_size ?? '未知'}${file.error ? `\n${file.error}` : ''}`)));
  if (!report.files.length) body.append(empty('尚无可核对的文件清单。任务需有存储目的地，且原下载器已取得文件元数据。'));
  if (check) await navigate();
}
async function jobs() {
  const jobs = await api('/jobs');
  const root = box('', intro('下载任务', '每个任务保留原下载器和存储位置。文件核对仅检查是否存在，不播放或移动媒体。'));
  root.append(box('toolbar', el('span', `最近 ${jobs.length} 个任务（最多 500 个）`, 'count'), button('立即扫描并同步', async () => {
    const result = await api('/run', 'POST');
    notify(`扫描 ${result.scanned} 个订阅，新入队 ${result.queued} 个任务。${result.errors.length ? `\n${result.errors.join('\n')}` : ''}`, result.errors.length > 0);
    await navigate();
  }, 'primary')), note('“立即扫描并同步”会扫描已启用的订阅、提交队列并同步下载状态。'));
  const filterBox = box('');
  const filter = select(filterBox, '筛选下载状态', 'state', [['', '全部任务'], ...['queued', 'submitting', 'submitted', 'downloading', 'completed', 'missing', 'failed'].map(key => [key, labels[key]])]);
  const list = box('result-list');
  function render() {
    const visible = jobs.filter(job => !filter.value || job.state === filter.value);
    list.replaceChildren();
    for (const job of visible) list.append(box('card', box('card-head', el('h3', job.title), badge(job.state)),
      note(`第 ${job.season} 季 · 第 ${job.episode} 集\n原下载器：${owner(job.downloader_id)} · 存储：${destination(job.destination_id)}`),
      el('div', job.download_save_path || '尚未记录下载路径', 'file-path'),
      box('actions', el('span', '文件：', 'meta'), badge(job.file_status)),
      job.last_error ? note(job.last_error) : null,
      box('actions', button('核对记录', () => showFiles(job)), button('核对文件', () => showFiles(job, true)),
        ['failed', 'missing'].includes(job.state) ? button('重试原下载器', async () => {
          if (!confirm('按原下载器和原保存路径重试此任务？')) return;
          await api(`/jobs/${job.id}/retry`, 'POST'); notify('已安排重试，任务归属保持不变。'); await navigate();
        }) : null)));
    if (!visible.length) list.append(empty('没有符合条件的任务。启用订阅后，新的资源会出现在这里。'));
  }
  filter.addEventListener('change', render); render(); root.append(filterBox, list); return root;
}
function createDownloader() {
  const body = modal('添加下载器'); const form = el('form');
  field(form, '实例名称', 'name');
  const kind = select(form, '下载工具', 'kind', [['qbittorrent', 'qBittorrent'], ['transmission', 'Transmission']]);
  const url = field(form, '服务地址', 'url', '', 'url'); url.placeholder = 'http://127.0.0.1:8081';
  kind.addEventListener('change', () => { url.placeholder = kind.value === 'transmission' ? 'http://127.0.0.1:9091/transmission/rpc' : 'http://127.0.0.1:8081'; });
  field(form, '用户名', 'username', '', 'text', false).autocomplete = 'username';
  field(form, '密码', 'password', '', 'password', false);
  field(form, '默认保存路径（下载器看到的路径）', 'save_path', '/downloads');
  form.append(note('Transmission 请填写完整 RPC 地址。实例创建后连接信息保持不变；更换地址或凭据时请新增实例。'), submit(form, '添加实例', async () => {
    const { name, kind, ...connection } = values(form);
    await api('/downloaders', 'POST', { name, kind, connection });
    await finishForm(form, '下载器已添加，请选择全局实例。');
  })); body.append(form);
}
async function downloaders() {
  const root = box('', intro('下载器', '设置一个全局实例接收新任务，旧任务继续由原下载器管理。'),
    box('toolbar', note('切换前请为新实例配置当前全局存储目的地的路径映射。'), button('添加下载器', createDownloader, 'primary')));
  const grid = box('grid');
  for (const item of data.downloaders) grid.append(box('card', box('card-head', el('h3', item.name), badge(item.is_active ? '全局使用中' : item.kind)),
    note(`${item.kind}\n${item.url}\n默认保存路径：${item.save_path}\n账号：${item.username || '未设置'} · 密码：${item.password_configured ? '已配置' : '未配置'}`),
    box('actions', item.is_active ? null : button('设为全局下载器', async () => {
      await api('/downloaders/active', 'PUT', { downloader_id: item.id }); notify('全局下载器已切换，旧任务保持原归属。'); await navigate();
    }), data.status.legacy_unassigned_jobs > 0 && item.kind === 'qbittorrent' ? button('恢复旧任务归属', async () => {
      if (!confirm(`仅当“${item.name}”确实是旧任务原来的 qBittorrent 时才继续。将为 ${data.status.legacy_unassigned_jobs} 个历史任务恢复归属，确定吗？`)) return;
      const result = await api(`/downloaders/${item.id}/claim-legacy-jobs`, 'POST'); notify(`已恢复 ${result.claimed} 个旧任务的归属。`); await navigate();
    }) : null)));
  root.append(data.downloaders.length ? grid : empty('尚未配置下载器。添加 qBittorrent 或 Transmission 后，选择全局实例。'));
  if (data.status.legacy_unassigned_jobs) root.append(note(`有 ${data.status.legacy_unassigned_jobs} 个历史任务等待确认原下载器，系统不会自动重新下载它们。`));
  return root;
}
function createDestination() {
  const body = modal('添加存储目的地'); const form = el('form');
  field(form, '目的地名称', 'name');
  const kind = select(form, '访问方式', 'kind', [['local', '本地目录'], ['smb', 'SMB 直连']]);
  const root = field(form, '访问根目录', 'root_path'); root.placeholder = '/mnt/media';
  const smb = el('fieldset'); smb.hidden = true; smb.disabled = true;
  smb.append(el('legend', 'SMB 连接'));
  field(smb, '服务器地址（域名或 IP）', 'server');
  const port = field(smb, '端口', 'port', 445, 'number'); port.min = 1; port.max = 65535;
  field(smb, '共享名称', 'share'); field(smb, '用户名', 'username').autocomplete = 'username';
  field(smb, '密码', 'password', '', 'password'); field(smb, '域（可选）', 'domain', '', 'text', false);
  const timeout = field(smb, '连接超时（秒）', 'timeout_seconds', 15, 'number'); timeout.min = 1; timeout.max = 60;
  kind.addEventListener('change', () => {
    smb.hidden = kind.value !== 'smb'; smb.disabled = smb.hidden;
    root.required = smb.hidden; root.value = ''; root.placeholder = smb.hidden ? '/mnt/media' : '共享内相对目录，留空为共享根';
  });
  form.append(smb, note('本地目录填写应用可访问的绝对路径；SMB 根目录填写共享内相对路径。建议使用只读共享账号。创建后请添加下载器路径映射，再设为全局。'), submit(form, '添加目的地', async () => {
    const v = values(form); const input = { name: v.name, kind: v.kind, root_path: v.root_path };
    if (v.kind === 'smb') input.smb = { server: v.server, port: Number(v.port), share: v.share, username: v.username, password: v.password, domain: v.domain, timeout_seconds: Number(v.timeout_seconds) };
    await api('/destinations', 'POST', input);
    await finishForm(form, '目的地已添加。下一步请配置下载器写入路径映射。');
  })); body.append(form);
}
async function mappings(item) {
  const body = modal(`路径映射 · ${item.name}`); const version = dialogVersion;
  body.append(note('正在加载…'));
  const items = await modalApi(version, `/destinations/${item.id}/mappings`);
  if (!items || version !== dialogVersion) return;
  body.replaceChildren(note(`应用读取：${item.root_path || '共享根目录'}\n填写下载器访问同一目录时使用的绝对路径。修改会增加版本，已有任务继续使用原版本。`));
  if (data.downloaders.length) {
    const form = el('form');
    select(form, '下载器实例', 'downloader_id', data.downloaders.map(d => [d.id, d.name]), data.status.active_downloader_id);
    field(form, '下载器写入根路径', 'download_root', '/downloads');
    form.append(submit(form, '保存新映射版本', async () => {
      await api(`/destinations/${item.id}/mappings`, 'POST', values(form)); notify('路径映射已保存，旧任务仍使用原版本。');
      if (version === dialogVersion) await mappings(item);
    })); body.append(form);
  } else body.append(empty('请先添加下载器实例。'));
  body.append(el('h3', '映射历史'));
  for (const mapping of items) body.append(box('row', el('div', `${owner(mapping.downloader_id)} · 版本 ${mapping.version}`, 'row-title'), el('div', mapping.download_root, 'file-path')));
  if (!items.length) body.append(note('暂无映射。'));
}
async function browse(item, path = '') {
  const body = modal(`浏览目录 · ${item.name}`); const version = dialogVersion;
  body.append(note('正在读取目录…'));
  const listing = await modalApi(version, `/destinations/${item.id}/entries?path=${encodeURIComponent(path)}`);
  if (!listing || version !== dialogVersion) return;
  body.replaceChildren(el('p', `/${path}`, 'file-path'), note('只读目录浏览，不提供播放、移动或删除。'));
  if (path) body.append(button('↑ 上一级', () => browse(item, path.split('/').slice(0, -1).join('/'))));
  if (listing.error) body.append(empty(`无法读取目录：${listing.error}`));
  for (const entry of listing.entries) body.append(box('directory', entry.kind === 'directory' ? button(`▸ ${entry.name}`, () => browse(item, [path, entry.name].filter(Boolean).join('/')), 'quiet') : el('span', entry.name, 'file-path'), el('span', entry.size == null ? entry.kind : `${entry.size} 字节`, 'meta')));
  if (!listing.error && !listing.entries.length) body.append(empty('此目录为空。'));
  if (listing.truncated) body.append(note('目录条目过多，当前仅显示部分结果。'));
}
async function storage() {
  const root = box('', intro('存储目的地', '所有番剧共用一个全局目的地，支持本地目录与应用直连 SMB。'),
    box('toolbar', note('切换只影响新任务；旧任务仍按创建时的位置核对文件。'), button('添加目的地', createDestination, 'primary')));
  const grid = box('grid');
  for (const item of data.destinations) grid.append(box('card', box('card-head', el('h3', item.name), badge(item.is_active ? '全局使用中' : item.kind === 'smb' ? 'SMB' : '本地目录')),
    note(item.smb ? `${item.smb.server}:${item.smb.port} / ${item.smb.share}\n根目录：${item.root_path || '/'}\n账号：${item.smb.username} · 密码：${item.smb.password_configured ? '已配置' : '未配置'}` : item.root_path),
    box('actions', button('路径映射', () => mappings(item)), button('浏览目录', () => browse(item)), item.is_active ? null : button('设为全局目的地', async () => {
      await api('/destinations/active', 'PUT', { destination_id: item.id }); notify('全局目的地已切换，旧任务仍使用原位置。'); await navigate();
    }))));
  root.append(data.destinations.length ? grid : empty('尚未设置存储目的地。配置后，新任务将保存目的地和路径映射，用于文件存在核对。'));
  return root;
}
const views = { discover, subscriptions, jobs, downloaders, storage };
$('#refresh').addEventListener('click', () => perform($('#refresh'), navigate));
window.addEventListener('hashchange', () => { if (dialog.open) closeModal(); navigate(); });
await navigate();
