"""用小型安装包夹具验证两渠道清单；验签、安装另用原生 Updater 测试。"""
import base64
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release


class ManifestTest(unittest.TestCase):
    def test_server_release_tag_is_not_a_client_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            assets = Path(tmp)
            version = '0.1.1+1001'
            for name in [release.APK, release.WINDOWS, release.MAC, 'shuku-macos-universal.dmg']:
                (assets / name).write_bytes(b'installer fixture')
            sig = base64.b64encode(f'untrusted comment: test\nx\ntrusted comment: timestamp:1\tversion:{version}\nx'.encode()).decode()
            for name in [release.WINDOWS, release.MAC]:
                (assets / (name + '.sig')).write_text(sig)
            for tag, channel in [('nightly', 'nightly'), ('v3.0.0', 'stable'), ('v4.0.0', 'stable')]:
                data = release.manifest(version, tag, assets)
                self.assertEqual((data['version'], data['build'], data['channel']), (version, 1001, channel))
                self.assertTrue(data['android'].endswith('/' + tag + '/' + release.APK))
                self.assertEqual(data['platforms']['darwin-x86_64'], data['platforms']['darwin-aarch64'])
                self.assertNotIn('linux-x86_64', data['platforms'])
            with self.assertRaises(ValueError):
                release.manifest('0.1.1+1002', 'nightly', assets)
            with self.assertRaises(ValueError):
                release.manifest(version, 'unstable', assets)


if __name__ == '__main__':
    unittest.main()
