"""生成客户端清单；服务器的版本号、提交号和 Actions 运行次数都不是客户端版本。"""
import argparse
import base64
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
CONFIG = 'app/src-tauri/tauri.conf.json'
APK = 'shuku-android-arm64.apk'
WINDOWS = 'shuku-windows-x86_64.setup.exe'
MAC = 'shuku-macos-universal.app.tar.gz'
LINUX = 'shuku-linux-x86_64.AppImage'


def identity(version):
    match = re.fullmatch(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)\+([1-9]\d*)', version)
    if not match or int(match[4]) > 2_100_000_000:
        raise ValueError('expected a client version in MAJOR.MINOR.PATCH+BUILD format')
    return tuple(map(int, match.groups()))


def check_version(version, base):
    major, minor, patch, build = identity(version)
    if not base or not base.strip('0'):
        return
    diff = subprocess.check_output(['git', 'diff', '--name-only', base, 'HEAD', '--', 'app/ui', 'app/src-tauri'], cwd=ROOT)
    if not diff:
        return
    old = json.loads(subprocess.check_output(['git', 'show', f'{base}:{CONFIG}'], cwd=ROOT))
    if 'version' in old:
        a, b, c, previous = identity(old['version'])
        if build <= previous or (major, minor, patch) < (a, b, c):
            raise ValueError('client changes require a higher build number and a non-decreasing client version')


def manifest(version, tag, assets):
    build = identity(version)[3]
    channel = 'nightly' if tag == 'nightly' else 'stable'
    if channel == 'stable' and not re.fullmatch(r'v\d+\.\d+\.\d+', tag):
        raise ValueError('Stable requires a vMAJOR.MINOR.PATCH tag')
    base = f'https://github.com/chinrw/shuku/releases/download/{tag}/'
    def signed(name):
        if not (assets / name).is_file():
            raise ValueError(f'missing updater payload: {name}')
        sig = (assets / (name + '.sig')).read_text().strip()
        comment = base64.b64decode(sig, validate=True).decode().splitlines()[2]
        if not comment.startswith('trusted comment: ') or f'version:{version}' not in comment.split('\t'):
            raise ValueError(f'{name} has no signature bound to client version {version}')
        return {'url': base + name, 'signature': sig}

    for name in [APK, 'shuku-macos-universal.dmg']:
        if not (assets / name).is_file():
            raise ValueError(f'missing installer: {name}')
    mac = signed(MAC)
    platforms = {'windows-x86_64': signed(WINDOWS), 'darwin-x86_64': mac, 'darwin-aarch64': mac}
    if (assets / LINUX).is_file():
        platforms['linux-x86_64'] = signed(LINUX)
    return {'channel': channel, 'tag': tag, 'version': version, 'build': build,
            'android': base + APK, 'platforms': platforms}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--base')
    parser.add_argument('--tag')
    parser.add_argument('--assets', type=Path)
    args = parser.parse_args()
    version = json.loads((ROOT / CONFIG).read_text())['version']
    check_version(version, args.base)
    if args.tag:
        result = manifest(version, args.tag, args.assets)
        (args.assets / 'client-update.json').write_text(json.dumps(result, indent=2) + '\n')
