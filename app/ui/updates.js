// 渠道偏好只在自带页面的源里保存，远端书库没有更新权限。
const CHANNEL_KEY = 'shuku.updateChannel';

export async function setupUpdates(stayOnList) {
	const invoke = window.__TAURI__?.core.invoke;
	if (!invoke) return;
	const section = document.getElementById('updates');
	const channel = document.getElementById('channel');
	const checkButton = document.getElementById('check-update');
	const install = document.getElementById('install-update');
	const status = document.getElementById('update-status');
	const stored = localStorage.getItem(CHANNEL_KEY);
	const preference = ['stable', 'nightly'].includes(stored) ? stored : null;
	// 与自动打开上次服务器并行。即使列表马上导航走，原生检查也已经启动。
	const details = invoke('client_info');
	const startup = invoke('check_client_update', { channel: preference });
	// 手动检查才显示连接失败；这里立即接住拒绝，不让导航时产生未处理异常。
	const initial = startup.catch(() => null);
	const info = await details;
	channel.value = preference ?? info.channel;
	document.getElementById('installed-version').textContent = info.version;
	section.hidden = false;
	let request = 0;
	let offered = null;

	function message(text) {
		status.textContent = text;
		status.hidden = !text;
	}

	async function check(manual, pending) {
		const token = ++request;
		offered = null;
		install.hidden = true;
		checkButton.disabled = true;
		message(manual ? '正在检查…' : '');
		try {
			const result = await (pending ?? invoke('check_client_update', { channel: channel.value }));
			if (token !== request || !result) return;
			if (result.status === 'available') {
				offered = result.version;
				message('可更新到 ' + offered);
				install.hidden = false;
			} else if (manual) {
				message(result.status === 'unavailable' ? '此渠道尚无适用的客户端更新包。' : '没有比当前构建更新的版本。');
			}
		} catch (error) {
			if (token === request && manual) message('检查失败：' + error);
		} finally {
			if (token === request) checkButton.disabled = false;
		}
	}

	section.addEventListener('pointerdown', stayOnList);
	section.addEventListener('focusin', stayOnList);
	if (info.managed) {
		checkButton.disabled = true;
		message('此安装方式不在应用内更新，请通过包管理器或原安装方式升级。');
	}
	channel.addEventListener('change', () => {
		localStorage.setItem(CHANNEL_KEY, channel.value);
		if (!info.managed) void check(true);
	});
	checkButton.addEventListener('click', () => check(true));
	install.addEventListener('click', async () => {
		if (!offered || install.disabled) return;
		channel.disabled = checkButton.disabled = install.disabled = true;
		try {
			message('正在准备更新…');
			await invoke('install_client_update', { channel: channel.value, version: offered });
			message('已打开 APK 下载页，请按系统提示确认安装。');
		} catch (error) {
			message('没有安装更新：' + error);
		} finally {
			install.hidden = true;
			offered = null;
			channel.disabled = checkButton.disabled = install.disabled = false;
		}
	});
	if (!info.managed) await check(false, initial);
}
