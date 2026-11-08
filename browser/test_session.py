#!/usr/bin/env python3
"""Runs the browser helpers inside a real desktop session and checks what the user gets: canvas
frames, transparency, input on web panels, and which windows open and take focus. Run it with the
browser runtime's Python inside a session; scripts/browser-session-test.sh starts one (sway with no
screen for Wayland, Xvfb for X11). Window checks need sway, so they run on Wayland only."""
import http.server, json, os, shutil, struct, subprocess, sys, tempfile, threading, time, unittest
from pathlib import Path
BASE=Path(__file__).resolve().parent
WAYLAND=bool(os.environ.get('WAYLAND_DISPLAY'))
SWAY=WAYLAND and bool(os.environ.get('SWAYSOCK')) and shutil.which('swaymsg') is not None
SHOTS=Path(os.environ.get('ROSORAH_SESSION_ARTIFACTS',tempfile.gettempdir()))
APP_ID='io.rosorah.Studio'
# Stands in for Rosorah (crates/activation): a window that gets focus and a key press, then prints
# the activation token it asks for with that key's serial.
TOKEN_TOOL=os.environ.get('ROSORAH_ACTIVATION_TOKEN_TOOL','')

# A transparent overlay with a red square and a bar that changes every frame, and a web panel
# whose left half turns blue when clicked and whose title follows what is typed in its box.
PAGES={
'/overlay.html':'''<!doctype html><html><body style="margin:0;background:transparent">
<div style="position:absolute;left:20px;top:20px;width:40px;height:40px;background:#f00"></div>
<div id="bar" style="position:absolute;left:0;top:100px;height:10px;background:#0f0"></div>
<script>let n=0;(function tick(){document.getElementById('bar').style.width=(n++%200)+'px';requestAnimationFrame(tick)})()</script>
</body></html>''',
'/panel.html':'''<!doctype html><html><head><title>panel ready</title></head><body style="margin:0;background:#fff">
<button id="b" style="position:absolute;left:0;top:0;width:200px;height:300px;border:0;background:transparent"
 onclick="document.body.style.background='#00f'">blue</button>
<input id="i" style="position:absolute;left:220px;top:130px;width:160px;height:40px" oninput="document.title='typed:'+this.value">
</body></html>''',
}
class Pages(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body=PAGES.get(self.path,'').encode()
        self.send_response(200 if body else 404);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    def log_message(self,*args):pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Pages)
threading.Thread(target=server.serve_forever,daemon=True).start()
def url(path):return f'http://127.0.0.1:{server.server_port}{path}'

def helper_env(scaled=True):
    # The same environment Rosorah gives its helpers.
    env={k:v for k,v in os.environ.items() if k not in ('LD_LIBRARY_PATH','QT_PLUGIN_PATH','QT_QPA_PLATFORM_PLUGIN_PATH')}
    if scaled:env.update(QT_SCALE_FACTOR='1',QT_AUTO_SCREEN_SCALE_FACTOR='0')
    if os.environ.get('ROSORAH_SESSION_DEBUG'):env['QT_LOGGING_RULES']='qt.qpa.wayland*=true'
    return env

class Helper:
    """A helper process; binary packets (sources, panels) or JSON lines (chat) from its stdout."""
    def __init__(self,script,config=None,args=(),magic=None,env=None):
        self.log=tempfile.NamedTemporaryFile('w+',prefix=script+'-',suffix='.log',delete=False)
        self.proc=subprocess.Popen([sys.executable,str(BASE/script),*args],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.log,env=env or helper_env())
        self.cv=threading.Condition();self.frame=None;self.frames=0;self.distinct=0;self.notes=[];self.titles=[];self.lines=[]
        if config is not None:self.send(config)
        threading.Thread(target=self.read_packets if magic else self.read_lines,args=(magic,) if magic else (),daemon=True).start()
    def send(self,message):
        self.proc.stdin.write((json.dumps(message)+'\n').encode());self.proc.stdin.flush()
    def read_packets(self,magic):
        out=self.proc.stdout
        while (header:=out.read(20)) and len(header)==20 and header[:4]==magic:
            kind,w,h,n=struct.unpack('<IIII',header[4:]);data=out.read(n)
            with self.cv:
                if kind==1:
                    self.frames+=1
                    if self.frame is None or data!=self.frame[2]:self.distinct+=1
                    self.frame=(w,h,data)
                elif kind==2:self.titles.append(data.decode())
                elif kind==3:self.notes.append(data.decode())
                self.cv.notify_all()
    def read_lines(self):
        for line in self.proc.stdout:
            try:item=json.loads(line)
            except ValueError:item={'type':'not json','text':line.decode(errors='replace')[:500]}
            with self.cv:self.lines.append(item);self.cv.notify_all()
    def click(self,x,y):
        # As Rosorah's panel sends them: the pointer arrives, then a press and a release a moment apart.
        for kind in ('move','press','release'):self.send({'type':kind,'x':x,'y':y,'button':'left'});time.sleep(0.05)
    def wait(self,test,timeout=30):
        with self.cv:return self.cv.wait_for(lambda:test(self),timeout)
    def pixel(self,x,y):
        w,h,data=self.frame;i=(y*w+x)*4;return tuple(data[i:i+4])
    def stderr(self):
        return Path(self.log.name).read_text(errors='replace')[-4000:]
    def close(self):
        if self.proc.poll() is None:
            try:self.send({'type':'quit'})
            except (BrokenPipeError,OSError):pass
            try:self.proc.wait(5)
            except subprocess.TimeoutExpired:self.proc.kill();self.proc.wait()
        for f in (self.proc.stdin,self.proc.stdout,self.log):f.close()

