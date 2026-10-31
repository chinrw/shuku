// 控制台的接口，格式见仓库的 docs/api.md 的“控制台”一节。
import { ApiError } from './api';

export interface ScanReport {
	added: number;
	updated: number;
	moved: number;
	unchanged: number;
	skipped: number;
	failed: number;
}

export interface HashReport {
	hashed: number;
	skipped: number;
	failed: number;
}

interface Task {
	running: boolean;
	/** Unix 毫秒。 */
	started_at: number | null;
	finished_at: number | null;
	total: number;
	error: string | null;
}

export interface ScanTask extends Task {
	processed: number;
	report: ScanReport;
	/** `broken` 是有坏页的本数，存在库里，不随扫描清空。 */
	issues: { failed: number; skipped: number; unreadable: number; missing: number; broken: number };
}

export interface HashTask extends Task {
	report: HashReport;
}

export interface Status {
	server: {
		version: string;
		started_at: number;
		content_dir: string;
		data_dir: string;
		web_dir: string;
		listen: string;
		detect_webtoons: boolean;
		allowed_hosts: string[];
	};
	library: {
		archives: number;
		pages: number;
		bytes: number;
		hashed: number;
		lrr_imported: number;
		lrr_pending: number;
		with_progress: number;
		read_last_week: number;
		categories: number;
		tankoubons: number;
		tags: number;
		namespaces: number;
	};
	storage: { database_bytes: number; wal_bytes: number; cache_bytes: number; cache_files: number };
	snapshot: { built_at: number; archives: number; read_ms: number; build_ms: number };
	scan: ScanTask;
	hash: HashTask;
}

interface Listed<T> {
	total: number;
	items: T[];
}

export interface Issues {
	scanning: boolean;
	finished_at: number | null;
	failed: Listed<{ path: string; error: string }>;
	skipped: Listed<{ path: string }>;
	unreadable: Listed<{ path: string; error: string }>;
	missing: Listed<{ id: number; path: string; title: string | null }>;
	broken: Listed<{ id: number; path: string; title: string; pages: string[] }>;
}

export interface DuplicateFile {
	id: number;
	path: string;
	title: string;
	file_size: number;
}

export interface NamespaceCount {
	namespace: string;
	tags: number;
	uses: number;
}

export type LogLevel = 'info' | 'warn' | 'error';

export interface LogEntry {
	seq: number;
	time: number;
	level: LogLevel;
	message: string;
}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
	const response = await fetch(path, init);
	if (!response.ok) {
		const body = await response.json().catch(() => null);
		throw new ApiError(response.status, body?.error ?? response.statusText);
	}
	return response.json();
}

/** 在控制台里改、存在服务端数据库里的设置。 */
export interface ServerSettings {
	scan_hidden: boolean;
	/** 0 表示不定时扫描。 */
	scan_interval_hours: number;
}

export const status = () => call<Status>('/api/admin/status');
export const serverSettings = () => call<ServerSettings>('/api/admin/settings/scan');
export const updateServerSettings = (patch: Partial<ServerSettings>) =>
	call<ServerSettings>('/api/admin/settings/scan', {
		method: 'PUT',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(patch)
	});
export const issues = () => call<Issues>('/api/admin/issues');
export const duplicates = (limit = 100) =>
	call<{ total: number; groups: { files: DuplicateFile[] }[] }>(
		`/api/admin/duplicates?limit=${limit}`
	);
export const namespaces = async () =>
	(await call<{ namespaces: NamespaceCount[] }>('/api/admin/namespaces')).namespaces;
/** `started_at` 标识服务端进程，序号每次启动从 1 开始。 */
export const logs = (level: LogLevel, after: number) =>
	call<{ entries: LogEntry[]; started_at: number }>(
		`/api/admin/logs?${new URLSearchParams({ level, after: String(after), limit: '1000' })}`
	);
export const startScan = () => call<{ started: boolean }>('/api/admin/scan', { method: 'POST' });
export const startHash = () => call<{ started: boolean }>('/api/admin/hash', { method: 'POST' });
export const refreshSnapshot = () => call<unknown>('/api/admin/refresh', { method: 'POST' });
export const clearCache = () =>
	call<{ removed_files: number; removed_bytes: number }>('/api/admin/cache', { method: 'DELETE' });
/** 删除文件已经不在的记录，一次最多 1000 条。文件还在的和已经没有的分别列出，不删。 */
export const deleteMissing = (ids: number[]) =>
	call<{ deleted: number[]; present: number[]; not_found: number[] }>(
		'/api/admin/missing/delete',
		{
			method: 'POST',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify({ ids })
		}
	);
/** 标签改名，目标已经存在时合并。返回原来用到这个标签的本数。 */
export const renameTag = (from: string, to: string) =>
	call<{ archives: number }>('/api/admin/tags/rename', {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ from, to })
	});
