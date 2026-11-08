#!/usr/bin/env python3
"""Builds the browser runtime the installers ship beside Rosorah, so nothing is downloaded the
first time chat, a web panel or a browser source starts: PySide6 (Qt WebEngine, so Chromium) at
the version the app pins, trimmed by prune-browser-runtime.py.

  windows <dir>  the official embeddable Python with PySide6 beside it (runs on Windows or Linux)
  linux <dir>    PySide6 in <dir>/site-packages for the system Python (runs on Linux)

Rosorah finds it as browser-runtime next to its executable (crates/app/src/browser_chat.rs)."""
import hashlib, re, shutil, subprocess, sys, urllib.request, zipfile
from pathlib import Path

PYTHON = '3.13.16'
EMBED_SHA256 = '97dae5274cc54867065e8d5a3226e48c35017ed332a0fdb0e27d5b5821961297'
root = Path(__file__).resolve().parent.parent
target, out = sys.argv[1], Path(sys.argv[2]).resolve()
pyside = re.search(r'PYSIDE6_VERSION: &str = "(.*)"', (root / 'crates/app/src/browser_chat.rs').read_text()).group(1)
work = root / 'target/browser-runtime'
work.mkdir(parents=True, exist_ok=True)

def run(*args):
    subprocess.run([str(a) for a in args], check=True)

def pip_install(dest, *args):
    run(sys.executable, '-m', 'pip', 'install', '--disable-pip-version-check', '-q', '--upgrade', '--target', dest, *args)

if out.exists():
    shutil.rmtree(out)
if target == 'windows':
    archive = work / f'python-{PYTHON}-embed-amd64.zip'
    if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest() != EMBED_SHA256:
        urllib.request.urlretrieve(f'https://www.python.org/ftp/python/{PYTHON}/python-{PYTHON}-embed-amd64.zip', archive)
        if hashlib.sha256(archive.read_bytes()).hexdigest() != EMBED_SHA256:
            sys.exit('Embeddable Python download does not match its checksum')
    with zipfile.ZipFile(archive) as z:
        z.extractall(out)
    site = out / 'Lib/site-packages'
    pip_install(site, '--only-binary=:all:', '--platform', 'win_amd64', '--python-version', '3.13', '--implementation', 'cp', f'PySide6=={pyside}')
    # Where this Python looks for modules: its own, PySide6, and the helpers beside it in ../browser.
    (out / 'python313._pth').write_text('python313.zip\n.\nLib/site-packages\n../browser\n')
elif target == 'linux':
    site = out / 'site-packages'
    pip_install(site, '--only-binary=:all:', f'PySide6=={pyside}')
else:
    sys.exit(__doc__)
tools = work / 'tools'
pip_install(tools, 'pefile', 'pyelftools')
run(sys.executable, root / 'packaging/prune-browser-runtime.py', site, tools)
shutil.rmtree(site / 'bin', ignore_errors=True)
(out / 'rosorah-runtime.version').write_text(pyside + '\n')

if target == 'windows' and sys.platform == 'win32':
    python = out / 'python.exe'
    sign = subprocess.run(['powershell', '-NoProfile', '-Command', f"(Get-AuthenticodeSignature '{python}').SignerCertificate.Subject"], capture_output=True, text=True)
    if 'O=Python Software Foundation' not in sign.stdout:
        sys.exit('Embeddable Python signature check failed')
    # Compiled now, so Python writes nothing into the install folder that the uninstaller misses.
    run(python, '-m', 'compileall', '-q', '-j', '0', site, *([out.parent / 'browser'] if (out.parent / 'browser').is_dir() else []))
    run(python, '-c', 'from PySide6.QtWebEngineWidgets import QWebEngineView; from PySide6.QtWebEngineCore import qWebEngineChromiumVersion as v; '
        f'print("Browser runtime: PySide6 {pyside}, Chromium", v())')
else:
    print(f'Browser runtime: PySide6 {pyside} for {target}')
