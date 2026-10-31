// 服务器列表页：添加、编辑、删除服务器，选中后跳到那个服务器的网页端。服务器和令牌只存在
// 这台设备上（这个页面的 localStorage），服务器的页面是别的源，读不到。
import { fetchInfo, normalizeUrl, openTarget, rememberPage, upsert } from './servers.js';

const SERVERS_KEY = 'shuku.servers';
const CURRENT_KEY = 'shuku.current';
/** 启动时自动打开上次的服务器只做一次，从服务器返回这里时不再自动跳走。 */
const STARTED_KEY = 'shuku.started';

const $ = (id) => /** @type {HTMLElement} */ (document.getElementById(id));

/** @type {import('./servers.js').Server[]} */
let servers = JSON.parse(localStorage.getItem(SERVERS_KEY) ?? '[]');
let current = localStorage.getItem(CURRENT_KEY);
let editing = false;
/** 表单正在改的服务器，添加时为 `null`。 */
let formServer = null;
/** 表单里最后一次读到的服务器信息和对应的地址。 */
let checked = null;
/**
 * 正在确认在线、确认后就打开的服务器（启动时自动打开或者点了列表里的一项）。用户接着做了
 * 别的操作就作废，不从他手里跳走。
 */
let pendingOpen = null;

function persist() {
	localStorage.setItem(SERVERS_KEY, JSON.stringify(servers));
	if (current) localStorage.setItem(CURRENT_KEY, current);
	else localStorage.removeItem(CURRENT_KEY);
}

function open(server) {
	pendingOpen = null;
	current = server.id;
	persist();
	location.assign(openTarget(server));
}

function showList() {
	$('form').hidden = true;
	$('list').hidden = false;
	$('toggle-edit').hidden = servers.length === 0;
	$('toggle-edit').textContent = editing ? '完成' : '编辑';
	const list = $('servers');
	list.replaceChildren(...servers.map(serverItem));
}

function serverItem(server) {
	const item = document.createElement('li');
	const button = document.createElement('button');
	button.type = 'button';
	button.className = server.id === current ? 'server current' : 'server';
	const badge = span('badge', Array.from(server.name)[0] ?? '?');
	badge.setAttribute('aria-hidden', 'true');
	const state = span('state', '正在连接…');
	const text = span('text');
	text.append(span('title', server.name), span('address', server.url), state);
	button.append(badge, text);
	if (editing) button.append(span('trailing', '修改'));
	else if (server.id === current) button.append(span('trailing', '当前'));
	// 打开前再确认一次在线：窗口跳到连不上的地址后只剩出错页，回不到这个列表。
	button.addEventListener('click', () => {
		if (editing) return showForm(server);
		pendingOpen = server;
		button.disabled = true;
		probe(server, state).then((online) => {
			button.disabled = false;
			if (online && pendingOpen === server) open(server);
		});
	});
	item.append(button);
	void probe(server, state);
	return item;
}

/** 读服务器信息并显示在线状态，返回是否在线。 */
async function probe(server, state) {
	try {
		const info = await fetchInfo(server.url);
		state.className = 'state online';
		state.textContent = `在线 · ${info.archives.toLocaleString('zh-CN')} 本`;
		return true;
	} catch (error) {
		state.className = 'state offline';
		state.textContent = error.message;
		return false;
	}
}

/** `crypto.randomUUID()` 只在安全上下文里有，`tauri://` 不一定算。 */
function newId() {
	return Date.now().toString(36) + Math.random().toString(36).slice(2);
}

function span(className, text = '') {
	const element = document.createElement('span');
	element.className = className;
	element.textContent = text;
	return element;
}

function showForm(server) {
	pendingOpen = null;
	formServer = server;
	checked = null;
	$('list').hidden = true;
	$('form').hidden = false;
	$('back').hidden = servers.length === 0;
	$('form-title').textContent = server ? '修改服务器' : '添加服务器';
	/** @type {HTMLInputElement} */ ($('url')).value = server?.url ?? '';
	/** @type {HTMLInputElement} */ ($('token')).value = server?.token ?? '';
	/** @type {HTMLInputElement} */ ($('name')).value = server?.name ?? '';
	$('token-field').hidden = !server?.token;
	$('remove').hidden = !server;
	$('remove').textContent = '删除这个服务器';
	delete $('remove').dataset.armed;
	$('status').hidden = true;
	if (server) void check();
	else $('url').focus();
}