def windows(pid=None):
    """Windows the compositor shows: title, app id, focus, and whether they are on screen."""
    tree=json.loads(subprocess.run(['swaymsg','-t','get_tree','-r'],capture_output=True,text=True,check=True).stdout);found=[]
    def walk(node):
        if node.get('pid') and node.get('type') in ('con','floating_con') and (pid is None or node['pid']==pid):
            found.append({'title':node.get('name'),'app_id':node.get('app_id'),'focused':node.get('focused'),'visible':node.get('visible'),'urgent':node.get('urgent'),'pid':node['pid']})
        for child in node.get('nodes',[])+node.get('floating_nodes',[]):walk(child)
    walk(tree);return found
def wait_for(test,timeout=15):
    end=time.monotonic()+timeout
    while time.monotonic()<end:
        if (result:=test()):return result
        time.sleep(0.2)
    return test()
def opening(message):
    """A command that opens a window, with the activation token Rosorah attaches on Wayland."""
    if WAYLAND and TOKEN_TOOL:
        tool=subprocess.Popen([TOKEN_TOOL],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        try:
            assert tool.stdout.readline().strip()=='ready','the token window never became active: '+tool.stderr.read()
            # A real key press in that window, from a virtual keyboard; a pause lets it join the seat.
            subprocess.run(['wtype','-s','300','-k','space','-s','300','-k','space'],check=True,timeout=10)
            out,err=tool.communicate(timeout=10)
        finally:
            if tool.poll() is None:tool.kill();tool.wait()
        assert tool.returncode==0 and out.strip(),'no activation token: '+err
        message=dict(message,activation=out.strip())
    return message
def screenshot(name):
    if WAYLAND and shutil.which('grim'):subprocess.run(['grim',str(SHOTS/(name+'.png'))],capture_output=True)

class SessionTest(unittest.TestCase):
    def start(self,*args,**kw):
        helper=Helper(*args,**kw);self.addCleanup(helper.close)
        self.addCleanup(lambda:self._outcome.success or print('\n--- '+args[0]+' stderr ---\n'+helper.stderr(),file=sys.stderr))
        return helper
    def blocker(self):
        # Another app's window, which takes focus the way a game or OBS dock would.
        proc=subprocess.Popen([sys.executable,'-c','from PySide6.QtWidgets import QApplication,QWidget\na=QApplication([]);a.setDesktopFileName("blocker");w=QWidget();w.setWindowTitle("blocker");w.show();a.exec()'],env=helper_env(False))
        self.addCleanup(lambda:(proc.kill(),proc.wait()))
        self.assertTrue(wait_for(lambda:any(w['focused'] for w in windows(proc.pid))),'the other window never took focus')
        return proc

class SourceTests(SessionTest):
    def source(self):
        return self.start('source_browser.py',{'url':url('/overlay.html'),'width':320,'height':240,'fps':30,'audio':False,'css':'','storage':tempfile.mkdtemp()},magic=b'RSBW')
    def test_frames_are_transparent_and_keep_updating(self):
        s=self.source()
        self.assertTrue(s.wait(lambda s:s.frame and s.pixel(30,30)[0]>200),f'no overlay frame; notes {s.notes}')
        self.assertEqual(s.frame[:2],(320,240))
        r,g,b,a=s.pixel(30,30);self.assertTrue(r>200 and g<60 and b<60 and a>200,f'square is {(r,g,b,a)}')
        self.assertEqual(s.pixel(300,220)[3],0,'the page background is not transparent')
        before=s.distinct;time.sleep(3)
        self.assertGreater(s.distinct-before,20,'the animation stopped updating while the source has no window')
        if SWAY:self.assertEqual(windows(s.proc.pid),[],'a browser source put a window on screen')
    @unittest.skipUnless(SWAY,'needs sway')
    def test_interact_window_opens_in_front(self):
        s=self.source();self.assertTrue(s.wait(lambda s:s.frame),f'no frame; notes {s.notes}')
        self.blocker();s.send(opening({'type':'interact'}))
        shown=wait_for(lambda:[w for w in windows(s.proc.pid) if w['visible']]);screenshot('source-interact')
        self.assertTrue(shown,'Interact opened no window')
        self.assertEqual(shown[0]['app_id'],APP_ID)
        self.assertTrue(wait_for(lambda:any(w['focused'] for w in windows(s.proc.pid)),5),'the Interact window opened without focus')
        before=s.frames;time.sleep(1);self.assertGreater(s.frames,before,'frames stopped while interacting')

class PanelTests(SessionTest):
    def test_frames_clicks_and_typing(self):
        p=self.start('dock_browser.py',{'url':url('/panel.html'),'width':400,'height':300,'zoom':1.0,'storage':tempfile.mkdtemp()},magic=b'RSDK')
        self.assertTrue(p.wait(lambda p:'panel ready' in p.titles and p.frame and p.pixel(100,150)[:3]==(255,255,255)),f'no panel frame; notes {p.notes}')
        self.assertEqual(p.frame[:2],(400,300))
        p.click(100,150)
        self.assertTrue(p.wait(lambda p:p.pixel(100,150)[2]>200 and p.pixel(100,150)[0]<60,10),f'click did not reach the page: {p.pixel(100,150)}')
        p.click(300,150)
        for ch in 'abc':p.send({'type':'text','text':ch})
        self.assertTrue(p.wait(lambda p:'typed:abc' in p.titles,10),f'typing did not reach the page; titles {p.titles}')
        if SWAY:self.assertEqual(windows(p.proc.pid),[],'a web panel put a window on screen')

class ChatTests(SessionTest):
    def chat(self,*args):
        env=helper_env(False);env['XDG_DATA_HOME']=tempfile.mkdtemp()
        c=self.start('chat_browser.py',args=args,env=env)
        self.assertTrue(c.wait(lambda c:any(l.get('type')=='status' for l in c.lines)),f'chat never started; exit {c.proc.poll()}; stdout {c.lines}; stderr {c.stderr()}')
        return c
    @unittest.skipUnless(SWAY,'needs sway')
    def test_background_start_shows_nothing_until_asked(self):
        c=self.chat('--background');time.sleep(3)
        self.assertEqual(windows(c.proc.pid),[],'background chat put a window on screen')
        c.send(opening({'type':'show'}))
        shown=wait_for(lambda:windows(c.proc.pid));screenshot('chat-show')
        self.assertTrue(shown,'show opened no window');self.assertEqual(shown[0]['app_id'],APP_ID)
        self.assertTrue(wait_for(lambda:any(w['focused'] for w in windows(c.proc.pid)),5),'chat opened without focus')
    @unittest.skipUnless(SWAY,'needs sway')
    def test_show_brings_an_open_chat_window_forward(self):
        c=self.chat();self.assertTrue(wait_for(lambda:windows(c.proc.pid)),'chat window never opened')
        self.blocker();c.send(opening({'type':'show'}))
        self.assertTrue(wait_for(lambda:any(w['focused'] for w in windows(c.proc.pid)),5),f'show left the chat window behind another app: {windows()}')
    @unittest.skipUnless(SWAY,'needs sway')
    def test_hidden_platform_page_opens_in_front(self):
        # Kick's page stays laid out off screen; opening it from Rosorah must bring it up focused.
        c=self.chat('--background');c.send({'type':'sources','kick':'rosorahtest'});time.sleep(2)
        self.assertEqual(windows(c.proc.pid),[],'the hidden Kick page put a window on screen')
        self.blocker();c.send(opening({'type':'open_platform','platform':'Kick'}))
        shown=wait_for(lambda:[w for w in windows(c.proc.pid) if w['title']=='Rosorah · Kick chat']);screenshot('chat-kick')
        self.assertTrue(shown,'the Kick page did not open');self.assertEqual(shown[0]['app_id'],APP_ID)
        self.assertTrue(wait_for(lambda:any(w['focused'] for w in windows(c.proc.pid)),5),'the Kick page opened without focus')

if __name__=='__main__':
    print('session:', 'Wayland' if WAYLAND else 'X11' if os.environ.get('DISPLAY') else sys.platform, '· window checks', 'on' if SWAY else 'off', '· activation tokens', 'on' if WAYLAND and TOKEN_TOOL else 'off', file=sys.stderr)
    unittest.main()
