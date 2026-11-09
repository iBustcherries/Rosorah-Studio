"""The Twitch login is read from a Firefox cookie database, picking the right profile."""
import sqlite3, tempfile, unittest
from pathlib import Path
import firefox_cookies

SCHEMA = ("CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY, originAttributes TEXT NOT NULL DEFAULT '', name TEXT, value TEXT, host TEXT, path TEXT,"
          " expiry INTEGER, lastAccessed INTEGER, creationTime INTEGER, isSecure INTEGER, isHttpOnly INTEGER, inBrowserElement INTEGER DEFAULT 0,"
          " sameSite INTEGER DEFAULT 0, rawSameSite INTEGER DEFAULT 0, schemeMap INTEGER DEFAULT 0)")
NOW = 1_800_000_000

def profile(root, name, rows):
    d = Path(root) / 'Profiles' / name
    d.mkdir(parents=True)
    db = sqlite3.connect(d / 'cookies.sqlite')
    db.execute(SCHEMA)
    for r in rows:
        db.execute('INSERT INTO moz_cookies (originAttributes, name, value, host, path, expiry, lastAccessed, isSecure, isHttpOnly, sameSite) VALUES (?,?,?,?,?,?,?,?,?,?)', r)
    db.commit(); db.close()

class FirefoxCookieTests(unittest.TestCase):
    def test_newest_twitch_login_wins_and_other_sites_stay_out(self):
        with tempfile.TemporaryDirectory() as root:
            profile(root, 'old.default', [('', 'auth-token', 'old', '.twitch.tv', '/', NOW + 99, 10, 1, 0, 0)])
            profile(root, 'new.default-release', [
                ('', 'auth-token', 'new', '.twitch.tv', '/', NOW + 99, 20, 1, 0, 0),
                ('', 'login', 'justmoe4', '.twitch.tv', '/', (NOW + 99) * 1000, 20, 1, 0, 1),
                ('', 'expired', 'x', '.twitch.tv', '/', NOW - 1, 20, 1, 0, 0),
                ('^userContextId=2', 'auth-token', 'container', '.twitch.tv', '/', NOW + 99, 99, 1, 0, 0),
                ('', 'session', 'secret', '.example.com', '/', NOW + 99, 99, 1, 0, 0),
            ])
            profile(root, 'none.default', [('', 'persistent', 'y', '.twitch.tv', '/', NOW + 99, 50, 1, 0, 0)])
            cookies = firefox_cookies.twitch_login([root], now=NOW)
            self.assertEqual(sorted((c['name'], c['value']) for c in cookies), [('auth-token', 'new'), ('login', 'justmoe4')])
            login = next(c for c in cookies if c['name'] == 'login')
            self.assertEqual(firefox_cookies.expiry_seconds(login), NOW + 99, 'milliseconds are understood')
    def test_no_login_anywhere(self):
        with tempfile.TemporaryDirectory() as root:
            profile(root, 'a.default', [('', 'persistent', 'y', '.twitch.tv', '/', NOW + 99, 50, 1, 0, 0)])
            self.assertIsNone(firefox_cookies.twitch_login([root], now=NOW))
            self.assertIsNone(firefox_cookies.twitch_login([Path(root) / 'missing'], now=NOW))

if __name__ == '__main__': unittest.main()
