# SPDX-License-Identifier: GPL-2.0-only
"""Real Caddy + Rust on temporary loopback ports. Does not request public certificates."""
import base64
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Thread

ROOT = Path(__file__).resolve().parents[2]
CADDY = os.environ.get('CADDY_BIN', 'caddy')
CLIENT = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]

def request(url, method='GET', body=None, headers=None):
    headers = dict(headers or {})
    if body is not None:
        body = json.dumps(body).encode()
        headers['Content-Type'] = 'application/json'
    try:
        response = CLIENT.open(urllib.request.Request(url, body, headers, method=method), timeout=5)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.read()

def ready(url):
    for _ in range(100):
        try:
            if request(url)[0] == 200:
                return
        except OSError:
            pass
        time.sleep(.1)
    raise AssertionError('temporary service did not become ready')

def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            raise AssertionError('temporary service did not shut down gracefully')

class Echo(BaseHTTPRequestHandler):
    def do_GET(self):
        value = json.dumps({'authorization': self.headers.get('Authorization')}).encode()
        self.send_response(200)
        self.end_headers()
        self.wfile.write(value)
    def log_message(self, *_):
        pass

with tempfile.TemporaryDirectory(prefix='autocomic-deployment-') as temporary:
    directory = Path(temporary)
    app_port, proxy_port = port(), port()
    base = f'http://127.0.0.1:{app_port}'
    proxy = f'http://127.0.0.1:{proxy_port}'
    password = 'test-only-deployment-password'
    hashed = subprocess.check_output([CADDY, 'hash-password', '--plaintext', password], text=True).strip()
    env = dict(os.environ, APP_HOST='0.0.0.0', APP_PORT=str(app_port),
               DATABASE_URL=f'sqlite://{directory}/state.db', POLL_INTERVAL_SECONDS='3600',
               MIKAN_BASE_URL='https://example.invalid', QBIT_URL='', WEBSITE_PROXY_URL='',
               SITE_ADDRESS=proxy, ADMIN_USER='admin', ADMIN_PASSWORD_HASH=hashed,
               UPSTREAM=f'127.0.0.1:{app_port}', XDG_DATA_HOME=str(directory/'caddy-data'),
               XDG_CONFIG_HOME=str(directory/'caddy-config'))
    auth = {'Authorization': 'Basic ' + base64.b64encode(f'admin:{password}'.encode()).decode()}
    processes = []
    echo = None
    with (directory/'services.log').open('w+') as logs:
        def start_app():
            process = subprocess.Popen([str(ROOT/'target/debug/autocomic-rss')], cwd=directory, env=env, stdout=logs, stderr=logs)
            processes.append(process)
            ready(base+'/ready')
            return process
        def start_proxy():
            process = subprocess.Popen([CADDY, 'run', '--config', str(ROOT/'deploy/Caddyfile'), '--adapter', 'caddyfile'], cwd=directory, env=env, stdout=logs, stderr=logs)
            processes.append(process)
            for _ in range(100):
                try:
                    if request(proxy+'/health')[0] == 401:
                        return process
                except OSError:
                    pass
                time.sleep(.1)
            raise AssertionError('proxy did not start')
        try:
            app = start_app()
            subprocess.run([CADDY, 'validate', '--config', str(ROOT/'deploy/Caddyfile')], env=env, stdout=logs, stderr=logs, check=True)
            gateway = start_proxy()
            for route in ['/', '/assets/app.js', '/api/status', '/ready', '/health']:
                assert request(proxy+route)[0] == 401
                assert request(proxy+route, headers=auth)[0] == 200
            assert request(proxy+'/api/run', 'POST')[0] == 401
            assert request(proxy+'/api/run', 'POST', headers={**auth, 'Origin': 'https://evil.invalid'})[0] == 403
            assert request(proxy+'/api/run', 'POST', headers={**auth, 'Sec-Fetch-Site': 'cross-site'})[0] == 403
            assert request(proxy+'/api/run', 'POST', headers={**auth, 'Origin': proxy})[0] == 200
            assert request(proxy+'/api/status', headers={'Authorization': 'Basic ' + base64.b64encode(b'admin:wrong').decode()})[0] == 401
            status, body = request(proxy+'/api/subscriptions', 'POST', {'title': 'deployment persistence', 'rss_url': 'https://example.invalid/rss', 'enabled': False}, auth)
            assert status == 201
            subscription = json.loads(body)
            stop(app)
            assert app.returncode == 0
            # Stop before copying SQLite's database/WAL files. Restore into an empty location.
            backup = directory/'backup'
            backup.mkdir()
            for file in directory.glob('state.db*'):
                shutil.copy2(file, backup/file.name)
            restore = directory/'restored'
            shutil.copytree(backup, restore)
            env['DATABASE_URL'] = f'sqlite://{restore}/state.db'
            app = start_app()
            restored = json.loads(request(proxy+'/api/subscriptions', headers=auth)[1])
            assert restored[0]['id'] == subscription['id']
            assert json.loads(request(proxy+'/api/jobs', headers=auth)[1]) == []
            # Verify the reverse proxy never forwards the browser's login to the upstream.
            stop(gateway)
            echo = ThreadingHTTPServer(('127.0.0.1', 0), Echo)
            Thread(target=echo.serve_forever, daemon=True).start()
            env['UPSTREAM'] = f'127.0.0.1:{echo.server_port}'
            gateway = start_proxy()
            assert json.loads(request(proxy+'/echo', headers=auth)[1])['authorization'] is None
            print('PASS: readiness, configurable bind, whole-site authentication, cross-origin rejection, credential stripping, graceful shutdown and SQLite restore.')
        except BaseException:
            logs.flush()
            logs.seek(0)
            print(logs.read()[-6000:])
            raise
        finally:
            for process in reversed(processes):
                stop(process)
            if echo:
                echo.shutdown()
                echo.server_close()
