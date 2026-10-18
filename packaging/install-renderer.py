#!/usr/bin/env python3
"""Install per-user Vulkan/OpenGL hooks; session-wide preload is optional."""
import argparse
import json
import os
from pathlib import Path
import shlex
import tempfile

VERSION = '0.8.0'
MARKER = '# Managed by Rosorah Studio: automatic OpenGL capture'


def atomic_write(path, data, mode):
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_file() and path.read_bytes() == data:
        return
    fd, temporary = tempfile.mkstemp(prefix='.rosorah-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as file:
            file.write(data)
            os.fchmod(file.fileno(), mode)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--uninstall', action='store_true')
    parser.add_argument('--quiet', action='store_true')
    parser.add_argument('--session-hook', action='store_true', help='Optional legacy KDE login preload instead of relying only on launch observation')
    parser.add_argument('--source-dir', type=Path, default=Path(__file__).resolve().parent / 'hooks')
    parser.add_argument('--data-home', type=Path, default=Path(os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share'))))
    parser.add_argument('--config-home', type=Path, default=Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home() / '.config'))))
    args = parser.parse_args()
    root = args.data_home.expanduser().resolve()
    installed = root / 'rosorah-studio' / f'renderer-{VERSION}'
    session = args.config_home.expanduser().resolve() / 'plasma-workspace/env/rosorah-capture.sh'
    manifests = [(root / 'vulkan/implicit_layer.d/rosorah-capture.json', '64', 'VK_LAYER_ROSORAH_capture'),
                 (root / 'vulkan/implicit_layer.d/rosorah-capture32.json', '32', 'VK_LAYER_ROSORAH_capture32')]
    # Check ownership and all input bytes before changing any installed file.
    if session.exists() and MARKER not in session.read_text():
        raise SystemExit('The session file is not managed by Rosorah; leaving it unchanged.')
    for manifest, _, name in manifests:
        if manifest.exists() and json.loads(manifest.read_text()).get('layer', {}).get('name') != name:
            raise SystemExit(f'{manifest.name} does not belong to Rosorah; leaving it unchanged.')
    if args.uninstall:
        for manifest, _, _ in manifests:
            manifest.unlink(missing_ok=True)
        session.unlink(missing_ok=True)
        # Retain mapped library bytes for programs in the current session.
        print('Automatic hooks removed. Close Rosorah to stop the launch observer. Already-loaded hooks remain idle until their processes exit.')
        return
    if any(c.isspace() or c == ':' for c in str(installed)):
        raise SystemExit('The data directory contains whitespace or a colon, which LD_PRELOAD cannot represent safely.')
    files = {}
    for arch, elf_class in [('64', 2), ('32', 1)]:
        for api in ['vulkan', 'gl', 'gl-capture']:
            path = args.source_dir / arch / f'librosorah-{api}.so'
            if not path.is_file():
                raise SystemExit(f'{path.name} ({arch}-bit) is missing. Extract the complete Linux package.')
            data = path.read_bytes()
            if data[:5] != b'\x7fELF' + bytes([elf_class]):
                raise SystemExit(f'{path}: incorrect library architecture.')
            files[arch, api] = data
    # $LIB varies across Fedora, Debian and Steam runtimes. Each spelling resolves
    # to the matching ELF architecture; no wrong-architecture preload warnings.
    for arch, directories in [('64', ['lib64', 'lib/x86_64-linux-gnu']), ('32', ['lib', 'lib32', 'lib/i386-linux-gnu'])]:
        for directory in directories:
            for api in ['vulkan', 'gl', 'gl-capture']:
                atomic_write(installed / directory / f'librosorah-{api}.so', files[arch, api], 0o755)
    for manifest, arch, name in manifests:
        library = installed / ('lib64' if arch == '64' else 'lib') / 'librosorah-vulkan.so'
        content = {'file_format_version': '1.2.0', 'layer': {
            'name': name, 'type': 'GLOBAL', 'library_path': str(library), 'library_arch': arch,
            'api_version': '1.3.0', 'implementation_version': '2',
            'description': 'Rosorah automatic Vulkan capture (active only while Rosorah is open)',
            'disable_environment': {'ROSORAH_DISABLE_CAPTURE': '1'}}}
        atomic_write(manifest, (json.dumps(content, indent=2) + '\n').encode(), 0o644)
    hook = shlex.quote(str(installed / '$LIB/librosorah-gl.so'))
    script = f'''#!/bin/sh
{MARKER}
# Sourced once by Plasma at login. Preserve other preload entries.
rosorah_gl_hook={hook}
case ":${{LD_PRELOAD-}}:" in
    *":$rosorah_gl_hook:"*) ;;
    *) LD_PRELOAD="$rosorah_gl_hook${{LD_PRELOAD:+:$LD_PRELOAD}}" ;;
esac
export LD_PRELOAD
ROSORAH_GL_HOOK_VERSION={shlex.quote(VERSION)}
export ROSORAH_GL_HOOK_VERSION
unset rosorah_gl_hook
'''
    if args.session_hook:
        atomic_write(session, script.encode(), 0o644)
    elif session.exists():
        # Migration affects only our own login script. A running Steam/KDE
        # session keeps its current environment until it naturally exits.
        session.unlink()
    if not args.quiet:
        print('64-bit and 32-bit Vulkan/OpenGL hooks installed for this user.')
        print('Open Rosorah before launching a game. Steam and your desktop session can remain open.')
        print('If Fedora blocks launch observation, use the permission button in Capture details. This activation method is experimental.')
        if args.session_hook:
            print('Optional KDE session hook installed; it takes effect at the next login.')
        print('No per-game Steam launch option or special Steam launcher is needed. Windows/Flatpak are not supported by this installer.')


if __name__ == '__main__':
    main()
