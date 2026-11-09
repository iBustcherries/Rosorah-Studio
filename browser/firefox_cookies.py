"""Reads your Twitch login from Firefox, for "Import Twitch login from Firefox". Twitch refuses to
log in from Rosorah's browser on Windows (error 5025, "Your browser is not currently supported"),
so the login you already have in Firefox is copied over instead. Only twitch.tv cookies are read,
and only when you press the button."""
import glob, os, shutil, sqlite3, sys, tempfile, time
from pathlib import Path

def firefox_roots():
    """Folders that hold Firefox profiles on this computer."""
    home = Path.home()
    if sys.platform == 'win32':
        appdata = Path(os.environ.get('APPDATA', home / 'AppData/Roaming'))
        local = Path(os.environ.get('LOCALAPPDATA', home / 'AppData/Local'))
        # The Microsoft Store Firefox keeps its profiles inside its package folder.
        store = [Path(p) / 'LocalCache/Roaming/Mozilla/Firefox' for p in glob.glob(str(local / 'Packages/Mozilla.Firefox*'))]
        return [appdata / 'Mozilla/Firefox', *store]
    xdg = Path(os.environ.get('XDG_CONFIG_HOME', home / '.config'))
    return [home / '.mozilla/firefox', xdg / 'mozilla/firefox', home / 'snap/firefox/common/.mozilla/firefox',
            home / '.var/app/org.mozilla.firefox/.mozilla/firefox', home / 'Library/Application Support/Firefox']

def cookie_files(roots=None):
    found = []
    for root in roots if roots is not None else firefox_roots():
        for pattern in ('*/cookies.sqlite', 'Profiles/*/cookies.sqlite'):
            found += [Path(p) for p in glob.glob(str(Path(root) / pattern))]
    return found

def read(path):
    """Twitch's cookies in one profile. Firefox locks its database while it runs, so a copy is read."""
    with tempfile.TemporaryDirectory() as tmp:
        copy = Path(tmp) / 'cookies.sqlite'
        shutil.copyfile(path, copy)
        if Path(str(path) + '-wal').is_file():
            shutil.copyfile(str(path) + '-wal', str(copy) + '-wal')
        db = sqlite3.connect(copy)
        try:
            db.row_factory = sqlite3.Row
            rows = db.execute("SELECT name, value, host, path, expiry, isSecure, isHttpOnly, sameSite, lastAccessed, originAttributes "
                              "FROM moz_cookies WHERE host = 'twitch.tv' OR host LIKE '%.twitch.tv'").fetchall()
            return [dict(r) for r in rows]
        finally:
            db.close()

def expiry_seconds(cookie):
    # Newer Firefox versions store milliseconds.
    e = cookie['expiry'] or 0
    return e / 1000 if e > 100_000_000_000 else e

def twitch_login(roots=None, now=None):
    """The cookies of the most recently used Twitch login in any Firefox profile (outside
    containers), or None when Firefox has no Twitch login."""
    now = now or time.time()
    best = None
    for path in cookie_files(roots):
        try:
            cookies = [c for c in read(path) if not c['originAttributes'] and expiry_seconds(c) > now]
        except (OSError, sqlite3.Error):
            continue
        token = next((c for c in cookies if c['name'] == 'auth-token' and c['value']), None)
        if token and (best is None or token['lastAccessed'] > best[0]):
            best = (token['lastAccessed'], cookies)
    return best[1] if best else None
