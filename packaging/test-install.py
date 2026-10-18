#!/usr/bin/env python3
"""Isolated install/uninstall regression; never changes the current user's setup."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

base = Path(__file__).resolve().parent
source = base.parent / 'native/build'
with tempfile.TemporaryDirectory(prefix='rosorah-install-') as temporary:
    root = Path(temporary)
    data, config = root / 'data', root / 'config'
    command = ['python3', str(base / 'install-renderer.py'), '--data-home', str(data), '--config-home', str(config), '--source-dir', str(source)]
    foreign = data / 'vulkan/implicit_layer.d/other.json'
    foreign.parent.mkdir(parents=True)
    foreign.write_text('unrelated layer')
    subprocess.run(command, check=True, capture_output=True)
    session = config / 'plasma-workspace/env/rosorah-capture.sh'
    assert not session.exists()  # Default setup requires no KDE login change.
    for suffix, arch in [('', '64'), ('32', '32')]:
        content = json.loads((foreign.parent / f'rosorah-capture{suffix}.json').read_text())
        assert content['layer']['library_arch'] == arch
        assert Path(content['layer']['library_path']).read_bytes() == (source / arch / 'librosorah-vulkan.so').read_bytes()
    # Optional legacy session fallback remains available; migration removes only
    # this owned file when the no-restart default is installed again.
    subprocess.run(command + ['--session-hook'], check=True, capture_output=True)
    # Source twice: own entry once and preserve someone else's entry exactly.
    script = 'LD_PRELOAD=existing-library.so; . "$1"; . "$1"; printf "%s\\n" "$LD_PRELOAD" "$ROSORAH_GL_HOOK_VERSION"'
    output = subprocess.check_output(['sh', '-c', script, 'test', str(session)], text=True).splitlines()
    assert output[0].endswith(':existing-library.so') and output[0].count('librosorah-gl.so') == 1
    assert output[1] == '0.8.0' and '$LIB' in output[0]
    # Verify the current host's dynamic loader expands the installed literal token.
    preload = output[0].split(':')[0]
    loaded = subprocess.run(['/bin/true'], env={**os.environ, 'LD_PRELOAD': preload}, check=True, capture_output=True)
    assert not loaded.stderr, loaded.stderr.decode()
    subprocess.run(command, check=True, capture_output=True)
    assert not session.exists()
    subprocess.run(command + ['--uninstall'], check=True, capture_output=True)
    assert not session.exists() and not (foreign.parent / 'rosorah-capture.json').exists()
    assert not (foreign.parent / 'rosorah-capture32.json').exists() and foreign.read_text() == 'unrelated layer'
    assert Path(content['layer']['library_path']).exists()  # Mapped files survive uninstall.
    session.parent.mkdir(parents=True, exist_ok=True)
    session.write_text('custom session file\n')
    refused = subprocess.run(command, capture_output=True)
    assert refused.returncode != 0 and session.read_text() == 'custom session file\n'
print('PASS: both architectures, token expansion, repeat install, preserved preload, scoped uninstall, foreign-file protection')
