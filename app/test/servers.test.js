import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fetchInfo, normalizeUrl, openTarget, rememberPage, upsert } from '../ui/servers.js';

const server = (fields) => ({ id: 'a', name: '本子', url: 'http://nas:3002', token: null, lastUrl: null, ...fields });

test('normalizeUrl keeps only the origin and defaults to http', () => {
	assert.equal(normalizeUrl(' 192.168.1.10:3002/ '), 'http://192.168.1.10:3002');
	assert.equal(normalizeUrl('https://media.example.ts.net/a?b'), 'https://media.example.ts.net');
	assert.throws(() => normalizeUrl(''), /请填/);
	assert.throws(() => normalizeUrl('ftp://nas'), /http/);
	assert.throws(() => normalizeUrl('http://'), /格式/);
});

test('rememberPage records the page on the matching server only', () => {
	const servers = [server(), server({ id: 'b', url: 'http://nas:3003' })];
	const { servers: next, current } = rememberPage(servers, 'http://nas:3003/?q=x');
	assert.equal(current, 'b');
	assert.equal(next[1].lastUrl, 'http://nas:3003/?q=x');
	assert.equal(next[0], servers[0]);
	assert.equal(servers[1].lastUrl, null);
	assert.deepEqual(rememberPage(servers, 'http://other/'), { servers, current: null });
	assert.deepEqual(rememberPage(servers, 'not a url'), { servers, current: null });
});

test('openTarget resumes the last page and hands the token over in the fragment', () => {
	assert.equal(openTarget(server()), 'http://nas:3002/');
	assert.equal(openTarget(server({ lastUrl: 'http://nas:3002/a/5/read?p=3' })), 'http://nas:3002/a/5/read?p=3');
	// 别的源的地址不当作上次的页面。
	assert.equal(openTarget(server({ lastUrl: 'http://evil/x' })), 'http://nas:3002/');
	const url = new URL(openTarget(server({ token: 'shuku_t', lastUrl: 'http://nas:3002/?q=a&p=2' })));
	assert.equal(url.pathname, '/login');
	assert.equal(url.searchParams.get('next'), '/?q=a&p=2');
	assert.equal(new URLSearchParams(url.hash.slice(1)).get('token'), 'shuku_t');
});

test('upsert replaces by id or appends', () => {
	const list = [server()];
	assert.deepEqual(upsert(list, server({ name: '新' })), [server({ name: '新' })]);
	assert.equal(upsert(list, server({ id: 'b' })).length, 2);
	assert.equal(list[0].name, '本子');
});

test('fetchInfo explains failures', async () => {
	const ok = { name: '本子', version: '0.1.0', archives: 3, require_token: true };
	const reply = (status, body) => async () => new Response(JSON.stringify(body), { status });
	assert.deepEqual(await fetchInfo('http://nas', reply(200, ok)), ok);
	await assert.rejects(fetchInfo('http://nas', reply(404, {})), /404/);
	await assert.rejects(fetchInfo('http://nas', reply(200, { hello: 1 })), /不是 Shuku/);
	await assert.rejects(
		fetchInfo('http://nas', async () => {
			throw new TypeError('network');
		}),
		/连不上/
	);
});

test('fetchInfo retries the same server after Android grants LAN access', async () => {
	const info = { name: '书库', version: '0.1.0', archives: 3, require_token: false };
	const calls = [];
	let granted = false;
	const fetcher = async (url) => {
		calls.push(url);
		if (!granted) throw new TypeError('Failed to fetch');
		return Response.json(info);
	};
	const grant = async () => { calls.push('permission'); granted = true; return true; };
	assert.deepEqual(await fetchInfo('http://192.168.0.231:3002', fetcher, grant), info);
	assert.deepEqual(calls, ['http://192.168.0.231:3002/api/info', 'permission', 'http://192.168.0.231:3002/api/info']);
});

test('fetchInfo does not prompt for a working public server or an HTTP error', async () => {
	const unexpected = () => assert.fail('不应申请局域网权限');
	const ok = { name: '公网书库', version: '0.1.0', archives: 1, require_token: true };
	assert.deepEqual(await fetchInfo('https://example.org', async () => Response.json(ok), unexpected), ok);
	await assert.rejects(fetchInfo('https://example.org', async () => new Response(null, {status: 403}), unexpected), /403/);
});

test('fetchInfo preserves a permission denial and does not retry an offline server indefinitely', async () => {
	let requests = 0;
	const offline = async () => { requests++; throw new TypeError('Failed to fetch'); };
	await assert.rejects(fetchInfo('http://nas', offline, async () => { throw new Error('未获局域网权限'); }), /未获局域网权限/);
	assert.equal(requests, 1);
	await assert.rejects(fetchInfo('http://nas', offline, async () => false), /连不上/);
	assert.equal(requests, 2);
	await assert.rejects(fetchInfo('http://nas', offline, async () => true), /连不上/);
	assert.equal(requests, 4);
});
