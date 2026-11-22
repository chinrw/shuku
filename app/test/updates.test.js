import assert from 'node:assert/strict';
import test from 'node:test';
import { setupUpdates } from '../ui/updates.js';

// 使用与自带页面相同的按钮和事件顺序，不需要启动 WebView 或下载真正的安装包。
function page(invoke, stored) {
	const elements = new Map();
	class Element {
		hidden = true;
		disabled = false;
		listeners = {};
		addEventListener(event, cb) { this.listeners[event] = cb; }
	}
	globalThis.document = {getElementById(id) {
		if (!elements.has(id)) elements.set(id, new Element());
		return elements.get(id);
	}};
	globalThis.window = {__TAURI__: {core: {invoke}}};
	globalThis.localStorage = {getItem: () => stored, setItem: (_, v) => { stored = v; }};
	return (id) => document.getElementById(id);
}

test('后台只检查；只有显式确认才安装选中的渠道和版本', async () => {
	const calls = [];
	const el = page(async (cmd, args) => {
		calls.push([cmd,args]);
		if (cmd === 'client_info') return { channel:'nightly', version:'0.1.1+1001', managed:false };
		if (cmd === 'check_client_update') return { status:'available', version:'0.1.1+1002' };
	});
	await setupUpdates(() => {});
	assert.equal(el('channel').value,'nightly');
	assert.deepEqual(calls.map(([cmd]) => cmd),['client_info','check_client_update']);
	assert.equal(el('install-update').hidden,false);
	await el('install-update').listeners.click();
	assert.deepEqual(calls.at(-1),['install_client_update',{channel:'nightly',version:'0.1.1+1002'}]);
});

test('选择较旧的稳定渠道或缺少清单时不提供安装', async () => {
	for (const result of ['current','unavailable']) {
		const el = page(async (cmd) => {
			if (cmd === 'client_info') return { channel:'nightly', version:'0.1.1+1002', managed:false };
			if (cmd === 'check_client_update') return {status:result};
			assert.fail(cmd);
		},'stable');
		await setupUpdates(() => {});
		assert.equal(el('channel').value,'stable');
		assert.equal(el('install-update').hidden,true);
	}
});

test('包管理器安装既不向更新服务发请求，也不提供下载', async () => {
	const el = page(async (cmd) => {
		if (cmd === 'check_client_update') return {status:'managed'};
		assert.equal(cmd,'client_info');
		return {channel:'stable',version:'0.1.1+1001',managed:true};
	});
	await setupUpdates(() => {});
	assert.equal(el('check-update').disabled,true);
	assert.equal(el('install-update').hidden,true);
});

test('换渠道后晚到的 Nightly 结果不能覆盖 Stable', async () => {
	let finishNightly;
	const el = page(async (cmd, args) => {
		if (cmd === 'client_info') return {channel:'nightly',version:'0.1.1+1001',managed:false};
		if (cmd === 'check_client_update' && args.channel === 'stable') return {status:'current'};
		if (cmd === 'check_client_update') return new Promise((resolve) => { finishNightly = resolve; });
		assert.fail(cmd);
	});
	const started = setupUpdates(() => {});
	await new Promise((resolve) => setImmediate(resolve));
	el('channel').value = 'stable';
	el('channel').listeners.change();
	await new Promise((resolve) => setImmediate(resolve));
	finishNightly({status:'available',version:'0.1.1+1002'});
	await started;
	assert.equal(el('install-update').hidden,true);
	assert.match(el('update-status').textContent,/没有比当前构建更新/);
});
