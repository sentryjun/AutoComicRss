# SPDX-License-Identifier: GPL-2.0-only
"""Run against a locally built Linux image; creates and cleans only its own containers/volumes."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

IMAGE = sys.argv[1] if len(sys.argv) == 2 else 'autocomic-rss:smoke'
PREFIX = 'autocomic-smoke-' + uuid.uuid4().hex[:12]
CLIENT = urllib.request.build_opener(urllib.request.ProxyHandler({}))
containers, volumes = [], []

def docker(*arguments, **kwargs):
    return subprocess.run(['docker', *arguments], check=True, capture_output=True, **kwargs).stdout

def api(base, route, body=None):
    headers = {}
    if body is not None:
        body = json.dumps(body).encode()
        headers['Content-Type'] = 'application/json'
    with CLIENT.open(urllib.request.Request(base + route, body, headers), timeout=5) as response:
        return json.load(response)

def volume(suffix):
    name = PREFIX + '-' + suffix
    docker('volume', 'create', name)
    volumes.append(name)
    return name

def start(name, data_volume, media):
    containers.append(name)
    docker('run', '-d', '--name', name, '--read-only', '--cap-drop', 'ALL',
           '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw,noexec,nosuid,size=64m,mode=1777',
           '--mount', f'type=volume,src={data_volume},dst=/app/data',
           '--mount', f'type=bind,src={media},dst=/media,readonly',
           '-p', '127.0.0.1::8080', '-e', 'POLL_INTERVAL_SECONDS=3600', IMAGE)
    inspect = json.loads(docker('inspect', name))[0]
    base = 'http://127.0.0.1:' + inspect['NetworkSettings']['Ports']['8080/tcp'][0]['HostPort']
    for _ in range(120):
        inspect = json.loads(docker('inspect', name))[0]
        if inspect['State'].get('Health', {}).get('Status') == 'healthy':
            assert api(base, '/ready')['status'] == 'ready'
            break
        if not inspect['State']['Running']:
            raise AssertionError(docker('logs', name).decode())
        time.sleep(1)
    else:
        raise AssertionError('container health check did not pass')
    assert docker('exec', name, 'id', '-u').strip() == b'10001'
    assert inspect['HostConfig']['ReadonlyRootfs']
    with CLIENT.open(base, timeout=5) as response:
        assert b'AutoComicRss' in response.read()
    return base

def stop(name):
    docker('stop', '-t', '120', name)
    assert json.loads(docker('inspect', name))[0]['State']['ExitCode'] == 0

try:
    docker('info')  # require a working daemon before creating anything
    with tempfile.TemporaryDirectory(prefix=PREFIX) as temporary:
        media = Path(temporary)/'media'
        media.mkdir(mode=0o755)
        (media/'example.mkv').write_bytes(b'test')
        (media/'example.mkv').chmod(0o644)
        data = volume('data')
        restored = volume('restored')
        first = PREFIX + '-first'
        base = start(first, data, media)
        sub = api(base, '/api/subscriptions', {'title': 'container persistence', 'rss_url': 'https://example.invalid/rss', 'enabled': False})
        destination = api(base, '/api/destinations', {'name': 'read-only media', 'kind': 'local', 'root_path': '/media'})
        listing = api(base, '/api/destinations/' + destination['id'] + '/entries')
        assert listing['entries'][0]['name'] == 'example.mkv'
        attempt = subprocess.run(['docker', 'exec', first, 'touch', '/media/should-not-exist'], capture_output=True)
        assert attempt.returncode != 0 and not (media/'should-not-exist').exists()
        stop(first)
        second = PREFIX + '-second'
        base = start(second, data, media)
        assert api(base, '/api/subscriptions')[0]['id'] == sub['id']
        assert api(base, '/api/jobs') == []
        stop(second)
        archive = docker('run', '--rm', '--mount', f'type=volume,src={data},dst=/app/data,readonly', '--entrypoint', 'tar', IMAGE, '-C', '/app/data', '-czf', '-', '.')
        docker('run', '--rm', '-i', '--mount', f'type=volume,src={restored},dst=/app/data', '--entrypoint', 'tar', IMAGE, '-C', '/app/data', '-xzf', '-', input=archive)
        third = PREFIX + '-third'
        base = start(third, restored, media)
        assert api(base, '/api/subscriptions')[0]['id'] == sub['id']
        assert api(base, '/api/destinations')[0]['id'] == destination['id']
        stop(third)
        print('PASS: image startup/health, non-root, read-only media, persistence across recreation, graceful shutdown and SQLite volume restore.')
finally:
    for name in containers:
        subprocess.run(['docker', 'rm', '-f', name], capture_output=True)
    for name in volumes:
        subprocess.run(['docker', 'volume', 'rm', name], capture_output=True)