/** 按表单里的地址读服务器信息，显示状态，需要令牌时露出令牌一栏。 */
async function check() {
	let origin;
	try {
		origin = normalizeUrl(/** @type {HTMLInputElement} */ ($('url')).value);
	} catch (error) {
		setStatus(true, error.message, '');
		checked = null;
		return null;
	}
	if (checked?.origin === origin) return checked;
	setStatus(false, '正在连接…', '');
	try {
		const info = await fetchInfo(origin);
		checked = { origin, info };
		setStatus(
			false,
			`已连上 Shuku ${info.version}`,
			`书库里 ${info.archives.toLocaleString('zh-CN')} 本。` +
				(info.require_token ? '这个服务器要求令牌。' : '这个服务器不要求令牌。')
		);
		if (info.require_token) $('token-field').hidden = false;
		const name = /** @type {HTMLInputElement} */ ($('name'));
		if (!name.value.trim()) name.value = info.name;
		return checked;
	} catch (error) {
		checked = null;
		setStatus(true, error.message, '检查地址和端口，以及服务器是否在运行。');
		return null;
	}
}

function setStatus(failed, title, detail) {
	const status = $('status');
	status.hidden = false;
	status.classList.toggle('error', failed);
	const strong = document.createElement('strong');
	strong.textContent = title;
	status.replaceChildren(strong);
	if (detail) status.append(span('', detail));
}

async function save(event) {
	event.preventDefault();
	const button = /** @type {HTMLButtonElement} */ ($('save'));
	button.disabled = true;
	// 连接检查期间可能返回列表、改去编辑另一台服务器。检查完以后表单已经不是这一台时放弃，
	// 免得把另一台的令牌和这一台的地址存在一起。
	const target = formServer;
	try {
		const result = await check();
		if (!result || formServer !== target || $('form').hidden) return;
		const token = /** @type {HTMLInputElement} */ ($('token')).value.trim() || null;
		if (result.info.require_token && !token) {
			setStatus(true, '这个服务器要求令牌', '在控制台的“设置 › 设备令牌”里新建一个，填到上面。');
			$('token').focus();
			return;
		}
		const name = /** @type {HTMLInputElement} */ ($('name')).value.trim() || result.info.name;
		if (normalizeUrl(/** @type {HTMLInputElement} */ ($('url')).value) !== result.origin) return;
		const server = {
			id: formServer?.id ?? newId(),
			name,
			url: result.origin,
			token,
			// 换了地址，上次的页面就不算了。
			lastUrl: formServer?.url === result.origin ? formServer.lastUrl : null
		};
		servers = upsert(servers, server);
		open(server);
	} finally {
		button.disabled = false;
	}
}

function remove() {
	const button = $('remove');
	// 原生的 confirm() 在 Android 的 WebView 里不一定有，改成再点一次。
	if (button.dataset.armed !== 'true') {
		button.dataset.armed = 'true';
		button.textContent = '再点一次删除';
		return;
	}
	delete button.dataset.armed;
	servers = servers.filter((server) => server.id !== formServer.id);
	if (current === formServer.id) current = null;
	persist();
	editing = servers.length > 0 && editing;
	if (servers.length === 0) showForm(null);
	else showList();
}

function start() {
	const from = new URLSearchParams(location.search).get('from');
	if (from) {
		const remembered = rememberPage(servers, from);
		servers = remembered.servers;
		current = remembered.current ?? current;
		persist();
		history.replaceState(null, '', location.pathname);
	}
	const last = servers.find((server) => server.id === current);
	const autoOpen = !from && !sessionStorage.getItem(STARTED_KEY) && last;
	sessionStorage.setItem(STARTED_KEY, '1');
	if (servers.length === 0) {
		showForm(null);
	} else if (autoOpen) {
		// 先确认服务器在线。不在线时停在列表上，不然窗口里只剩一张出错的页面，回不到这里。
		showList();
		pendingOpen = last;
		fetchInfo(last.url).then(
			() => {
				if (pendingOpen === last) open(last);
			},
			() => {}
		);
	} else {
		showList();
	}
}

$('toggle-edit').addEventListener('click', () => {
	pendingOpen = null;
	editing = !editing;
	showList();
});
$('add').addEventListener('click', () => showForm(null));
$('back').addEventListener('click', showList);
$('url').addEventListener('change', () => void check());
$('server-form').addEventListener('submit', save);
$('remove').addEventListener('click', remove);
start();
