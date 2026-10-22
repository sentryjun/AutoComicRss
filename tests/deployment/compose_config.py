# SPDX-License-Identifier: GPL-2.0-only
"""Validate real Compose merging without a daemon. COMPOSE_BIN may be a standalone binary."""
import itertools
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
COMPOSE = shlex.split(os.environ.get('COMPOSE_BIN', 'docker compose'))
HASH = '$2a$14$Zkx19XLiW6VYouLHR5NmfOFU0z2GTNmpkT/5qqR7hx4IjWJPDhjvG'
with tempfile.TemporaryDirectory(prefix='autocomic-compose-') as directory:
    settings = {
        'HOST_PORT': '18080', 'DATABASE_URL': 'sqlite:///app/data/ani.db',
        'MEDIA_PATH': directory, 'MEDIA_GID': '1234',
        'MYSQL_PASSWORD': 'a' * 48, 'MYSQL_ROOT_PASSWORD': 'b' * 48,
        'SITE_ADDRESS': 'anime.example.com', 'ADMIN_USER': 'admin',
        'ADMIN_PASSWORD_HASH': HASH,
        'QBIT_URL': '', 'QBIT_USERNAME': '', 'QBIT_PASSWORD': '',
        'WEBSITE_PROXY_URL': '', 'WEBSITE_DIRECT_HOSTS': '',
    }
    env = {key: value for key, value in os.environ.items() if key not in settings and key not in ('COMPOSE_FILE', 'COMPOSE_PROJECT_NAME')}
    env_file = Path(directory) / '.env'
    env_file.write_text('\n'.join(f"{key}='{value}'" for key, value in settings.items()))
    def config(files, check=True):
        command = COMPOSE + ['--env-file', str(env_file), '-f', 'compose.yaml']
        for name in files:
            command += ['-f', f'deploy/compose.{name}.yaml']
        return subprocess.run(command + ['config', '--format', 'json'], cwd=ROOT, env=env, capture_output=True, text=True, check=check)
    for flags in itertools.product([False, True], repeat=3):
        overlays = [name for name, enabled in zip(['media', 'mysql', 'https'], flags) if enabled]
        model = json.loads(config(overlays).stdout)
        app = model['services']['app']
        assert app['read_only'] and app['environment']['APP_HOST'] == '0.0.0.0'
        assert app['environment']['APP_PORT'] == '8080'
        assert any(v['target'] == '/app/data' and v['type'] == 'volume' for v in app['volumes'])
        if 'media' in overlays:
            mount = next(v for v in app['volumes'] if v['target'] == '/media')
            assert mount['read_only'] and not mount['bind'].get('create_host_path', False)
        if 'mysql' in overlays:
            assert app['environment']['DATABASE_URL'] == f"mysql://autocomic:{settings['MYSQL_PASSWORD']}@mysql:3306/autocomic"
            assert not model['services']['mysql'].get('ports')
            assert app['depends_on']['mysql']['condition'] == 'service_healthy'
        if 'https' in overlays:
            assert not app.get('ports'), 'HTTPS mode must not leave an unauthenticated host port'
            proxy = model['services']['caddy']
            # `compose config` re-escapes dollar signs so its output can be loaded again.
            assert proxy['environment']['ADMIN_PASSWORD_HASH'] == HASH.replace('$', '$$'), 'bcrypt dollar signs must survive interpolation'
            assert {p['target'] for p in proxy['ports']} == {80, 443}
            assert proxy['volumes'][0]['source'] == str(ROOT / 'deploy/Caddyfile')
        else:
            assert app['ports'][0]['host_ip'] == '127.0.0.1'
    # Incomplete protected deployments must fail closed during config generation.
    settings.pop('ADMIN_PASSWORD_HASH')
    env_file.write_text('\n'.join(f"{key}='{value}'" for key, value in settings.items()))
    assert config(['https'], check=False).returncode != 0
print('PASS: all 8 Compose combinations, private ports, mounts, MySQL readiness and bcrypt interpolation; missing auth rejected.')
