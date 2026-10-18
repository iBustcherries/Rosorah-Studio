"""Exercise actual Qt PNG, animated GIF and animated WebP decoders offline."""
import base64,unittest
from PySide6.QtCore import QCoreApplication
from emotes import Images,safe_url
FIXTURES={'png': 'iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAYAAABe3VzdAAAAPElEQVR4nO3OMQGAMBAAsQf/nh8JDDe0Q6Igz87sXOw9HfgjWAlWgpVgJVgJVoKVYCVYCVaClWAlWAlWH9x0AjrjEu4GAAAAAElFTkSuQmCC', 'gif': 'R0lGODlhKAAeAIEAAP8AAAAAAAAAAAAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQACAAAACwAAAAAKAAeAAAIOgABCBxIsKDBgwgTKlzIsKHDhxAjSpxIsaLFixgzatzIsaPHjyBDihxJsqTJkyhTqlzJsqXLlzAxBgQAIfkEAQ4AAQAsAAAAACgAHgCBAAD/AAAAAAAAAAAACDoAAQgcSLCgwYMIEypcyLChw4cQI0qcSLGixYsYM2rcyLGjx48gQ4ocSbKkyZMoU6pcybKly5cwMQYEADs=', 'webp': 'UklGRu4AAABXRUJQVlA4WAoAAAACAAAAJwAAHQAAQU5JTQYAAAAAAAAAAABBTk1GYgAAAAAAAAAAACcAAB0AAFAAAAJWUDggSgAAANADAJ0BKigAHgA+bTaXSKQjIiElaACADYlnAND6gH4AArRcgxcMAAD+8JtD/+QXLC65Gv/8gP+QH/ID/+QH/5a0L0/y0YcUAAAAQU5NRlgAAAAAAAAAAAAnAAAdAACMAAAAVlA4IEAAAAD0AgCdASooAB4APm0ukUiCVgAA2JZwB2AATE+hbBkiAAD++R3b//5Af/5Af/5Af/yA//4hd7Ic//IZaf8hgAAA'}
class EmoteTests(unittest.TestCase):
    def test_real_decoders(self):
        app=QCoreApplication.instance() or QCoreApplication([])
        for extension,data in FIXTURES.items():
            frames=[];images=Images(None,frames.append)
            images.decode('https://static-cdn.jtvnw.net/test',base64.b64decode(data))
            self.assertEqual(len(frames),1 if extension=='png' else 2)
            if extension!='png':self.assertEqual([f['delay'] for f in frames],[80,140])
            self.assertTrue(all(len(f['png'])<65536 for f in frames))
    def test_rejects_remote_untrusted_and_local_urls(self):
        for url in ['file:///etc/passwd','https://static-cdn.jtvnw.net.evil.test/x','http://www.gstatic.com/x','https://user@www.gstatic.com/x','https://www.gstatic.com:999/x']:
            self.assertFalse(safe_url(url))
if __name__=='__main__':unittest.main()
