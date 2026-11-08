#!/usr/bin/env python3
"""Trims the PySide6 the installers ship to what Rosorah's browser helpers use: Qt WebEngine
(Chromium) with widgets, networking and web channels. It keeps every Qt library those modules,
QtWebEngineProcess and the kept plugins actually load (read from their import tables), and removes
the rest of Qt (3D, Multimedia, Designer, QML tools...), developer files and debug data.

Usage: prune-browser-runtime.py <site-packages> <dir with pefile and pyelftools>"""
import os, shutil, sys
from pathlib import Path

site = Path(sys.argv[1])
sys.path.insert(0, sys.argv[2])
pyside = site / 'PySide6'
windows = (pyside / 'QtCore.pyd').exists()
# On Linux the Qt libraries, plugins and data live under PySide6/Qt.
qt = pyside if windows else pyside / 'Qt'
libdir = pyside if windows else qt / 'lib'

# What the helpers import (browser/*.py), and the plugin families Qt loads by itself.
MODULES = ['QtCore', 'QtGui', 'QtWidgets', 'QtNetwork', 'QtWebChannel', 'QtWebEngineCore', 'QtWebEngineWidgets']
PLUGINS = ['platforms', 'imageformats', 'styles', 'tls', 'networkinformation', 'iconengines', 'generic',
           'platforminputcontexts', 'platformthemes', 'xcbglintegrations', 'egldeviceintegrations',
           'wayland-decoration-client', 'wayland-graphics-integration-client', 'wayland-shell-integration']

def binding(module):
    return pyside / (f'{module}.pyd' if windows else f'{module}.abi3.so')

def is_qt_library(path):
    # Qt's own module libraries and FFmpeg, which only Qt Multimedia uses.
    n = path.name
    return (n.startswith('Qt6') and n.endswith('.dll')) or n.startswith('libQt6') or \
        n.split('-')[0] in ('avcodec', 'avformat', 'avutil', 'swresample', 'swscale') or \
        n.split('.so')[0] in ('libavcodec', 'libavformat', 'libavutil', 'libswresample', 'libswscale')

def imports(path):
    """The libraries a binary names in its import table (file names)."""
    try:
        if windows:
            import pefile
            pe = pefile.PE(str(path), fast_load=True)
            pe.parse_data_directories(directories=[pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_IMPORT'],
                                                   pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT']])
            entries = getattr(pe, 'DIRECTORY_ENTRY_IMPORT', []) + getattr(pe, 'DIRECTORY_ENTRY_DELAY_IMPORT', [])
            return {e.dll.decode().lower() for e in entries}
        from elftools.elf.elffile import ELFFile
        from elftools.elf.dynamic import DynamicSection
        with open(path, 'rb') as f:
            elf = ELFFile(f)
            return {t.needed for s in elf.iter_sections() if isinstance(s, DynamicSection)
                    for t in s.iter_tags() if t.entry.d_tag == 'DT_NEEDED'}
    except Exception:
        return set()

libraries = {(p.name.lower() if windows else p.name): p for p in libdir.iterdir() if p.is_file()}
process = qt / ('QtWebEngineProcess.exe' if windows else 'libexec/QtWebEngineProcess')
roots = [binding(m) for m in MODULES] + [process]
roots += [p for d in PLUGINS if (qt / 'plugins' / d).is_dir() for p in (qt / 'plugins' / d).iterdir() if p.is_file()]
missing = [r for r in roots[:len(MODULES) + 1] if not r.exists()]
if missing:
    sys.exit(f'PySide6 layout not recognised; missing {missing}')

def module_of(lib):
    # Qt6Gui.dll and libQt6Gui.so.6 belong to the QtGui module.
    name = lib.name.removeprefix('lib').split('.')[0]
    return 'Qt' + name[3:] if name.startswith('Qt6') else None

# A kept Qt library keeps its Python module too, since modules import each other's types.
needed, modules, todo = set(), set(MODULES), list(roots)
while todo:
    for name in imports(todo.pop()):
        lib = libraries.get(name)
        if lib and lib not in needed:
            needed.add(lib)
            todo.append(lib)
            module = module_of(lib)
            if module and module not in modules and binding(module).exists():
                modules.add(module)
                todo.append(binding(module))

removed = 0
def remove(path):
    global removed
    if not path.exists():
        return
    # Some QML folders nest past Windows' 260-character path limit.
    long = Path('\\\\?\\' + str(path.resolve())) if os.name == 'nt' else path
    removed += sum(f.stat().st_size for f in long.rglob('*') if f.is_file()) if long.is_dir() else long.stat().st_size
    shutil.rmtree(long) if long.is_dir() else long.unlink()

for lib in libraries.values():
    if is_qt_library(lib) and lib not in needed:
        remove(lib)
for p in list(pyside.iterdir()):
    stem = p.name.split('.')[0]
    # Python bindings of modules that aren't used, with their type stubs.
    if stem.startswith('Qt') and stem not in modules and p.suffix in ('.pyd', '.so', '.pyi'):
        remove(p)
    # Qt's developer tools; QtWebEngineProcess is the browser itself.
    elif p.suffix == '.exe' and p != process:
        remove(p)
for name in ('metatypes', 'include', 'typesystems', 'glue', 'doc', 'scripts', 'qml', 'examples'):
    remove(pyside / name)
    remove(qt / name)
remove(qt / 'bin')
for d in (qt / 'plugins').iterdir():
    if d.name not in PLUGINS:
        remove(d)
# Qt's interface translations; Chromium's own (qtwebengine_locales) stay.
for f in (qt / 'translations').glob('*.qm'):
    remove(f)
for f in (qt / 'resources').glob('*.debug.*'):
    remove(f)
for p in site.glob('*/__pycache__'):
    remove(p)
print(f'Trimmed {removed / 2**20:.0f} MiB; kept {len(needed)} libraries and modules {", ".join(sorted(modules))}')
